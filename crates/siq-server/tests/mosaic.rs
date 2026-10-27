//! The `mosaic_live` Thing (ADR-0059) over HTTP, on the simulated instrument with the simulator's
//! mount as its calibration: moving the stage over a 3 × 3 grid of fields paints them, within
//! bounds that match, with what the specimen shows there, and its snapshot is a dataset; a busy
//! live view loses no frames while it paints, within the VRAM budget, and the view streams at its
//! size; the settings are checked; and without a GPU, enabling it is refused. Each GPU test is
//! skipped, saying why, where there is no GPU. REQ-STI-001, REQ-PRV-001.

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::{config, finished, invoke, serve};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use siq_acquire::AcquisitionContext;
use siq_calib::{CalibrationRecord, Fit};
use siq_config::model::{CameraConfig, InstrumentConfig};
use siq_server::Instrument;
use siq_sim::camera::SimCameraSetup;
use siq_storage::zarr::ZarrImage;
use siq_synth::WorldInfo;
use siq_types::{Affine2, DatasetId, Micrometers, ObjectiveId, RectPx, Vec2Um};
use teta_wot::http::axum::body::Body;
use teta_wot::http::axum::http::StatusCode;
use teta_wot::testing::TestClient;

/// A field of view of the 256 × 192 sensor at 10×, through the mount's 0.2 % scale, in µm.
const FIELD: [f64; 2] = [256.0 * 0.376 / 1.002, 192.0 * 0.376 / 1.002];

struct Rig {
    client: TestClient,
    instrument: Arc<Instrument>,
    config: InstrumentConfig,
    _folders: [tempfile::TempDir; 2],
}

/// The simulated instrument of `config`, with settings and data of its own, and the simulator's
/// mount as the calibration in force.
async fn rig(mut config: InstrumentConfig) -> Rig {
    let folders = [tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap()];
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
    Rig {
        client,
        instrument,
        config,
        _folders: folders,
    }
}

/// Live view started, in focus, and the mosaic enabled: `false` if there is no GPU for it.
async fn painting(client: &TestClient) -> bool {
    invoke(client, "/api/v1/focus/move_to", json!({ "z": 2000.0 })).await;
    invoke(client, "/api/v1/camera/start_live", json!({})).await;
    let enabled = client
        .put_json("/api/v1/mosaic_live/enabled", &json!(true))
        .await;
    if enabled.status == StatusCode::UNPROCESSABLE_ENTITY {
        eprintln!("skipped: {}", enabled.text());
        return false;
    }
    assert!(enabled.status.is_success(), "{}", enabled.text());
    assert_eq!(client.get("/api/v1/mosaic_live/enabled").await.json(), true);
    true
}

fn painted(stats: &Value) -> u64 {
    stats["painted"].as_u64().unwrap()
}

/// Waits until `count` more frames are painted than `stats` says, for up to 10 s.
async fn more_painted(client: &TestClient, count: u64) {
    let from = painted(&client.get("/api/v1/mosaic_live/stats").await.json());
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while painted(&client.get("/api/v1/mosaic_live/stats").await.json()) < from + count {
        assert!(tokio::time::Instant::now() < deadline, "no frame painted");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// The normalized cross-correlation of two equally long series.
fn correlation(a: &[f64], b: &[f64]) -> f64 {
    let mean = |v: &[f64]| v.iter().sum::<f64>() / v.len() as f64;
    let (ma, mb) = (mean(a), mean(b));
    let (mut ab, mut aa, mut bb) = (0.0, 0.0, 0.0);
    for (x, y) in a.iter().zip(b) {
        let (x, y) = (x - ma, y - mb);
        (ab, aa, bb) = (ab + x * y, aa + x * x, bb + y * y);
    }
    ab / (aa * bb).sqrt()
}

// REQ-STI-001
#[tokio::test]
async fn moving_the_stage_paints_the_fields_it_visits() {
    let rig = rig(config()).await;
    let client = &rig.client;
    // A 3 × 3 grid of fields at 10 % overlap, from its first, each painted once the stage is there.
    let (x, y) = (-20_000.0, -15_000.0);
    let z = client.get("/api/v1/stage/position").await.json()["z"]
        .as_f64()
        .unwrap();
    invoke(
        client,
        "/api/v1/stage/move_to",
        json!({ "x": x, "y": y, "z": z }),
    )
    .await;
    if !painting(client).await {
        return;
    }
    let step = [0.9 * FIELD[0], 0.9 * FIELD[1]];
    for j in 0..3 {
        for i in 0..3 {
            let to =
                json!({ "x": x + f64::from(i) * step[0], "y": y + f64::from(j) * step[1], "z": z });
            invoke(client, "/api/v1/stage/move_to", to).await;
            more_painted(client, 2).await;
        }
    }
    // Bounds of the nine fields: the grid, half a field out, and the mount's turn of 0.4°.
    let bounds = client.get("/api/v1/mosaic_live/bounds").await.json();
    let near = |got: &Value, expected: f64| (got.as_f64().unwrap() - expected).abs() < 3.0;
    let (far_x, far_y) = (x + 2.0 * step[0], y + 2.0 * step[1]);
    assert!(
        near(&bounds["x0"], x - FIELD[0] / 2.0)
            && near(&bounds["y0"], y - FIELD[1] / 2.0)
            && near(&bounds["x1"], far_x + FIELD[0] / 2.0)
            && near(&bounds["y1"], far_y + FIELD[1] / 2.0),
        "{bounds}"
    );
    let px_per_um = client.get("/api/v1/mosaic_live/px_per_um").await.json();
    assert!((px_per_um.as_f64().unwrap() - 0.5 * 1.002 / 0.376).abs() < 1e-9);

    // The snapshot, where the stage stopped: what the specimen shows there, where the stage
    // truly is, and nowhere else.
    let saved = invoke(client, "/api/v1/mosaic_live/save_snapshot", json!({})).await;
    assert_eq!(saved["dataset"]["kind"], "mosaic", "{saved}");
    let id: DatasetId = serde_json::from_value(saved["dataset"]["id"].clone()).unwrap();
    let folder = rig.instrument.catalog.get(id).unwrap().path;
    let image = ZarrImage::open(&folder.join("zarr/mosaic.ome.zarr")).unwrap();
    let info = image.info().clone();
    let um = info.pixel_size[2];
    let reported = client.get("/api/v1/stage/position").await.json();
    let reported = [
        reported["x"].as_f64().unwrap(),
        reported["y"].as_f64().unwrap(),
    ];
    let truth = rig.instrument.state.pose();
    let error = [truth.x.0 - reported[0], truth.y.0 - reported[1]];
    // 40 × 40 canvas pixels about the stage, inside the last frame's feather.
    let corner = [
        ((reported[0] - info.origin[2]) / um) as u32 - 20,
        ((reported[1] - info.origin[1]) / um) as u32 - 20,
    ];
    let crop = image
        .read_region::<u16>(0, (0, 0, 0), RectPx::new(corner[0], corner[1], 40, 40))
        .unwrap();
    let shown: Vec<f64> = crop.pixels.iter().map(|&v| f64::from(v)).collect();
    let world = WorldInfo::from_config(&rig.config.sim.world, rig.config.sim.seed)
        .build()
        .unwrap();
    let specimen = |dx: i32, dy: i32| -> Vec<f64> {
        (0..40u32)
            .flat_map(|j| (0..40u32).map(move |i| (i, j)))
            .map(|(i, j)| {
                let at = [
                    info.origin[2] + (f64::from(corner[0] + i) + f64::from(dx)) * um + error[0],
                    info.origin[1] + (f64::from(corner[1] + j) + f64::from(dy)) * um + error[1],
                ];
                f64::from(world.intensity(Vec2Um::from_um(at[0], at[1]), Micrometers(um)))
            })
            .collect()
    };
    let here = correlation(&shown, &specimen(0, 0));
    let best = (-3..=3)
        .flat_map(|dy| (-3..=3).map(move |dx| (dx, dy)))
        .max_by(|a, b| {
            let (ca, cb) = (
                correlation(&shown, &specimen(a.0, a.1)),
                correlation(&shown, &specimen(b.0, b.1)),
            );
            ca.total_cmp(&cb)
        })
        .unwrap();
    eprintln!("the mosaic correlates {here:.3} with the specimen; best at {best:?} canvas pixels");
    assert!(here > 0.6, "{here}");
    assert!(best.0.abs() <= 1 && best.1.abs() <= 1, "{best:?}");

    // Painting stopped and cleared, nothing is painted: while it paints, the field the stage
    // stands on would be again at once.
    let stopped = client
        .put_json("/api/v1/mosaic_live/enabled", &json!(false))
        .await;
    assert!(stopped.status.is_success(), "{}", stopped.text());
    invoke(client, "/api/v1/mosaic_live/clear", json!({})).await;
    assert_eq!(
        client.get("/api/v1/mosaic_live/bounds").await.json(),
        Value::Null
    );
}

/// The width and height in a JPEG's frame header.
fn jpeg_size(jpeg: &[u8]) -> (u32, u32) {
    let at = jpeg
        .windows(2)
        .position(|w| w[0] == 0xFF && w[1] == 0xC0)
        .expect("a frame header");
    let height = u16::from_be_bytes([jpeg[at + 5], jpeg[at + 6]]);
    let width = u16::from_be_bytes([jpeg[at + 7], jpeg[at + 8]]);
    (u32::from(width), u32::from(height))
}

async fn next_jpeg(body: &mut Body) -> Vec<u8> {
    let chunk = tokio::time::timeout(Duration::from_secs(10), body.frame()).await;
    let data = chunk.expect("a part in time").expect("open").unwrap();
    let data = data.into_data().expect("data");
    let start = data
        .windows(2)
        .position(|w| w == [0xFF, 0xD8])
        .expect("a JPEG");
    data[start..].to_vec()
}

// REQ-STI-001, REQ-PRV-001
#[tokio::test]
async fn a_busy_live_view_loses_no_frames_while_it_paints() {
    let rig = rig(config()).await;
    let client = &rig.client;
    if !painting(client).await {
        return;
    }
    // The largest view, about where the stage is, while it moves back and forth.
    let at = client.get("/api/v1/stage/position").await.json();
    let viewport =
        json!({ "centre": [at["x"], at["y"]], "zoom": 1.0, "width": 1920, "height": 1080 });
    let written = client
        .put_json("/api/v1/mosaic_live/viewport", &viewport)
        .await;
    assert!(written.status.is_success(), "{}", written.text());
    for dx in [-150.0, 150.0, -150.0, 150.0] {
        invoke(client, "/api/v1/stage/move_by", json!({ "dx": dx })).await;
        more_painted(client, 1).await;
    }
    let mut body = client.stream("/api/v1/mosaic_live/view").await.into_body();
    assert_eq!(jpeg_size(&next_jpeg(&mut body).await), (1920, 1080));
    drop(body);

    let camera = client.get("/api/v1/camera/stats").await.json();
    assert!(camera["published"].as_u64().unwrap() > 10, "{camera}");
    assert_eq!(camera["lost_no_slot"], 0, "{camera}");
    assert_eq!(camera["lost_device"], 0, "{camera}");
    let stats = client.get("/api/v1/mosaic_live/stats").await.json();
    eprintln!("{camera}\n{stats}");
    assert!(
        painted(&stats) >= 4 && stats["views"].as_u64().unwrap() >= 1,
        "{stats}"
    );
    assert!(stats["vram_bytes"].as_u64() <= stats["vram_budget_bytes"].as_u64());

    let disabled = client
        .put_json("/api/v1/mosaic_live/enabled", &json!(false))
        .await;
    assert!(disabled.status.is_success(), "{}", disabled.text());
    assert_eq!(
        client.get("/api/v1/mosaic_live/enabled").await.json(),
        false
    );
}

#[tokio::test]
async fn the_settings_are_checked() {
    let rig = rig(config()).await;
    let client = &rig.client;
    let viewport = json!({ "centre": [10.0, -5.0], "zoom": 0.5, "width": 800, "height": 600 });
    for wrong in [
        json!({ "centre": [0.0, 0.0], "zoom": 0.0, "width": 800, "height": 600 }),
        json!({ "centre": [0.0, 0.0], "zoom": 1.0, "width": 4000, "height": 600 }),
        json!({ "centre": [0.0, 0.0], "zoom": 1.0, "width": 800 }),
    ] {
        let refused = client
            .put_json("/api/v1/mosaic_live/viewport", &wrong)
            .await;
        assert_eq!(refused.status, StatusCode::UNPROCESSABLE_ENTITY, "{wrong}");
    }
    let written = client
        .put_json("/api/v1/mosaic_live/viewport", &viewport)
        .await;
    assert!(written.status.is_success(), "{}", written.text());
    assert_eq!(
        client.get("/api/v1/mosaic_live/viewport").await.json(),
        viewport
    );
    for wrong in [0.0, 2.0] {
        let refused = client
            .put_json("/api/v1/mosaic_live/canvas_scale", &json!(wrong))
            .await;
        assert_eq!(refused.status, StatusCode::UNPROCESSABLE_ENTITY, "{wrong}");
    }
    let written = client
        .put_json("/api/v1/mosaic_live/canvas_scale", &json!(0.25))
        .await;
    assert!(written.status.is_success(), "{}", written.text());
    assert_eq!(
        client.get("/api/v1/mosaic_live/canvas_scale").await.json(),
        0.25
    );
    // Nothing painted: no bounds, no scale, and nothing to save.
    assert_eq!(
        client.get("/api/v1/mosaic_live/bounds").await.json(),
        Value::Null
    );
    assert_eq!(
        client.get("/api/v1/mosaic_live/px_per_um").await.json(),
        Value::Null
    );
    let saving = client
        .post_json("/api/v1/mosaic_live/save_snapshot", Some(&json!({})))
        .await;
    let saved = finished(client, saving).await;
    assert_eq!(saved["status"], "error", "{saved}");
    assert_eq!(saved["error"]["detail"], "nothing was painted");
}

#[tokio::test]
async fn without_a_gpu_enabling_is_refused() {
    let mut config = config();
    config.gpu.enabled = false;
    let rig = rig(config).await;
    let client = &rig.client;
    let refused = client
        .put_json("/api/v1/mosaic_live/enabled", &json!(true))
        .await;
    assert_eq!(refused.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(refused.text().contains("needs a GPU"), "{}", refused.text());
    assert_eq!(
        client.get("/api/v1/mosaic_live/enabled").await.json(),
        false
    );
}
