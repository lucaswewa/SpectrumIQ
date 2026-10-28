//! The live mosaic of the simulated instrument, as the service paints it: the camera of
//! `config/sim.toml` at 2048 × 1536 over its cells, with the simulator's mount as the calibration
//! in force, stepped field by field over a 6 × 6-field area at 10 % overlap while the live view
//! runs, each field painted where its pose puts it; then the whole area's view written as a JPEG,
//! and a 1:1 view of where four fields meet, and the mosaic saved as a dataset.
//!
//! ```text
//! cargo run --release -p siq-server --example live_mosaic_sim [-- <out.jpg>]
//! ```
//!
//! It prints the paint's time, the frames lost (none), the pages and their GPU memory, and the
//! snapshot's size; the views go to `<out.jpg>` and `<out>_corner.jpg`, or `live_mosaic_sim.jpg`
//! and its corner in the temporary folder. Smoke mode (`SIQ_SMOKE=1`) uses a 512 × 384 sensor and 3 × 3 fields.

use std::error::Error;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use http_body_util::BodyExt;
use serde_json::{Value, json};
use siq_acquire::AcquisitionContext;
use siq_calib::{CalibrationRecord, Fit};
use siq_config::model::CameraConfig;
use siq_server::Service;
use siq_sim::camera::SimCameraSetup;
use siq_types::{Affine2, ObjectiveId};
use teta_wot::testing::TestClient;

/// Invokes `action` with `input`, and waits for it to end: its output.
async fn invoke(client: &TestClient, action: &str, input: Value) -> Result<Value, Box<dyn Error>> {
    let invocation = client.post_json(action, Some(&input)).await.json();
    let href = invocation["href"]
        .as_str()
        .ok_or_else(|| format!("{action}: {invocation}"))?
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

/// `viewport` set, and the next view it streams written to `out`.
async fn view(client: &TestClient, viewport: Value, out: &Path) -> Result<(), Box<dyn Error>> {
    let written = client
        .put_json("/api/v1/mosaic_live/viewport", &viewport)
        .await;
    if !written.status.is_success() {
        return Err(format!("the viewport: {}", written.text()).into());
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    let mut body = client.stream("/api/v1/mosaic_live/view").await.into_body();
    let part = tokio::time::timeout(Duration::from_secs(10), body.frame()).await?;
    let part = part.ok_or("the view's stream ended")??.into_data();
    let part = part.map_err(|_| "a part of the stream without data")?;
    let start = part
        .windows(2)
        .position(|w| w == [0xFF, 0xD8])
        .ok_or("no JPEG in the stream's part")?;
    std::fs::write(out, &part[start..])?;
    Ok(())
}

async fn stats(client: &TestClient) -> Value {
    client.get("/api/v1/mosaic_live/stats").await.json()
}

/// Waits until two more frames are painted: one at least since the stage arrived.
async fn painted_here(client: &TestClient) -> Result<(), Box<dyn Error>> {
    let count = |stats: &Value| stats["painted"].as_u64().unwrap_or(0);
    let from = count(&stats(client).await);
    let deadline = Instant::now() + Duration::from_secs(20);
    while count(&stats(client).await) < from + 2 {
        if Instant::now() > deadline {
            return Err("no frame was painted".into());
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    Ok(())
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn Error>> {
    let smoke = std::env::var_os("SIQ_SMOKE").is_some();
    let out = std::env::args().nth(1).map_or_else(
        || std::env::temp_dir().join("live_mosaic_sim.jpg"),
        PathBuf::from,
    );
    let folders = [tempfile::tempdir()?, tempfile::tempdir()?];
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config/sim.toml");
    let mut config = siq_config::load(&path)?;
    config.server.settings = folders[0].path().to_owned();
    config.server.audit = folders[0].path().join("audit");
    config.server.data_root = folders[1].path().to_owned();
    config.server.min_free_gb = 0.0;
    let CameraConfig::Sim(camera) = &mut config.camera else {
        return Err("config/sim.toml has a simulated camera".into());
    };
    let (sensor, fields) = if smoke {
        ([512, 384], 3)
    } else {
        ([2048, 1536], 6)
    };
    (camera.width, camera.height) = (sensor[0], sensor[1]);
    // The calibration the stage–camera routine would find: the mount, at 10× on 3.76 µm pixels.
    let setup = SimCameraSetup::from_config(camera, &config.objectives, &config.sim);
    let l = setup
        .mount
        .compose(&Affine2::scale(10.0 / 3.76, 10.0 / 3.76));
    let (server, instrument) = Service::build(config).await?.into_parts();
    let client = TestClient::start(server).await?;
    let fit = Fit {
        affine: l,
        residuals: Vec::new(),
    };
    let record = CalibrationRecord::new(ObjectiveId(1), &fit, None, "the simulator's mount");
    instrument.calibrations.save(&record, &[])?;
    let acquisition = &instrument.acquisition;
    acquisition.set_context(AcquisitionContext {
        objective: Some(ObjectiveId(1)),
        calibration: Some(record.id),
        ..acquisition.context()
    });

    // The area's first field, in focus, the live view running and the mosaic painting.
    let field = sensor.map(|pixels| f64::from(pixels) / record.scale[0]);
    let step = field.map(|side| 0.9 * side);
    let (x, y) = (-20_000.0, -15_000.0);
    let z = client.get("/api/v1/stage/position").await.json()["z"]
        .as_f64()
        .unwrap_or(0.0);
    invoke(
        &client,
        "/api/v1/stage/move_to",
        json!({ "x": x, "y": y, "z": z }),
    )
    .await?;
    invoke(&client, "/api/v1/focus/move_to", json!({ "z": 2000.0 })).await?;
    invoke(&client, "/api/v1/camera/start_live", json!({})).await?;
    let enabled = client
        .put_json("/api/v1/mosaic_live/enabled", &json!(true))
        .await;
    if !enabled.status.is_success() {
        println!("no live mosaic: {}", enabled.text());
        client.stop().await;
        return Ok(());
    }

    // Field by field, along each row and back along the next.
    let began = Instant::now();
    for j in 0..fields {
        for k in 0..fields {
            let i = if j % 2 == 0 { k } else { fields - 1 - k };
            let to = json!({
                "x": x + f64::from(i) * step[0],
                "y": y + f64::from(j) * step[1],
                "z": z,
            });
            invoke(&client, "/api/v1/stage/move_to", to).await?;
            painted_here(&client).await?;
        }
    }
    let took = began.elapsed();
    let done = stats(&client).await;
    let camera = client.get("/api/v1/camera/stats").await.json();
    println!(
        "{} fields of {} × {} painted in {took:.1?}: {} frames painted, {} passed over, the last in {:.1} ms",
        fields * fields,
        sensor[0],
        sensor[1],
        done["painted"],
        done["passed_over"],
        done["last_paint_ms"].as_f64().unwrap_or(f64::NAN),
    );
    println!(
        "camera: {} frames published, {} lost for want of a slot, {} by the device",
        camera["published"], camera["lost_no_slot"], camera["lost_device"]
    );
    println!(
        "{} pages on the GPU ({:.0} MB of {:.0} MB), {} in RAM",
        done["pages_on_gpu"],
        done["vram_bytes"].as_f64().unwrap_or(0.0) / 1e6,
        done["vram_budget_bytes"].as_f64().unwrap_or(0.0) / 1e6,
        done["pages_in_ram"],
    );

    // The whole area in view, then where the first four fields meet, at the canvas's detail.
    let bounds = client.get("/api/v1/mosaic_live/bounds").await.json();
    let px_per_um = client.get("/api/v1/mosaic_live/px_per_um").await.json();
    let (x0, y0, x1, y1) = (
        bounds["x0"].as_f64().unwrap_or(0.0),
        bounds["y0"].as_f64().unwrap_or(0.0),
        bounds["x1"].as_f64().unwrap_or(0.0),
        bounds["y1"].as_f64().unwrap_or(0.0),
    );
    println!("painted from ({x0:.1}, {y0:.1}) to ({x1:.1}, {y1:.1}) µm");
    let (width, height) = (1920.0, 1080.0);
    let canvas = [x1 - x0, y1 - y0].map(|um| um * px_per_um.as_f64().unwrap_or(1.0));
    let zoom = (width / canvas[0]).min(height / canvas[1]).min(1.0);
    let viewport = json!({
        "centre": [(x0 + x1) / 2.0, (y0 + y1) / 2.0],
        "zoom": zoom,
        "width": width,
        "height": height,
    });
    view(&client, viewport, &out).await?;
    println!(
        "the view, 1920 × 1080 at zoom {zoom:.3}, in {}",
        out.display()
    );
    let corner = out.with_file_name(format!(
        "{}_corner.jpg",
        out.file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("view")
    ));
    let meet = [x + step[0] / 2.0, y + step[1] / 2.0];
    let viewport = json!({ "centre": meet, "zoom": 1.0, "width": 512, "height": 384 });
    view(&client, viewport, &corner).await?;
    println!("where four fields meet, 1:1, in {}", corner.display());

    let saved = invoke(&client, "/api/v1/mosaic_live/save_snapshot", json!({})).await?;
    println!(
        "saved as dataset {} ({}), image `{}`",
        saved["dataset"]["id"], saved["dataset"]["name"], saved["image"]
    );
    client.stop().await;
    Ok(())
}
