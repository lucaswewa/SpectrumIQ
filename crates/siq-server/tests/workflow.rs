//! The `workflow` Thing (ADR-0056) over HTTP, on the simulated instrument: a tile scan run under
//! the global lock, its progress observed and a manual move refused meanwhile, its dataset told to
//! the hooks; a time-lapse paused, resumed and cancelled by deleting its invocation; plans refused
//! with 422; and the saved positions, kept across restarts, which a multi-position visits.
//! REQ-WF-005, REQ-WF-006, REQ-NET-002.

mod common;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::{config, finished, finished_within, invoke, serve};
use serde_json::{Value, json};
use siq_acquire::AcquisitionContext;
use siq_calib::{CalibrationRecord, Fit};
use siq_config::model::{CameraConfig, InstrumentConfig};
use siq_server::{DatasetHook, Instrument};
use siq_sim::camera::SimCameraSetup;
use siq_storage::DatasetKind;
use siq_types::{Affine2, DatasetId, ObjectiveId};
use siq_workflow::engine::{LAYOUT, Layout, Outcome, RunReport};
use teta_wot::http::axum::http::StatusCode;
use teta_wot::testing::TestClient;

/// Every dataset the hooks were told of: how its run ended, and its frames.
#[derive(Default)]
struct Told(Mutex<Vec<(Outcome, u64)>>);

impl DatasetHook for Told {
    fn on_dataset_complete(&self, _: &Instrument, report: &RunReport) {
        self.0.lock().unwrap().push((report.outcome, report.frames));
    }
}

struct Rig {
    client: TestClient,
    instrument: Arc<Instrument>,
    told: Arc<Told>,
    config: InstrumentConfig,
    _folders: [tempfile::TempDir; 2],
}

/// The simulated instrument, with settings and data of its own, and the simulator's mount as the
/// calibration in force, as the stage–camera routine would find it.
async fn rig() -> Rig {
    let folders = [tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap()];
    let mut config = config();
    config.server.settings = folders[0].path().to_owned();
    config.server.data_root = folders[1].path().to_owned();
    config.server.min_free_gb = 0.0;
    let CameraConfig::Sim(camera) = &config.camera else {
        panic!("a simulated camera")
    };
    let setup = SimCameraSetup::from_config(camera, &config.objectives, &config.sim);
    let affine = setup
        .mount
        .compose(&Affine2::scale(10.0 / 3.76, 10.0 / 3.76));
    let (client, instrument) = serve(config.clone()).await;
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
    let told = Arc::new(Told::default());
    instrument.on_dataset_complete(told.clone());
    Rig {
        client,
        instrument,
        told,
        config,
        _folders: folders,
    }
}

/// GETs `path` until `done` says so of its value, for up to 20 s: the last value.
async fn until(client: &TestClient, path: &str, done: impl Fn(&Value) -> bool) -> Value {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    loop {
        let value = client.get(path).await.json();
        if done(&value) || tokio::time::Instant::now() > deadline {
            return value;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

fn done(progress: &Value) -> u64 {
    progress["done"].as_u64().unwrap_or(0)
}

// REQ-WF-005, REQ-NET-002
#[tokio::test]
async fn a_tile_scan_holds_the_global_lock_and_reports_its_progress() {
    let rig = rig().await;
    let client = &rig.client;
    // 2 × 2 tiles at 10 % overlap, each a z-stack of three planes about the focus.
    let step = [0.9 * 256.0 * 0.376 / 1.002, 0.9 * 192.0 * 0.376 / 1.002];
    let (x, y) = (-20_000.0, -15_000.0);
    let region = json!({
        "shape": "rect",
        "x0": x - 0.8 * step[0],
        "y0": y - 0.8 * step[1],
        "x1": x + 0.8 * step[0],
        "y1": y + 0.8 * step[1],
    });
    let scan = json!({
        "name": "scan",
        "region": region,
        "channels": [1],
        "z_stack": { "range_um": 4.0, "step_um": 2.0 },
    });
    let invocation = client
        .post_json("/api/v1/workflow/tile_scan", Some(&scan))
        .await;

    // While it runs: its state and progress, and a manual move refused.
    let progress = until(client, "/api/v1/workflow/progress", |p| done(p) >= 2).await;
    // The channel once, then at each tile a move and three planes, each a focus and a frame, and
    // the focus back.
    assert_eq!(progress["total"], 1 + 4 * (1 + 3 * 2 + 1), "{progress}");
    assert_eq!(client.get("/api/v1/workflow/state").await.json(), "running");
    let to = json!({ "x": -1000.0, "y": -1000.0, "z": 0.0 });
    let moving = client.post_json("/api/v1/stage/move_to", Some(&to)).await;
    let refused = finished(client, moving).await;
    assert_eq!(refused["status"], "error", "{refused}");
    assert!(refused.to_string().contains("GlobalLockBusy"), "{refused}");

    let ended = finished_within(client, invocation, Duration::from_secs(120)).await;
    assert_eq!(ended["status"], "completed", "{ended}");
    let output = &ended["output"];
    assert_eq!(output["outcome"], "finished", "{output}");
    assert_eq!(output["frames"], 12);
    assert_eq!(output["steps"], json!([33, 33]));
    assert_eq!(client.get("/api/v1/workflow/state").await.json(), "idle");
    let progress = client.get("/api/v1/workflow/progress").await.json();
    assert_eq!(done(&progress), 33);
    // Listed, and the hooks told.
    let id: DatasetId = serde_json::from_value(output["dataset"].clone()).unwrap();
    let listed = rig.instrument.catalog.get(id).expect("listed");
    assert_eq!((listed.kind, listed.frames), (DatasetKind::TileScan, 12));
    assert_eq!(*rig.told.0.lock().unwrap(), [(Outcome::Finished, 12)]);
    rig.client.stop().await;
}

// REQ-WF-005
#[tokio::test]
async fn a_time_lapse_pauses_resumes_and_is_cancelled_by_deleting_its_invocation() {
    let rig = rig().await;
    let client = &rig.client;
    // Nothing to pause yet.
    let pausing = client
        .post_json("/api/v1/workflow/pause", Some(&json!({})))
        .await;
    let refused = finished(client, pausing).await;
    assert!(
        refused.to_string().contains("no run is under way"),
        "{refused}"
    );

    // A frame every half second, a thousand times.
    let lapse = json!({ "name": "lapse", "interval_s": 0.5, "count": 1000 });
    let invocation = client
        .post_json("/api/v1/workflow/time_lapse", Some(&lapse))
        .await;
    assert_eq!(
        invocation.status,
        StatusCode::CREATED,
        "{}",
        invocation.text()
    );
    let href = invocation.json()["href"]
        .as_str()
        .unwrap()
        .replace("http://testserver", "");
    until(client, "/api/v1/workflow/progress", |p| done(p) >= 2).await;
    invoke(client, "/api/v1/workflow/pause", json!({})).await;
    let state = until(client, "/api/v1/workflow/state", |s| s == "paused").await;
    assert_eq!(state, "paused");
    let paused = done(&client.get("/api/v1/workflow/progress").await.json());
    tokio::time::sleep(Duration::from_millis(1200)).await;
    let still = done(&client.get("/api/v1/workflow/progress").await.json());
    assert_eq!(still, paused, "no steps while paused");
    invoke(client, "/api/v1/workflow/resume", json!({})).await;
    let progress = until(client, "/api/v1/workflow/progress", |p| done(p) > paused).await;
    assert!(done(&progress) > paused, "{progress}");

    let deleted = client.delete(&href).await;
    assert!(deleted.status.is_success(), "{}", deleted.text());
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    let mut ended = client.get(&href).await.json();
    while ended["status"] == "running" && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(20)).await;
        ended = client.get(&href).await.json();
    }
    assert_eq!(ended["status"], "cancelled", "{ended}");
    assert_eq!(client.get("/api/v1/workflow/state").await.json(), "idle");
    // The dataset keeps what was taken.
    let told = rig.told.0.lock().unwrap().clone();
    assert!(
        matches!(told[..], [(Outcome::Cancelled, frames)] if frames > paused / 2),
        "{told:?}"
    );
    rig.client.stop().await;
}

// REQ-WF-006
#[tokio::test]
async fn plans_that_could_not_run_are_refused_with_422() {
    let rig = rig().await;
    let client = &rig.client;
    let region = json!({ "shape": "rect", "x0": -100.0, "y0": -100.0, "x1": 0.0, "y1": 0.0 });
    let refused = [
        (
            "z_stack",
            json!({ "name": "z", "planes": { "range_um": 4.0, "step_um": 0.0 } }),
            "a z-stack",
        ),
        (
            "time_lapse",
            json!({ "name": "t", "interval_s": 1.0 }),
            "a count or a duration",
        ),
        (
            "run",
            json!({ "name": " ", "steps": [{ "step": "snap" }] }),
            "it needs a name",
        ),
        (
            "run",
            json!({ "name": "r", "steps": [{ "step": "teleport" }] }),
            "teleport",
        ),
        (
            "tile_scan",
            json!({ "name": "s", "region": region, "focus": { "mode": "surface" } }),
            "before any focus map",
        ),
    ];
    for (action, body, why) in refused {
        let path = format!("/api/v1/workflow/{action}");
        let response = client.post_json(&path, Some(&body)).await;
        let text = response.text();
        assert_eq!(
            response.status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{action}: {text}"
        );
        assert!(text.contains(why), "{action}: {text}");
    }
    // One this instrument can't run fails its invocation at once, before any dataset.
    let steps = json!([{ "step": "channel", "id": 9 }, { "step": "snap" }]);
    let unknown = json!({ "name": "c", "steps": steps });
    let invocation = client
        .post_json("/api/v1/workflow/run", Some(&unknown))
        .await;
    let failed = finished(client, invocation).await;
    assert_eq!(failed["status"], "error", "{failed}");
    assert!(failed.to_string().contains("no channel 9"), "{failed}");
    assert!(rig.instrument.catalog.list().is_empty());
    rig.client.stop().await;
}

// REQ-WF-004, REQ-WF-006
#[tokio::test]
async fn saved_positions_are_kept_and_a_multi_position_visits_them() {
    let rig = rig().await;
    let client = &rig.client;
    let save = "/api/v1/workflow/save_position";
    // Where the stage and the focus are, and two given.
    invoke(client, save, json!({ "name": "start" })).await;
    invoke(
        client,
        save,
        json!({ "name": "a", "x": -20000.0, "y": -15000.0 }),
    )
    .await;
    let b = json!({ "name": "b", "x": -19800.0, "y": -15000.0, "focus": 2001.0 });
    invoke(client, save, b).await;
    let again = client.post_json(save, Some(&json!({ "name": "a" }))).await;
    let refused = finished(client, again).await;
    assert_eq!(refused["status"], "error", "{refused}");
    let rename = json!({ "name": "b", "to": "c" });
    invoke(client, "/api/v1/workflow/rename_position", rename).await;
    let removed = json!({ "name": "start" });
    let left = invoke(client, "/api/v1/workflow/remove_position", removed).await;
    let names = |positions: &Value| -> Vec<String> {
        let positions = positions.as_array().unwrap().iter();
        positions
            .map(|p| p["name"].as_str().unwrap().to_owned())
            .collect()
    };
    assert_eq!(names(&left), ["a", "c"]);
    assert_eq!(client.get("/api/v1/workflow/positions").await.json(), left);
    let start = &client.get("/api/v1/workflow/positions").await.json()[0];
    assert_eq!(
        (start["z"].as_f64(), start["focus"].as_f64()),
        (Some(0.0), Some(2000.0))
    );

    // A multi-position given none visits them.
    let visit = json!({ "name": "visit" });
    let output = invoke(client, "/api/v1/workflow/multi_position", visit).await;
    assert_eq!(
        (&output["outcome"], &output["frames"]),
        (&json!("finished"), &json!(2))
    );
    let id: DatasetId = serde_json::from_value(output["dataset"].clone()).unwrap();
    let path = rig.instrument.catalog.get(id).unwrap().path.join(LAYOUT);
    let layout: Layout = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let visited: Vec<_> = layout
        .positions
        .iter()
        .map(|placed| (placed.p, placed.position.name.as_str(), placed.position.x))
        .collect();
    assert_eq!(
        visited,
        [(Some(0), "a", -20000.0), (Some(1), "c", -19800.0)]
    );
    rig.client.stop().await;

    // Kept in the settings folder: a server started on it has them.
    let (again, _) = serve(rig.config.clone()).await;
    let kept = again.get("/api/v1/workflow/positions").await.json();
    assert_eq!(names(&kept), ["a", "c"]);
    again.stop().await;
}
