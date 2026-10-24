//! Runs the simulated camera of `config/sim.toml` live, in real time, and moves the stage to a new
//! pose every second. For each pose it prints the frames, their rate, the frames lost, and the
//! mean intensity.
//!
//! ```text
//! cargo run --release -p siq-sim --example sim_live
//! ```
//!
//! It uses the `fast` profile (25 MP) and runs 5 s. Smoke mode (`SIQ_SMOKE=1`) runs 1 s with a
//! 512 × 384 sensor.

use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use siq_buffer::{Backend, FillGuard, FramePool, PoolConfig};
use siq_config::model::{CameraConfig, SimProfile};
use siq_devices::{Camera, CameraSettings, FrameSink};
use siq_sim::camera::{SimCamera, SimCameraSetup};
use siq_sim::state::TrueState;
use siq_synth::WorldInfo;
use siq_types::{FrameMeta, Micrometers, Millis, SystemClock};

/// Per pose: the frames' mid-exposure times in ms, and the sum of their means.
#[derive(Default)]
struct Tally {
    pose: AtomicUsize,
    poses: Mutex<Vec<(Vec<f64>, f64)>>,
}

impl FrameSink for Tally {
    fn frame(&self, slot: FillGuard, meta: FrameMeta) {
        let lease = slot.publish(meta).expect("the frame fits");
        let image = lease.image::<u16>().expect("Mono16 words");
        let values: Vec<f64> = image
            .rows()
            .step_by(16)
            .flat_map(|row| row.iter().step_by(16))
            .map(|&v| f64::from(v))
            .collect();
        let mean = values.iter().sum::<f64>() / values.len().max(1) as f64;
        let mut poses = self.poses.lock().expect("the tally");
        let pose = self.pose.load(Ordering::Relaxed);
        let len = poses.len().max(pose + 1);
        poses.resize_with(len, Default::default);
        poses[pose].0.push(meta.host_ts_mid_exposure.0 as f64 / 1e6);
        poses[pose].1 += mean;
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let smoke = std::env::var_os("SIQ_SMOKE").is_some();
    let mut config =
        siq_config::load(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config/sim.toml"))?;
    config.sim.profile = SimProfile::Fast;
    let CameraConfig::Sim(mut camera) = config.camera.clone() else {
        return Err("config/sim.toml's camera isn't simulated".into());
    };
    if smoke {
        (camera.width, camera.height) = (1024, 768);
    }
    let setup = SimCameraSetup::from_config(&camera, &config.objectives, &config.sim);
    let world: Arc<dyn siq_synth::World> =
        WorldInfo::from_config(&config.sim.world, config.sim.seed)
            .build()?
            .into();
    let state = Arc::new(TrueState::new(1));
    let clock = Arc::new(SystemClock::new());
    let sim = SimCamera::new(setup, world, state.clone(), clock.clone());
    let settings = CameraSettings {
        exposure: Millis(10.0),
        ..sim.settings()
    };
    sim.configure(settings).await?;
    let (width, height) = settings.image_size();
    println!(
        "{width} × {height} {:?} at up to {:.1} frames/s",
        settings.pixel_format,
        sim.capabilities().frame_rate.max_rate(&settings).0
    );

    let pool_config = PoolConfig {
        name: "camera.pool".into(),
        slots: 4,
        slot_bytes: settings.frame_bytes(),
        backend: Backend::Heap,
    };
    let pool = FramePool::new(&pool_config, clock, None)?;
    let tally = Arc::new(Tally::default());
    sim.start(pool, tally.clone()).await?;
    let poses = if smoke { 1 } else { 5 };
    for pose in 0..poses {
        tally.pose.store(pose, Ordering::Relaxed);
        let x = Micrometers(800.0 * pose as f64);
        state.set_stage(x, Micrometers(-300.0 * pose as f64), Micrometers::ZERO);
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    sim.stop().await?;

    let stats = sim.stats();
    for (pose, (times, sum)) in tally.poses.lock().expect("the tally").iter().enumerate() {
        let rate = match times.len() {
            0 | 1 => 0.0,
            n => (n - 1) as f64 / (times[n - 1] - times[0]) * 1e3,
        };
        let mean = sum / times.len().max(1) as f64;
        println!(
            "pose {pose}: {:>3} frames, {rate:>5.1} frames/s, mean {mean:>7.1} ADU",
            times.len()
        );
    }
    println!(
        "delivered {}, lost for want of a slot {}",
        stats.delivered, stats.lost_no_slot
    );
    Ok(())
}
