//! Records 20 frames of the simulated camera of `config/sim.toml` into a dataset, through the
//! acquisition engine and a lossless subscriber, and prints the throughput.
//!
//! ```text
//! cargo run --release -p siq-storage --example record_sim [-- --keep]
//! ```
//!
//! The dataset goes in a temporary data root, removed at the end unless `--keep` is given. At full
//! size that is 20 × 204 MB. Smoke mode (`SIQ_SMOKE=1`) uses a 1024 × 768 sensor.

use std::error::Error;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use siq_acquire::Acquisition;
use siq_buffer::{Backend, Policy, PoolConfig};
use siq_config::model::CameraConfig;
use siq_devices::Camera;
use siq_sim::camera::{SimCamera, SimCameraSetup};
use siq_sim::state::TrueState;
use siq_storage::{
    Catalog, Dataset, DatasetKind, DatasetReader, DatasetWriter, NewDataset, Snapshot,
    WriterOptions,
};
use siq_synth::WorldInfo;
use siq_types::{Clock, SystemClock};

const FRAMES: u32 = 20;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config/sim.toml");
    let config = siq_config::load(&path)?;
    let CameraConfig::Sim(mut camera) = config.camera.clone() else {
        return Err("config/sim.toml's camera isn't simulated".into());
    };
    if std::env::var_os("SIQ_SMOKE").is_some() {
        (camera.width, camera.height) = (1024, 768);
    }
    let clock: Arc<dyn Clock> = Arc::new(SystemClock::new());
    let setup = SimCameraSetup::from_config(&camera, &config.objectives, &config.sim);
    let world = WorldInfo::from_config(&config.sim.world, config.sim.seed).build()?;
    let state = Arc::new(TrueState::new(1));
    let sim = Arc::new(SimCamera::new(setup, world.into(), state, clock.clone()));
    let pool = PoolConfig {
        name: "camera.pool".into(),
        slots: config.memory.pool_slots,
        slot_bytes: sim.settings().frame_bytes(),
        backend: Backend::Heap,
    };
    let acquisition = Acquisition::new(sim, &pool, clock)?;

    let root = tempfile::Builder::new()
        .prefix("siq-record-sim")
        .tempdir()?;
    let mut new = NewDataset::new(DatasetKind::Recording, "record_sim");
    new.instrument = Some(Snapshot::of(&config)?);
    let dataset = Dataset::create(root.path(), new)?;
    let catalog = Arc::new(Catalog::scan(root.path())?);
    let mut options = WriterOptions::new((config.server.min_free_gb * 1e9) as u64);
    options.catalog = Some(catalog.clone());
    let frames = acquisition.subscribe("recorder", Policy::Lossless, |meta| meta.flags.save);
    let writer = DatasetWriter::start(&dataset, frames, options)?;

    acquisition.start_live().await?;
    let started = Instant::now();
    let mut captured = acquisition.capture_next(FRAMES, true);
    for _ in 0..FRAMES {
        // Each frame goes to the recorder too; this copy is let go at once.
        drop(captured.recv().await.ok_or("the capture ended early")?);
    }
    while writer.stats().frames < u64::from(FRAMES) && !writer.is_finished() {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let wall = started.elapsed();
    acquisition.stop().await?;
    let report = writer.finish();
    if let Some(error) = report.error {
        return Err(error.into());
    }

    let reader = DatasetReader::open(&dataset)?;
    let last = reader.frames().last().ok_or("nothing was recorded")?;
    let (width, height) = last.sidecar.meta.image_size();
    let stats = report.stats;
    let gb = stats.bytes as f64 / 1e9;
    println!(
        "{} frames of {width} × {height}: {gb:.2} GB in {:.2} s writing, {:.2} GB/s; {:.2} s from \
         the first capture to the last write",
        stats.frames,
        stats.busy.as_secs_f64(),
        gb / stats.busy.as_secs_f64(),
        wall.as_secs_f64(),
    );
    let mapped = last.map()?;
    let image = mapped.image_u16().ok_or("the frame isn't 16-bit")?;
    let mean = image.rows().flatten().map(|&p| f64::from(p)).sum::<f64>()
        / (image.width() * image.height()) as f64;
    println!(
        "read back {} frames; the last, {}, mapped: frame {}, mean {mean:.0}",
        reader.frames().len(),
        last.sidecar.key,
        mapped.meta().seq.0,
    );
    let listed = catalog.list();
    println!(
        "the catalog lists {} dataset with {} frames",
        listed.len(),
        listed[0].frames
    );
    if std::env::args().any(|arg| arg == "--keep") {
        println!("kept in {}", root.keep().display());
    }
    Ok(())
}
