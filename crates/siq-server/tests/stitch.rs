//! Stitching from the service (ADR-0064), over HTTP on the simulated instrument with the
//! simulator's mount as the calibration in force: a 2 × 2 tile scan stitched by itself once it
//! finishes, its history recording the stitch's residuals, its mosaic served as deep-zoom tiles,
//! and its QA overlays drawn; stitched again on `jobs.stitch` into another image; and a dataset
//! without a calibration refused. REQ-STI-002, REQ-WF-001, REQ-VIS-001.

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::{config, finished_within, invoke};
use serde_json::{Value, json};
use siq_acquire::AcquisitionContext;
use siq_calib::{CalibrationRecord, Fit};
use siq_config::model::{CameraConfig, WorldConfig};
use siq_server::{Instrument, Service};
use siq_sim::camera::SimCameraSetup;
use siq_storage::{Dataset, DatasetKind, NewDataset};
use siq_types::{Affine2, ObjectiveId};
use teta_wot::http::axum::Router;
use teta_wot::http::axum::body::{Body, to_bytes};
use teta_wot::http::axum::http::{Request, StatusCode};
use teta_wot::testing::TestClient;
use tower::ServiceExt;

/// The simulated instrument over grains, with data and settings of its own, and the simulator's
/// mount as the calibration in force: a client of its Things, and its routes, the data routes
/// among them.
async fn rig() -> (TestClient, Arc<Instrument>, Router, [tempfile::TempDir; 2]) {
    let folders = [tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap()];
    let mut config = config();
    config.server.settings = folders[0].path().to_owned();
    config.server.data_root = folders[1].path().to_owned();
    config.server.min_free_gb = 0.0;
    // Textured all over, so that every overlap has something to register.
    config.sim.world = WorldConfig::Grains;
    let CameraConfig::Sim(camera) = &config.camera else {
        panic!("a simulated camera")
    };
    let setup = SimCameraSetup::from_config(camera, &config.objectives, &config.sim);
    let affine = setup
        .mount
        .compose(&Affine2::scale(10.0 / 3.76, 10.0 / 3.76));
    let service = Service::build(config).await.unwrap();
    let routes = service.router();
    let (server, instrument) = service.into_parts();
    let client = TestClient::start(server).await.unwrap();
    let fit = Fit {
        affine,
        residuals: Vec::new(),
    };
    let record = CalibrationRecord::new(ObjectiveId(1), &fit, None, "the simulator's mount");
    instrument.calibrations.save(&record, &[]).unwrap();
    let acquisition = &instrument.acquisition;
    acquisition.set_context(AcquisitionContext {
        objective: Some(ObjectiveId(1)),
        calibration: Some(record.id),
        ..acquisition.context()
    });
    (client, instrument, routes, folders)
}

/// GETs `path` from the service's routes: its status, and its body.
async fn get(routes: &Router, path: &str) -> (StatusCode, Vec<u8>) {
    let request = Request::builder().uri(path).body(Body::empty()).unwrap();
    let response = routes.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (status, body.to_vec())
}

async fn json_of(routes: &Router, path: &str) -> Value {
    let (status, body) = get(routes, path).await;
    let text = String::from_utf8_lossy(&body);
    assert_eq!(status, StatusCode::OK, "{path}: {text}");
    serde_json::from_slice(&body).unwrap()
}

/// Waits for dataset `id`'s history to record `count` of `kind` that ended, for up to 60 s: the
/// last.
async fn ended(routes: &Router, id: &str, kind: &str, count: usize) -> Value {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    loop {
        let manifest = json_of(routes, &format!("/api/siq/data/{id}/manifest")).await;
        let history = manifest["history"].as_array().cloned().unwrap_or_default();
        let done: Vec<&Value> = history
            .iter()
            .filter(|record| {
                record["kind"] == kind
                    && record["state"] != "running"
                    && record["state"] != "queued"
            })
            .collect();
        if done.len() >= count {
            return done[count - 1].clone();
        }
        assert!(tokio::time::Instant::now() < deadline, "{manifest}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test]
async fn a_finished_tile_scan_is_stitched_by_itself_and_its_qa_drawn() {
    let (client, _instrument, routes, _folders) = rig().await;
    assert_eq!(client.get("/api/v1/jobs/auto_stitch").await.json(), true);
    // 2 × 2 tiles at a quarter's overlap: 64 pixels a side to register, and the corners, 6 % of a
    // tile.
    let field = [256.0 * 0.376 / 1.002, 192.0 * 0.376 / 1.002];
    let step = [0.75 * field[0], 0.75 * field[1]];
    let (x, y) = (-20_000.0, -15_000.0);
    let region = json!({
        "shape": "rect",
        "x0": x - 0.8 * step[0],
        "y0": y - 0.8 * step[1],
        "x1": x + 0.8 * step[0],
        "y1": y + 0.8 * step[1],
    });
    let scan = json!({ "name": "scan", "region": region, "overlap": 0.25, "channels": [1] });
    let running = client
        .post_json("/api/v1/workflow/tile_scan", Some(&scan))
        .await;
    let done = finished_within(&client, running, Duration::from_secs(60)).await;
    assert_eq!(done["status"], "completed", "{done}");
    let id = done["output"]["dataset"].as_str().unwrap().to_owned();

    // The stitch, recorded with its measures.
    let record = ended(&routes, &id, "stitch", 1).await;
    assert_eq!(record["state"], "done", "{record}");
    let outputs = json!(["zarr/stitched.ome.zarr", "qa/stitched.json"]);
    assert_eq!(record["outputs"], outputs);
    let results = &record["results"];
    let counts = (results["tiles"].as_u64(), results["pairs"].as_u64());
    assert_eq!(counts, (Some(4), Some(6)), "{results}");
    assert!(results["trusted"].as_u64().unwrap() >= 4, "{results}");
    assert_eq!(results["components"], 1, "{results}");
    assert!(results["rms_px"].as_f64().unwrap() < 0.5, "{results}");

    // Its mosaic, as the viewer reads it.
    let info = json_of(&routes, &format!("/api/siq/data/{id}/stitched/info")).await;
    let width = info["size"]["x"].as_u64().unwrap();
    assert!((400..460).contains(&width), "{info}");
    let tile = format!("/api/siq/data/{id}/stitched/0/0/0_files/0/0_0.jpeg");
    assert_eq!(get(&routes, &tile).await.0, StatusCode::OK);
    // Its overlays: the four tiles, and the six pairs, coloured by their residuals.
    let layers = json_of(&routes, &format!("/api/siq/data/{id}/stitched/overlays")).await;
    assert_eq!(layers[0]["id"], "stitch-tiles");
    assert_eq!(layers[0]["shapes"].as_array().unwrap().len(), 4);
    let lines = layers[1]["shapes"].as_array().unwrap();
    assert_eq!(lines.len(), 6);
    assert!(
        lines
            .iter()
            .all(|line| line["kind"] == "line" && line["tone"].is_string())
    );
    assert_eq!(layers[1]["size"], json!([width, info["size"]["y"]]));

    // Again on request, into another image.
    let asked = json!({ "dataset": id, "image": "again" });
    let job = invoke(&client, "/api/v1/jobs/stitch", asked).await;
    assert!(job.as_u64().is_some(), "{job}");
    let record = ended(&routes, &id, "stitch", 2).await;
    let outputs = json!(["zarr/again.ome.zarr", "qa/again.json"]);
    assert_eq!(record["outputs"], outputs);
    let missing = format!("/api/siq/data/{id}/nothing/overlays");
    assert_eq!(get(&routes, &missing).await.0, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_dataset_without_a_calibration_is_not_stitched() {
    let (client, instrument, _routes, _folders) = rig().await;
    let new = NewDataset::new(DatasetKind::TileScan, "bare");
    let dataset = Dataset::create(instrument.catalog.data_root(), new).unwrap();
    instrument.catalog.record(dataset.path()).unwrap();
    let id = dataset.manifest().id.to_string();
    let asked = client
        .post_json("/api/v1/jobs/stitch", Some(&json!({ "dataset": id })))
        .await;
    let done = finished_within(&client, asked, Duration::from_secs(10)).await;
    assert_eq!(done["status"], "error", "{done}");
    let detail = done["error"]["detail"].as_str().unwrap();
    assert!(detail.contains("calibration"), "{detail}");
    let written = client
        .put_json("/api/v1/jobs/auto_stitch", &json!(false))
        .await;
    assert!(written.status.is_success());
    assert_eq!(client.get("/api/v1/jobs/auto_stitch").await.json(), false);
}
