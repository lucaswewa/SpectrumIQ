//! The dark and the flat on the simulated instrument (ADR-0050), end to end over HTTP: a blank
//! slide, the camera's cos⁴ vignetting (the corners at 0.8 of the centre), a dark taken with the
//! lights off and turned back on, a flat taken out of focus and the focus put back, the flat
//! within 2 % of the vignetting, both in force and in `status`, the previews' correction, and a
//! tile scan's flat estimated as a job; and a colour camera's flat, of its luminance (ADR-0071).
//! REQ-CAL-002.

mod common;

use std::time::Duration;

use common::{config, invoke, serve};
use serde_json::json;
use siq_calib::shading::{ShadingKind, ShadingRecord};
use siq_config::model::WorldConfig;
use siq_jobs::{JobId, JobState};
use siq_sim::optics::Vignetting;
use siq_types::{Micrometers, ObjectiveId};

/// A blank slide: a uniform 16-bit PNG of 16 × 16 pixels, 10 mm each, centred on the origin.
fn blank_slide(folder: &std::path::Path) -> std::path::PathBuf {
    let path = folder.join("blank.png");
    let file = std::io::BufWriter::new(std::fs::File::create(&path).unwrap());
    let mut encoder = png::Encoder::new(file, 16, 16);
    encoder.set_color(png::ColorType::Grayscale);
    encoder.set_depth(png::BitDepth::Sixteen);
    let mut writer = encoder.write_header().unwrap();
    writer
        .write_image_data(&48_000u16.to_be_bytes().repeat(256))
        .unwrap();
    path
}

/// How far `flat`'s surface is from the simulator's vignetting on its 256 × 192 sensor, at
/// worst: a share, its level aside.
fn off(flat: &ShadingRecord) -> f64 {
    let surface = flat.flat.as_ref().unwrap();
    let (cx, cy) = (128.0, 96.0);
    let light = |x: f64, y: f64| {
        let rho2 = ((x - cx) * (x - cx) + (y - cy) * (y - cy)) / (cx * cx + cy * cy);
        Vignetting::Cos4 { corner: 0.8 }.at(rho2)
    };
    let places: Vec<_> = (0..=8)
        .flat_map(|j| (0..=8).map(move |i| (f64::from(i) * 32.0, f64::from(j) * 24.0)))
        .collect();
    let ratios: Vec<f64> = places
        .iter()
        .map(|&(x, y)| surface.at(x, y) / light(x, y))
        .collect();
    let mean = ratios.iter().sum::<f64>() / ratios.len() as f64;
    ratios
        .iter()
        .map(|r| (r / mean - 1.0).abs())
        .fold(0.0, f64::max)
}

#[tokio::test]
async fn a_colour_camera_s_flat_is_its_luminance_s() {
    let (folder, settings) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let mut config = config();
    config.server.settings = settings.path().to_owned();
    config.sim.world = WorldConfig::Image {
        path: blank_slide(folder.path()),
        pixel_size: Micrometers(10_000.0),
    };
    let siq_config::model::CameraConfig::Sim(camera) = &mut config.camera else {
        panic!("a simulated camera")
    };
    camera.cfa = Some(siq_types::Cfa::Gbrg);
    let (client, _) = serve(config).await;
    invoke(
        &client,
        "/api/v1/imaging/select_channel",
        json!({ "id": 1 }),
    )
    .await;
    let to = json!({ "x": -20000.0, "y": -15000.0, "z": 0.0 });
    invoke(&client, "/api/v1/stage/move_to", to).await;
    invoke(
        &client,
        "/api/v1/calibration/capture_dark",
        json!({ "frames": 8 }),
    )
    .await;
    let input = json!({ "defocus_um": 50.0, "frames": 18 });
    let flat = invoke(&client, "/api/v1/calibration/capture_flat", input).await;
    let worst = off(&serde_json::from_value(flat).unwrap());
    assert!(worst < 0.02, "the flat off by {:.2} %", worst * 100.0);
    client.stop().await;
}

#[tokio::test]
async fn a_dark_and_a_flat_are_captured_kept_and_shown_and_a_scan_s_flat_is_estimated() {
    let (folder, settings) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let mut config = config();
    config.server.settings = settings.path().to_owned();
    config.server.data_root = folder.path().join("data");
    config.sim.world = WorldConfig::Image {
        path: blank_slide(folder.path()),
        pixel_size: Micrometers(10_000.0),
    };
    // A tile scan to estimate from, of the cells world: 3 × 2 tiles of 256 × 192.
    let scan = siq_datagen::tilescan::TileScanArgs {
        world: "cells".to_owned(),
        columns: 3,
        rows: 2,
        width: 256,
        height: 192,
        overlap: 0.15,
        stage_error: 2.0,
        light: 1.0,
        no_mosaic: true,
        colour: false,
    };
    let output = siq_datagen::Output {
        data_root: config.server.data_root.clone(),
        smoke: false,
        seed: 3,
    };
    siq_datagen::tilescan::tilescan(&scan, &output).unwrap();
    let (client, instrument) = serve(config).await;

    // In the brightfield channel, with room to move.
    invoke(
        &client,
        "/api/v1/imaging/select_channel",
        json!({ "id": 1 }),
    )
    .await;
    let to = json!({ "x": -20000.0, "y": -15000.0, "z": 0.0 });
    invoke(&client, "/api/v1/stage/move_to", to).await;
    let lights = client.get("/api/v1/illumination/levels").await.json();
    let focus = client.get("/api/v1/focus/position").await.json();

    // The dark: the sensor's 100 ADU, and the lights as they were.
    let dark = invoke(
        &client,
        "/api/v1/calibration/capture_dark",
        json!({ "frames": 8 }),
    )
    .await;
    let dark: ShadingRecord = serde_json::from_value(dark).unwrap();
    assert_eq!(dark.kind, ShadingKind::Dark);
    assert!((dark.level - 100.0).abs() < 2.0, "{dark:?}");
    assert_eq!(
        client.get("/api/v1/illumination/levels").await.json(),
        lights
    );

    // The flat, 50 µm out of focus, less the dark, and the focus back.
    let input = json!({ "defocus_um": 50.0, "frames": 18 });
    let flat = invoke(&client, "/api/v1/calibration/capture_flat", input).await;
    let flat: ShadingRecord = serde_json::from_value(flat).unwrap();
    assert_eq!(client.get("/api/v1/focus/position").await.json(), focus);
    assert_eq!(
        (flat.kind, flat.frames, flat.level),
        (ShadingKind::Flat, 18, dark.level)
    );
    let worst = off(&flat);
    assert!(worst < 0.02, "the flat off by {:.2} %", worst * 100.0);

    // Both in force, and in `status`.
    let status = client.get("/api/v1/calibration/status").await.json();
    let brightfield = &status[0]["shading"][0];
    assert_eq!(brightfield["name"], "Brightfield");
    assert_eq!(brightfield["dark"]["id"], json!(dark.id));
    assert_eq!(brightfield["flat"]["id"], json!(flat.id));

    // The previews, corrected on request.
    assert!(instrument.previews.flat().shading.is_some());
    assert_eq!(client.get("/api/v1/display/flat_field").await.json(), false);
    let written = client
        .put_json("/api/v1/display/flat_field", &json!(true))
        .await;
    assert!(written.status.is_success(), "{}", written.text());
    assert_eq!(client.get("/api/v1/display/flat_field").await.json(), true);
    assert!(instrument.previews.flat().enabled);

    // The tile scan's flat, estimated as a job, for its frames' objective and channel: none.
    let datasets = invoke(&client, "/api/v1/datasets/summary", json!({})).await;
    let dataset = &datasets["datasets"][0]["id"];
    let input = json!({ "dataset": dataset, "dark_level": 100.0 });
    let job = invoke(&client, "/api/v1/calibration/estimate_shading", input).await;
    let jobs = instrument.jobs.clone();
    let id = JobId(job.as_u64().unwrap());
    let ended = tokio::task::spawn_blocking(move || jobs.wait(id, Duration::from_secs(60)));
    let ended = ended.await.unwrap().expect("the job ended");
    assert_eq!(ended.state, JobState::Done, "{ended:?}");
    let estimated = instrument
        .shading
        .current((None, None, ShadingKind::Flat))
        .unwrap();
    assert_eq!(
        (estimated.method.as_str(), estimated.frames),
        ("estimate", 6)
    );
    assert!(
        instrument
            .shading
            .current((Some(ObjectiveId(1)), None, ShadingKind::Flat))
            .is_none()
    );
    client.stop().await;
}

#[tokio::test]
async fn a_flat_taken_at_the_edge_of_the_travel_stays_within_it() {
    let settings = tempfile::tempdir().unwrap();
    let mut config = config();
    config.server.settings = settings.path().to_owned();
    let (client, _instrument) = serve(config).await;
    // The stage starts at the corner of its travel, where the places a field away on +x and +y
    // are beyond it: the flat is taken a field in, and the stage comes back.
    let at = client.get("/api/v1/stage/position").await.json();
    assert_eq!((at["x"].as_f64(), at["y"].as_f64()), (Some(0.0), Some(0.0)));
    let flat = invoke(&client, "/api/v1/calibration/capture_flat", json!({})).await;
    assert_eq!(flat["frames"], 18);
    assert_eq!(client.get("/api/v1/stage/position").await.json(), at);
    client.stop().await;
}
