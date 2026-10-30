//! A colour camera (ADR-0071): the simulated sensor of `config/sim.toml` given a filter array, end
//! to end over HTTP. The histogram per colour, the white balance fixed from the ROI, and a snap's
//! image keeping the filter array, its tiles in colour. REQ-ACQ-001, REQ-PRV-001, REQ-PRV-003,
//! REQ-VIS-001.

mod common;

use std::time::Duration;

use common::{config, finished, invoke};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use siq_config::model::CameraConfig;
use siq_server::Service;
use siq_storage::zarr::ZarrImage;
use siq_types::Cfa;
use teta_wot::http::axum::body::Body;
use teta_wot::http::axum::http::Request;
use teta_wot::testing::TestClient;
use tower::ServiceExt;

#[tokio::test]
async fn a_colour_camera_is_measured_balanced_and_snapped_in_colour() {
    let root = tempfile::tempdir().unwrap();
    let mut config = config();
    config.server.data_root = root.path().to_owned();
    config.server.min_free_gb = 0.0;
    let CameraConfig::Sim(camera) = &mut config.camera else {
        panic!("a simulated camera")
    };
    camera.cfa = Some(Cfa::Gbrg);
    let service = Service::build(config).await.unwrap();
    let router = service.router();
    let (server, instrument) = service.into_parts();
    let client = TestClient::start(server).await.unwrap();

    // Short enough that no colour clips.
    let exposure = client
        .put_json("/api/v1/camera/exposure_ms", &json!(2.0))
        .await;
    assert!(exposure.status.is_success(), "{}", exposure.text());
    let started = client.post_json("/api/v1/camera/start_live", None).await;
    assert_eq!(finished(&client, started).await["status"], "completed");
    // The ROI's statistics, per colour: under the warm lamp, blue below red below green.
    let mut histogram = Value::Null;
    for _ in 0..400 {
        histogram = client.get("/api/v1/display/histogram").await.json();
        if histogram["colours"]
            .as_array()
            .is_some_and(|c| c.len() == 3)
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let high = |c: usize| {
        histogram["colours"][c]["window"][1]
            .as_f64()
            .expect("a window")
    };
    assert!(high(2) < high(0) && high(0) < high(1), "{histogram}");
    assert_eq!(histogram["window"], histogram["colours"][1]["window"]);

    // White from the ROI: red and blue raised to green.
    let settings = invoke(&client, "/api/v1/display/white_from_roi", json!({})).await;
    let balance = &settings["white_balance"];
    let (red, blue) = (
        balance["red"].as_f64().unwrap(),
        balance["blue"].as_f64().unwrap(),
    );
    assert_eq!(balance["kind"], "fixed");
    assert!(1.0 < red && red < blue, "{settings}");
    assert_eq!(client.get("/api/v1/display/lut").await.json(), settings);
    let stopped = client.post_json("/api/v1/camera/stop_live", None).await;
    assert_eq!(finished(&client, stopped).await["status"], "completed");

    // A snap's image keeps the filter array, and its tiles are in colour.
    let snap = invoke(&client, "/api/v1/datasets/snap_to_dataset", json!({})).await;
    let id = snap["dataset"]["id"].as_str().unwrap().parse().unwrap();
    let summary = instrument.catalog.get(id).unwrap();
    let image = ZarrImage::open(&summary.path.join("zarr/snap.ome.zarr")).unwrap();
    assert_eq!(image.info().cfa, Some(Cfa::Gbrg));
    let path = format!("/api/siq/data/{id}/snap/0/0/0_files/8/0_0.jpeg");
    let request = Request::builder().uri(path).body(Body::empty()).unwrap();
    let response = router.oneshot(request).await.unwrap();
    assert!(response.status().is_success());
    let tile = response.into_body().collect().await.unwrap().to_bytes();
    // Its frame header's components: three.
    let sof = tile.windows(2).position(|w| w == [0xFF, 0xC0]).unwrap();
    assert_eq!(tile[sof + 9], 3, "a colour JPEG");
    client.stop().await;
}
