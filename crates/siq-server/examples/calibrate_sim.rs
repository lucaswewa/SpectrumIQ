//! The stage–camera calibration of the simulated instrument, as the service runs it: the camera of
//! `config/sim.toml`, mounted turned and sheared, calibrated against its stage through the
//! `calibration` Thing, and what was found set against what was injected.
//!
//! ```text
//! cargo run --release -p siq-server --example calibrate_sim [-- --settings <folder>]
//! ```
//!
//! The sensor is 4096 × 3072, binned 2 × 2. The calibration is kept in a temporary settings folder
//! removed at the end, or under `<folder>` with `--settings`, where a service given that folder
//! finds it in force. Smoke mode (`SIQ_SMOKE=1`) uses 1024 × 768.

use std::error::Error;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use siq_calib::CalibrationRecord;
use siq_config::model::CameraConfig;
use siq_server::Service;
use teta_wot::testing::TestClient;

/// Invokes `action` with `input`, and waits for it to end: its output.
async fn invoke(client: &TestClient, action: &str, input: Value) -> Result<Value, Box<dyn Error>> {
    let invocation = client.post_json(action, Some(&input)).await.json();
    let href = invocation["href"]
        .as_str()
        .ok_or("no invocation")?
        .replace("http://testserver", "");
    loop {
        let state = client.get(&href).await.json();
        match state["status"].as_str() {
            Some("pending" | "running") => tokio::time::sleep(Duration::from_millis(20)).await,
            Some("completed") => return Ok(state["output"].clone()),
            _ => return Err(format!("{action}: {}", state["error"]["detail"]).into()),
        }
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn Error>> {
    let smoke = std::env::var_os("SIQ_SMOKE").is_some();
    let args: Vec<String> = std::env::args().collect();
    let kept = args
        .iter()
        .position(|arg| arg == "--settings")
        .and_then(|at| args.get(at + 1))
        .map(PathBuf::from);
    let temporary = tempfile::tempdir()?;
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config/sim.toml");
    let mut config = siq_config::load(&path)?;
    config.server.settings = kept.unwrap_or_else(|| temporary.path().to_owned());
    config.server.audit = temporary.path().join("audit");
    let CameraConfig::Sim(camera) = &mut config.camera else {
        return Err("config/sim.toml has a simulated camera".into());
    };
    (camera.width, camera.height) = if smoke { (1024, 768) } else { (4096, 3072) };
    let (mount, stage_scale) = (config.sim.mount, config.sim.errors.scale_error);
    let settings = config.server.settings.clone();
    let (server, _instrument) = Service::build(config).await?.into_parts();
    let client = TestClient::start(server).await?;

    // Binned 2 × 2 at a quarter of the exposure, somewhere with room to move.
    let binned = json!({ "horizontal": 2, "vertical": 2 });
    client.put_json("/api/v1/camera/binning", &binned).await;
    let exposure = client.get("/api/v1/camera/exposure_ms").await.json();
    let quarter = json!(exposure.as_f64().unwrap_or(10.0) / 4.0);
    client
        .put_json("/api/v1/camera/exposure_ms", &quarter)
        .await;
    let somewhere = json!({ "x": -20000.0, "y": -15000.0, "z": 0.0 });
    invoke(&client, "/api/v1/stage/move_to", somewhere).await?;

    let began = Instant::now();
    let output = invoke(&client, "/api/v1/calibration/stage_camera", json!({})).await?;
    let record: CalibrationRecord = serde_json::from_value(output)?;
    println!(
        "calibrated objective {} in {:.1?}, from {} moves",
        record.objective,
        began.elapsed(),
        record.moves
    );

    // The objective's 10× on the 3.76 µm pixels, through the mount, and the stage's own scale
    // error, as the moves are the ones it reports.
    let expected = 10.0 / 3.76 * (1.0 + mount.scale_error) * (1.0 + stage_scale);
    println!("               found      injected   off");
    for (axis, scale) in ["x", "y"].into_iter().zip(record.scale) {
        let off = (scale / expected - 1.0) * 100.0;
        println!("scale {axis} px/µm  {scale:<10.6} {expected:<10.6} {off:+.4} %");
    }
    let (rotation, shear) = (record.rotation_deg, record.shear);
    let off = rotation - mount.rotation;
    println!(
        "rotation °     {rotation:<10.5} {:<10.5} {off:+.4}°",
        mount.rotation
    );
    let off = shear - mount.shear;
    println!(
        "shear          {shear:<10.6} {:<10.6} {off:+.6}",
        mount.shear
    );
    println!(
        "pixel {:.5} µm; residuals {:.2} µm RMS, {:.2} µm at most",
        record.px_size_um, record.residual_rms_um, record.residual_max_um
    );
    if let Some([x, y]) = record.backlash_um {
        println!("backlash {x:.2} µm in x, {y:.2} µm in y (2 injected)");
    }
    println!(
        "kept under {}",
        settings
            .join("calibrations")
            .join(format!("objective-{}", record.objective))
            .display()
    );
    client.stop().await;
    Ok(())
}
