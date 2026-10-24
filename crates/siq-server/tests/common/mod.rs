//! What the service's test binaries share: the simulated instrument of `config/sim.toml` on a
//! small sensor, an in-process client, and waiting for an invocation.

#![allow(dead_code, reason = "each binary uses some of it")]

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use siq_config::model::{CameraConfig, InstrumentConfig};
use siq_server::{Instrument, Service};
use teta_wot::http::axum::http::StatusCode;
use teta_wot::testing::{TestClient, TestResponse};

pub fn config() -> InstrumentConfig {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config/sim.toml");
    let mut config = siq_config::load(&path).unwrap();
    let CameraConfig::Sim(camera) = &mut config.camera else {
        panic!("a simulated camera")
    };
    (camera.width, camera.height) = (256, 192);
    config.memory.pool_slots = 6;
    config
}

/// A client of the service of `config`, and its instrument.
pub async fn serve(config: InstrumentConfig) -> (TestClient, Arc<Instrument>) {
    let (server, instrument) = Service::build(config).await.unwrap().into_parts();
    (TestClient::start(server).await.unwrap(), instrument)
}

pub async fn client() -> TestClient {
    serve(config()).await.0
}

/// Waits for an invocation to end, and gives back its last state.
pub async fn finished(client: &TestClient, invocation: TestResponse) -> Value {
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
    for _ in 0..1000 {
        let state = client.get(&href).await.json();
        if !matches!(state["status"].as_str(), Some("pending" | "running")) {
            return state;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("{href} didn't finish");
}

/// Invokes `action` with `input`, and waits for it to complete: its output.
pub async fn invoke(client: &TestClient, action: &str, input: Value) -> Value {
    let invocation = client.post_json(action, Some(&input)).await;
    let done = finished(client, invocation).await;
    assert_eq!(done["status"], "completed", "{action}: {done}");
    done["output"].clone()
}
