//! The service with the simulated instrument of `config/sim.toml`, on a small sensor, through
//! teta-wot's in-process client: the Things and their descriptions, a move observed over HTTP, the
//! MJPEG streams, a snap, the global lock, and SpectrumIQ's own routes.

mod common;

use std::time::Duration;

use common::{client, config, finished};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use siq_server::Service;
use teta_wot::http::axum::body::Body;
use teta_wot::http::axum::http::{Request, StatusCode};
use tower::ServiceExt;

/// Each Thing's affordances, by kind, for a snapshot.
fn affordances(td: &Value) -> Value {
    let names = |kind: &str| {
        let mut names: Vec<_> = td[kind]
            .as_object()
            .map(|o| o.keys().cloned().collect())
            .unwrap_or_default();
        names.sort();
        names
    };
    json!({"title": td["title"], "properties": names("properties"), "actions": names("actions"), "events": names("events")})
}

#[tokio::test]
async fn the_things_are_listed_and_described() {
    let client = client().await;
    let things = client.get("/api/v1/things/").await.json();
    let mut names: Vec<_> = things.as_object().unwrap().keys().cloned().collect();
    names.sort();
    let expected = [
        "calibration",
        "camera",
        "datasets",
        "display",
        "filter_wheel",
        "focus",
        "illumination",
        "imaging",
        "jobs",
        "modality",
        "objective",
        "stage",
    ];
    assert_eq!(names, expected);
    let mut descriptions = serde_json::Map::new();
    for name in names {
        let td = client.get(&format!("/api/v1/{name}/")).await.json();
        descriptions.insert(name, affordances(&td));
    }
    insta::assert_json_snapshot!("things", descriptions);
    let sensor = client.get("/api/v1/camera/sensor").await.json();
    assert_eq!(
        (sensor["width"].as_u64(), sensor["height"].as_u64()),
        (Some(256), Some(192))
    );
    client.stop().await;
}

#[tokio::test]
async fn a_value_the_camera_refuses_is_a_422_with_its_reason() {
    let client = client().await;
    // The simulated camera aligns regions (to 8 × 8 pixels on this small sensor): an odd first
    // row is refused.
    let misaligned = json!({"offset_x": 8, "offset_y": 3, "width": 64, "height": 64});
    let refused = client.put_json("/api/v1/camera/region", &misaligned).await;
    assert_eq!(
        refused.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        refused.text()
    );
    let issue = &refused.json()["detail"][0];
    assert_eq!(issue["type"], "value_error", "{issue}");
    assert_eq!(issue["loc"], json!(["body"]));
    assert!(issue["msg"].as_str().unwrap().contains("align"), "{issue}");
    assert_eq!(issue["input"], misaligned);
    let whole = json!({"offset_x": 0, "offset_y": 0, "width": 256, "height": 192});
    assert_eq!(client.get("/api/v1/camera/region").await.json(), whole);

    // A value the camera takes is still written.
    let aligned = json!({"offset_x": 8, "offset_y": 8, "width": 64, "height": 64});
    let written = client.put_json("/api/v1/camera/region", &aligned).await;
    assert!(written.status.is_success(), "{}", written.text());
    assert_eq!(client.get("/api/v1/camera/region").await.json(), aligned);
    client.stop().await;
}

#[tokio::test]
async fn a_stage_move_over_http_is_observed_until_it_arrives() {
    let client = client().await;
    let mut positions = client.events("/api/v1/stage/position").await;
    let moving = client
        .post_json(
            "/api/v1/stage/move_to",
            Some(&json!({"x": -2000.0, "y": -1000.0, "z": 0.0})),
        )
        .await;
    let done = finished(&client, moving).await;
    assert_eq!(done["status"], "completed", "{done}");
    assert_eq!(done["output"]["state"], "idle");
    // The position property was observed on its way there.
    let mut seen = Vec::new();
    while let Some(position) = positions.next_within(Duration::from_millis(500)).await {
        seen.push(position);
        if seen.last().unwrap()["x"] == -2000.0 {
            break;
        }
    }
    assert!(seen.len() >= 2, "{seen:?}");
    assert_eq!(
        seen.last().unwrap(),
        &json!({"x": -2000.0, "y": -1000.0, "z": 0.0})
    );
    assert_eq!(client.get("/api/v1/stage/state").await.json(), "idle");
    client.stop().await;
}

/// The next part of a multipart MJPEG body: its JPEG.
async fn next_jpeg(body: &mut Body) -> Vec<u8> {
    let chunk = tokio::time::timeout(Duration::from_secs(10), body.frame()).await;
    let chunk = chunk
        .expect("a part in time")
        .expect("the stream is open")
        .unwrap();
    let data = chunk.into_data().expect("data");
    let start = data
        .windows(2)
        .position(|w| w == [0xFF, 0xD8])
        .expect("a JPEG in the part");
    data[start..].to_vec()
}

#[tokio::test]
async fn live_view_streams_mjpeg_and_a_snap_comes_back_as_a_jpeg() {
    let client = client().await;
    let started = finished(
        &client,
        client.post_json("/api/v1/camera/start_live", None).await,
    )
    .await;
    assert_eq!(started["status"], "completed", "{started}");
    let overview = client.stream("/api/v1/camera/overview").await;
    let mut body = overview.into_body();
    let (first, second) = (next_jpeg(&mut body).await, next_jpeg(&mut body).await);
    assert!(first.len() > 100 && second.len() > 100);
    drop(body);
    assert_eq!(
        client.get("/api/v1/camera/stats").await.json()["mode"],
        "live"
    );

    let snap = finished(
        &client,
        client
            .post_json("/api/v1/camera/snap", Some(&json!({})))
            .await,
    )
    .await;
    assert_eq!(snap["status"], "completed", "{snap}");
    let output = snap["output"]["href"]
        .as_str()
        .unwrap()
        .replace("http://testserver", "");
    let jpeg = client.get(&output).await;
    assert_eq!(jpeg.header("content-type"), Some("image/jpeg"));
    let histogram = client.get("/api/v1/display/histogram").await.json();
    assert_eq!(
        histogram["bins"].as_array().map(Vec::len),
        Some(256),
        "{histogram}"
    );
    client.stop().await;
}

#[tokio::test]
async fn the_global_lock_refuses_a_second_hardware_action_but_not_halt() {
    let client = client().await;
    // A long move holds the lock: 90 mm at 50 mm/s.
    let far = client
        .post_json(
            "/api/v1/stage/move_to",
            Some(&json!({"x": -90000.0, "y": 0.0, "z": 0.0})),
        )
        .await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    let focus = client
        .post_json("/api/v1/focus/move_to", Some(&json!({"z": 100.0})))
        .await;
    let refused = finished(&client, focus).await;
    assert_eq!(refused["status"], "error", "{refused}");
    assert!(refused.to_string().contains("GlobalLockBusy"), "{refused}");
    // Halting doesn't wait for the lock, and stops the move.
    let halted = finished(&client, client.post_json("/api/v1/stage/halt", None).await).await;
    assert_eq!(halted["status"], "completed", "{halted}");
    let moved = finished(&client, far).await;
    assert_eq!(moved["status"], "error", "{moved}");
    let x = client.get("/api/v1/stage/position").await.json()["x"]
        .as_f64()
        .unwrap();
    assert!((-90000.0..0.0).contains(&x), "{x}");
    client.stop().await;
}

#[tokio::test]
async fn siq_routes_come_first_and_the_things_after() {
    let service = Service::build(config()).await.unwrap();
    let router = service.router();
    let get = |path: &str| Request::get(path).body(Body::empty()).unwrap();
    let health = router
        .clone()
        .oneshot(get("/api/siq/system/health"))
        .await
        .unwrap();
    assert_eq!(health.status(), StatusCode::OK);
    let body: Value =
        serde_json::from_slice(&health.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["status"], "ok");
    let things = router.oneshot(get("/api/v1/things/")).await.unwrap();
    assert_eq!(things.status(), StatusCode::OK);
}

#[tokio::test]
async fn the_openapi_document_has_every_affordance() {
    let document = siq_server::openapi().await.unwrap();
    let mut paths: Vec<_> = document["paths"]
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect();
    paths.sort();
    // SpectrumIQ's routes and teta-wot's never overlap (ADR-0003).
    assert!(
        paths.iter().all(|path| !path.starts_with("/api/siq/")),
        "{paths:?}"
    );
    insta::assert_json_snapshot!("openapi_paths", paths);
    assert_eq!(document["info"]["title"], "SpectrumIQ");
}

#[tokio::test]
async fn the_web_apps_openapi_document_is_current() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../web/src/api/generated/openapi.json");
    let committed: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    assert!(
        committed == siq_server::openapi().await.unwrap(),
        "web/src/api/generated/openapi.json is out of date: in web/, run          `npm run api:openapi` then `npm run api:types`"
    );
}

#[tokio::test]
async fn serving_answers_over_tcp_and_shutting_down_stops_live_view_first() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let service = Service::build(config()).await.unwrap();
    let instrument = service.instrument().clone();
    instrument.acquisition.start_live().await.unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let serving = tokio::spawn(service.serve(listener, async {
        let _ = stopped.await;
    }));
    let mut connection = tokio::net::TcpStream::connect(address).await.unwrap();
    let request = "GET /api/siq/system/health HTTP/1.1\r\nHost: test\r\nConnection: close\r\n\r\n";
    connection.write_all(request.as_bytes()).await.unwrap();
    let mut answer = String::new();
    connection.read_to_string(&mut answer).await.unwrap();
    assert!(
        answer.starts_with("HTTP/1.1 200") && answer.contains("\"status\":\"ok\""),
        "{answer}"
    );

    stop.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(10), serving)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(
        instrument.acquisition.stats().mode,
        siq_acquire::Mode::Stopped
    );
}
