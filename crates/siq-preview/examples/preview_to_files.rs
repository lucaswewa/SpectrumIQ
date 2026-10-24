//! Previews 3 s of simulated live view, and writes the last overview and ROI JPEGs. The camera is
//! `config/sim.toml`'s, and the ROI is a 2:1 view of the middle. It prints how many previews each
//! stream made, their mean time of work and from mid-exposure, and where the files are.
//!
//! ```text
//! cargo run --release -p siq-preview --example preview_to_files [-- <folder>]
//! ```
//!
//! The folder defaults to `target/preview_to_files`. It uses the full sensor, about 100 MP. Smoke
//! mode (`SIQ_SMOKE=1`) uses the `fast` profile with a 1024 × 768 sensor.

use std::error::Error;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use siq_acquire::Acquisition;
use siq_buffer::{Backend, PoolConfig};
use siq_config::model::{CameraConfig, SimProfile};
use siq_devices::Camera;
use siq_preview::{
    FrameStats, PreviewConfig, PreviewSink, Previews, RoiSettings, StatsSink, StreamKind, Zoom,
};
use siq_sim::camera::{SimCamera, SimCameraSetup};
use siq_sim::state::TrueState;
use siq_synth::WorldInfo;
use siq_telemetry::{MetricValue, Registry};
use siq_types::{Clock, FrameMeta, SystemClock};

/// Keeps each stream's last JPEG, the number made, and the last statistics.
#[derive(Default)]
struct Keep {
    last: Mutex<[(usize, Vec<u8>); 2]>,
    stats: Mutex<Option<FrameStats>>,
}

impl PreviewSink for Keep {
    fn push_jpeg(&self, stream: StreamKind, jpeg: Vec<u8>, _: &FrameMeta) {
        let index = usize::from(stream == StreamKind::Roi);
        let mut last = self.last.lock().expect("the previews");
        last[index] = (last[index].0 + 1, jpeg);
    }
}

impl StatsSink for Keep {
    fn push_stats(&self, stats: &FrameStats) {
        *self.stats.lock().expect("the statistics") = Some(stats.clone());
    }
}

fn mean_ms(registry: &Registry, name: &str) -> f64 {
    match registry.snapshot().get(name) {
        Some(MetricValue::Histogram { count, sum, .. }) if *count > 0 => sum / *count as f64,
        _ => f64::NAN,
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn Error>> {
    let folder = std::env::args()
        .nth(1)
        .map_or_else(|| PathBuf::from("target/preview_to_files"), PathBuf::from);
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config/sim.toml");
    let mut config = siq_config::load(&path)?;
    let CameraConfig::Sim(mut camera) = config.camera.clone() else {
        return Err("config/sim.toml's camera isn't simulated".into());
    };
    if std::env::var_os("SIQ_SMOKE").is_some() {
        config.sim.profile = SimProfile::Fast;
        (camera.width, camera.height) = (1024, 768);
    }
    let clock: Arc<dyn Clock> = Arc::new(SystemClock::new());
    let setup = SimCameraSetup::from_config(&camera, &config.objectives, &config.sim);
    let world = WorldInfo::from_config(&config.sim.world, config.sim.seed).build()?;
    let sim = Arc::new(SimCamera::new(
        setup,
        world.into(),
        Arc::new(TrueState::new(1)),
        clock.clone(),
    ));
    let settings = sim.settings();
    let (width, height) = settings.image_size();
    println!(
        "{width} × {height} {:?}, live for 3 s",
        settings.pixel_format
    );
    let pool = PoolConfig {
        name: "camera.pool".into(),
        slots: config.memory.pool_slots,
        slot_bytes: settings.frame_bytes(),
        backend: Backend::Heap,
    };
    let acquisition = Acquisition::new(sim, &pool, clock.clone())?;
    let (keep, registry) = (Arc::new(Keep::default()), Registry::new());
    let previews = Previews::start(
        &acquisition,
        &PreviewConfig::default(),
        keep.clone(),
        keep.clone(),
        clock,
        Some(&registry),
    )?;
    previews.set_roi(RoiSettings {
        region: None,
        zoom: Zoom::Half,
    });
    acquisition.start_live().await?;
    tokio::time::sleep(Duration::from_secs(3)).await;
    acquisition.stop().await?;
    drop(previews);

    std::fs::create_dir_all(&folder)?;
    let last = keep.last.lock().expect("the previews");
    for ((made, jpeg), name) in last.iter().zip(["overview", "roi"]) {
        let file = folder.join(format!("{name}.jpg"));
        std::fs::write(&file, jpeg)?;
        let mean = |what: &str| mean_ms(&registry, &format!("preview.{name}.{what}"));
        let (latency, work) = (mean("latency_ms"), mean("work_ms"));
        println!(
            "{name:<8} {made:>3} JPEGs, {work:5.1} ms of work and {latency:5.1} ms from mid-exposure on average: {}",
            file.display()
        );
    }
    if let Some(stats) = keep.stats.lock().expect("the statistics").as_ref() {
        let (low, high) = stats.window;
        println!(
            "stats    frame {}: window {low}–{high}, focus {:.0}",
            stats.meta.seq.0, stats.focus
        );
    }
    println!("{:?}", acquisition.stats());
    Ok(())
}
