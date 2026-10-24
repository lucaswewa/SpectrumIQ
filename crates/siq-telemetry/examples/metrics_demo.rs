//! Logs to the console and a daily file, updates metrics from several threads as a camera and
//! its consumers would, and prints the registry's JSON snapshot.
//!
//! ```text
//! cargo run -p siq-telemetry --example metrics_demo
//! ```
//!
//! Smoke mode (`SIQ_SMOKE=1`) simulates fewer frames. The example exits with an error if a count
//! is wrong.

use std::process::ExitCode;
use std::sync::Arc;
use std::thread;

use siq_telemetry::{LogConfig, MetricValue, Registry};

fn main() -> ExitCode {
    let folder = std::env::temp_dir().join("siq-metrics-demo");
    let config = LogConfig {
        directory: Some(folder.clone()),
        ..LogConfig::default()
    };
    let _guard = match siq_telemetry::init(&config) {
        Ok(guard) => guard,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::FAILURE;
        }
    };

    let frames: u64 = if std::env::var_os("SIQ_SMOKE").is_some() {
        90
    } else {
        9_000
    };
    let registry = Arc::new(Registry::new());
    tracing::info!(frames, folder = %folder.display(), "simulating a camera and two consumers");
    let camera = {
        let registry = Arc::clone(&registry);
        thread::spawn(move || {
            let acquired = registry
                .counter("camera.frames_acquired")
                .expect("a valid name");
            let lost = registry
                .counter("camera.frames_lost")
                .expect("a valid name");
            for frame in 0..frames {
                acquired.increment();
                if frame % 1000 == 999 {
                    lost.increment();
                    tracing::warn!(frame, "a frame was lost");
                }
            }
        })
    };
    let preview = {
        let registry = Arc::clone(&registry);
        thread::spawn(move || {
            let bounds = [10.0, 20.0, 40.0, 80.0];
            let latency = registry
                .histogram("preview.latency_ms", &bounds)
                .expect("valid bounds");
            let free = registry.gauge("pool.free_slots").expect("a valid name");
            for frame in 0..frames {
                latency.record(12.0 + (frame % 50) as f64);
                free.set((12 - frame % 4) as f64);
            }
        })
    };
    camera.join().expect("the camera thread finishes");
    preview.join().expect("the preview thread finishes");

    let snapshot = registry.snapshot();
    match serde_json::to_string_pretty(&snapshot) {
        Ok(json) => println!("{json}"),
        Err(error) => eprintln!("{error}"),
    }
    let acquired = snapshot.get("camera.frames_acquired");
    if acquired == Some(&MetricValue::Counter { value: frames }) {
        ExitCode::SUCCESS
    } else {
        eprintln!("expected {frames} frames, got {acquired:?}");
        ExitCode::FAILURE
    }
}
