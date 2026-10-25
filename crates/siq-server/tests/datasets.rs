//! The `datasets` Thing and processing on snap (ADR-0041, as amended in P4.7), with the simulated
//! instrument of `config/sim.toml`, end to end over HTTP: a snap in two channels made into a
//! dataset, its image written and its pyramid built, then its tiles, manifest and thumbnail
//! served; the summary, the exports, and deleting it. REQ-DATA-001, REQ-ACQ-006, REQ-NET-003.

mod common;

use std::time::Duration;

use common::{config, invoke};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use siq_jobs::{Job, JobCtx, JobError, JobId, JobOutput, JobState, Priority, Resources};
use siq_server::{Instrument, Service};
use siq_storage::zarr::ZarrImage;
use siq_storage::{Dataset, DatasetReader};
use siq_types::RectPx;
use teta_wot::http::axum::Router;
use teta_wot::http::axum::body::{Body, Bytes};
use teta_wot::http::axum::http::{Request, StatusCode};
use teta_wot::testing::{TestClient, TestResponse};
use tower::ServiceExt;

async fn get(router: &Router, path: &str) -> (StatusCode, Bytes) {
    let request = Request::builder().uri(path).body(Body::empty()).unwrap();
    let response = router.clone().oneshot(request).await.unwrap();
    let status = response.status();
    (
        status,
        response.into_body().collect().await.unwrap().to_bytes(),
    )
}

/// Waits for job `id` to end.
async fn wait(instrument: &Instrument, id: u64) -> JobState {
    let jobs = instrument.jobs.clone();
    let ended = tokio::task::spawn_blocking(move || jobs.wait(JobId(id), Duration::from_secs(60)));
    ended.await.unwrap().expect("the job ended").state
}

/// A JPEG's width and height, from its baseline frame header.
fn jpeg_size(jpeg: &[u8]) -> (u16, u16) {
    assert_eq!(&jpeg[..2], [0xFF, 0xD8], "a JPEG");
    let sof = jpeg.windows(2).position(|w| w == [0xFF, 0xC0]).unwrap();
    let size = |at: usize| u16::from_be_bytes([jpeg[at], jpeg[at + 1]]);
    (size(sof + 7), size(sof + 5))
}

/// A job that runs until cancelled.
struct Waits;

impl Job for Waits {
    fn kind(&self) -> &str {
        "waits"
    }

    fn estimate(&self) -> Resources {
        Resources::default()
    }

    fn run(&mut self, ctx: &JobCtx) -> Result<JobOutput, JobError> {
        loop {
            ctx.check()?;
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

#[tokio::test]
async fn a_snap_becomes_a_dataset_whose_tiles_are_served() {
    let root = tempfile::tempdir().unwrap();
    let mut config = config();
    config.server.data_root = root.path().to_owned();
    config.server.min_free_gb = 0.0;
    let siq_config::model::CameraConfig::Sim(camera) = &mut config.camera else {
        panic!("a simulated camera")
    };
    // Big enough for a pyramid of three levels: 640, 320, then 160 across.
    (camera.width, camera.height) = (640, 480);
    let service = Service::build(config).await.unwrap();
    let router = service.router();
    let (server, instrument) = service.into_parts();
    let client = TestClient::start(server).await.unwrap();
    let mut created = client.events("/api/v1/datasets/created").await;

    // A snap in DAPI, then in GFP: a channel each.
    let snap = invoke(
        &client,
        "/api/v1/datasets/snap_to_dataset",
        json!({ "channels": [2, 3], "name": "kidney" }),
    )
    .await;
    let entry = &snap["dataset"];
    let id = entry["id"].as_str().unwrap().to_owned();
    assert_eq!(
        (
            &entry["kind"],
            &entry["name"],
            &entry["frames"],
            &entry["images"]
        ),
        (
            &json!("snap"),
            &json!("kidney"),
            &json!(2),
            &json!(["snap"])
        )
    );
    assert_eq!(snap["image"], "snap");
    let event: Value = created.next().await.expect("the dataset's creation");
    assert_eq!(event["id"], id);

    // The pyramid is built as a job.
    let pyramid = snap["pyramid"].as_u64().unwrap();
    assert_eq!(wait(&instrument, pyramid).await, JobState::Done);
    let summary = instrument.catalog.get(id.parse().unwrap()).unwrap();
    let image = ZarrImage::open(&summary.path.join("zarr/snap.ome.zarr")).unwrap();
    let info = image.info();
    assert_eq!(
        (info.size.c, info.size.y, info.size.x, info.levels),
        (2, 480, 640, 3)
    );
    let labels: Vec<_> = info
        .channels
        .iter()
        .map(|c| (c.label.as_str(), c.colour))
        .collect();
    assert_eq!(
        labels,
        [("DAPI", [0x3f, 0x6f, 0xff]), ("GFP", [0x3f, 0xff, 0x6f])]
    );
    let (min, max) = info.channels[0].window;
    assert!(min < max, "the frame's own window: {min}..{max}");

    // Level 0 holds the raw frames' pixels, in the order snapped.
    let dataset = Dataset::open(&summary.path).unwrap();
    let raw = DatasetReader::open(&dataset).unwrap();
    assert_eq!(raw.frames().len(), 2);
    for (c, frame) in raw.frames().iter().enumerate() {
        let mapped = frame.map().unwrap();
        let expected: Vec<u16> = mapped
            .image_u16()
            .unwrap()
            .rows()
            .flatten()
            .copied()
            .collect();
        let written = image
            .read_region::<u16>(0, (0, c as u64, 0), RectPx::new(0, 0, 640, 480))
            .unwrap();
        assert!(written.pixels == expected, "channel {c} is its frame");
    }
    drop(raw);

    // Its tiles, over HTTP.
    let base = format!("/api/siq/data/{id}/snap");
    let (status, xml) = get(&router, &format!("{base}/1/0/0.dzi")).await;
    assert_eq!(status, StatusCode::OK);
    assert!(String::from_utf8_lossy(&xml).contains(r#"<Size Width="640" Height="480"/>"#));
    let (status, tile) = get(&router, &format!("{base}/1/0/0_files/10/0_0.jpeg")).await;
    assert_eq!((status, jpeg_size(&tile)), (StatusCode::OK, (255, 255)));
    let (status, tile) = get(&router, &format!("{base}/0/0/0_files/8/0_0.jpeg")).await;
    assert_eq!((status, jpeg_size(&tile)), (StatusCode::OK, (160, 120)));

    // Its manifest: the instrument, and both steps recorded.
    let (status, manifest) = get(&router, &format!("/api/siq/data/{id}/manifest")).await;
    assert_eq!(status, StatusCode::OK);
    let manifest: Value = serde_json::from_slice(&manifest).unwrap();
    assert_eq!(manifest["id"], id);
    assert!(manifest["instrument"]["value"]["channels"].is_array());
    let steps: Vec<_> = manifest["history"]
        .as_array()
        .unwrap()
        .iter()
        .map(|record| (record["kind"].clone(), record["state"].clone()))
        .collect();
    assert_eq!(
        steps,
        [
            (json!("zarr_from_raw"), json!("done")),
            (json!("pyramid"), json!("done"))
        ]
    );

    // Its thumbnail: the top level, 160 × 120.
    let (status, jpeg) = get(&router, &format!("/api/siq/data/{id}/thumb.jpg")).await;
    assert_eq!((status, jpeg_size(&jpeg)), (StatusCode::OK, (160, 120)));

    // The summary lists it.
    let page = invoke(&client, "/api/v1/datasets/summary", json!({})).await;
    assert_eq!(
        (&page["total"], &page["datasets"][0]["id"]),
        (&json!(1), &json!(id))
    );
    let page = invoke(&client, "/api/v1/datasets/summary", json!({ "offset": 1 })).await;
    assert_eq!(page["datasets"], json!([]));

    // Its exports, as jobs.
    let job = invoke(
        &client,
        "/api/v1/datasets/export_ome_tiff",
        json!({ "dataset": id, "image": "snap" }),
    )
    .await;
    assert_eq!(
        wait(&instrument, job.as_u64().unwrap()).await,
        JobState::Done
    );
    assert!(summary.path.join("exports/snap.ome.tif").is_file());
    let job = invoke(
        &client,
        "/api/v1/datasets/export_dzi",
        json!({ "dataset": id, "image": "snap", "c": 1 }),
    )
    .await;
    assert_eq!(
        wait(&instrument, job.as_u64().unwrap()).await,
        JobState::Done
    );
    assert!(summary.path.join("exports/dzi/snap_c1_z0_t0.dzi").is_file());
    let refused = client
        .post_json(
            "/api/v1/datasets/export_ome_tiff",
            Some(&json!({ "dataset": id, "image": "mosaic" })),
        )
        .await;
    assert_failed(&client, refused, "has no image mosaic").await;

    // Deleting it: refused while a job works on it, then done.
    let busy = instrument.jobs.submit(
        Box::new(Waits),
        Priority::Normal,
        Some(summary.path.clone()),
    );
    let refused = client
        .post_json("/api/v1/datasets/delete", Some(&json!({ "dataset": id })))
        .await;
    assert_failed(&client, refused, "is working on it").await;
    instrument.jobs.cancel(busy);
    assert_eq!(wait(&instrument, busy.0).await, JobState::Cancelled);
    let deleted = invoke(&client, "/api/v1/datasets/delete", json!({ "dataset": id })).await;
    assert_eq!(deleted, true);
    assert!(!summary.path.exists());
    let (status, _) = get(&router, &format!("/api/siq/data/{id}/manifest")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let deleted = invoke(&client, "/api/v1/datasets/delete", json!({ "dataset": id })).await;
    assert_eq!(deleted, false);
}

/// Checks that an invocation failed, saying `why`.
async fn assert_failed(client: &TestClient, invocation: TestResponse, why: &str) {
    let state = common::finished(client, invocation).await;
    assert_eq!(state["status"], "error", "{state}");
    assert!(state["error"].to_string().contains(why), "{state}");
}
