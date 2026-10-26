//! The stage–camera calibration on the simulated instrument (ADR-0049), end to end over HTTP:
//! the camera mounted turned 0.4°, with 0.2 % more pixels per µm and a shear of 0.001, the stage
//! with its backlash, scale error and noise. The calibration recovers the mount within 0.1 % in
//! scale and 0.05° in rotation (M5's criterion), measures the backlash, is kept as a version, and
//! frames and datasets carry it from then on. REQ-CAL-001, REQ-CAL-003.

mod common;

use std::time::Duration;

use common::{config, finished_within, invoke};
use serde_json::json;
use siq_calib::CalibrationRecord;
use siq_config::model::CameraConfig;
use siq_server::Service;
use teta_wot::testing::TestClient;

#[tokio::test]
async fn the_calibration_recovers_the_camera_s_mount_and_is_stamped_on_frames() {
    let (settings, data) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let mut config = config();
    config.server.settings = settings.path().to_owned();
    config.server.data_root = data.path().to_owned();
    config.server.min_free_gb = 0.0;
    let CameraConfig::Sim(camera) = &mut config.camera else {
        panic!("a simulated camera")
    };
    // A field of 1.5 × 1.2 mm: against the stage's 0.5 µm, moves out to 0.4 of it measure the
    // scale to 0.03 % and the rotation to 0.015°, a third of the targets.
    (camera.width, camera.height) = (4096, 3072);
    let (mount, stage_scale) = (config.sim.mount, config.sim.errors.scale_error);
    assert!(mount.rotation > 0.0, "sim.toml turns the camera");
    let (server, instrument) = Service::build(config).await.unwrap().into_parts();
    let client = TestClient::start(server).await.unwrap();

    // Binned 2 × 2, as a calibration needn't be taken at full resolution: the shifts are scaled.
    // Four pixels' light in each, so a quarter of the exposure.
    let binned = json!({ "horizontal": 2, "vertical": 2 });
    let written = client.put_json("/api/v1/camera/binning", &binned).await;
    assert!(written.status.is_success(), "{}", written.text());
    let exposure = client.get("/api/v1/camera/exposure_ms").await.json();
    let quarter = json!(exposure.as_f64().unwrap() / 4.0);
    let written = client
        .put_json("/api/v1/camera/exposure_ms", &quarter)
        .await;
    assert!(written.status.is_success(), "{}", written.text());

    // From the corner of the stage's travel, where it starts: the moves are made a little way in,
    // and the stage comes back.
    let at = client.get("/api/v1/stage/position").await.json();
    let calibrating = client
        .post_json("/api/v1/calibration/stage_camera", Some(&json!({})))
        .await;
    let done = finished_within(&client, calibrating, Duration::from_secs(300)).await;
    assert_eq!(done["status"], "completed", "{done}");
    let record: CalibrationRecord = serde_json::from_value(done["output"].clone()).unwrap();
    assert_eq!(client.get("/api/v1/stage/position").await.json(), at);

    // What it should find: the objective's 10× on the 3.76 µm sensor, through the mount, and the
    // stage's own scale error, as the reported moves are the stage's.
    let nominal = 10.0 / 3.76;
    let expected = nominal * (1.0 + mount.scale_error) * (1.0 + stage_scale);
    let scale = record.scale.map(|scale| (scale / expected - 1.0).abs());
    let rotation = (record.rotation_deg - mount.rotation).abs();
    assert!(
        scale.iter().all(|&off| off < 1e-3),
        "scale off by {scale:?}: {record:?}"
    );
    assert!(
        rotation < 0.05,
        "rotation off by {rotation:.4}°: {record:?}"
    );
    assert!((record.shear - mount.shear).abs() < 2e-3, "{record:?}");
    assert!(record.residual_rms_um < 1.0, "{record:?}");
    let backlash = record.backlash_um.expect("the backlash is measured");
    assert!(
        backlash.iter().all(|b| (b - 2.0).abs() < 1.0),
        "{backlash:?}"
    );

    // Kept, in force, and on the frames from now on.
    let objective = record.objective;
    assert_eq!(
        instrument.calibrations.current(objective),
        Some(record.clone())
    );
    let status = client.get("/api/v1/calibration/status").await.json();
    let entry = status
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["objective"] == objective.0)
        .unwrap();
    assert_eq!(entry["calibration"]["id"], json!(record.id));
    let frame = instrument.acquisition.snap(false).await.unwrap();
    assert_eq!(frame.meta().calibration, Some(record.id));
    assert_eq!(frame.meta().objective, Some(objective));
    let files = std::fs::read_dir(settings.path().join("calibrations")).unwrap();
    assert_eq!(files.count(), 1, "one objective's folder");
    // A dataset keeps a copy.
    invoke(&client, "/api/v1/datasets/snap_to_dataset", json!({})).await;
    let mut folders = std::fs::read_dir(data.path().join("datasets")).unwrap();
    let folder = folders.next().unwrap().unwrap().path();
    let manifest = std::fs::read_to_string(folder.join("manifest.json")).unwrap();
    let manifest: serde_json::Value = serde_json::from_str(&manifest).unwrap();
    assert_eq!(manifest["calibration"]["value"]["id"], json!(record.id));
    client.stop().await;
}
