//! Live view through the acquisition engine, in real time, while the stage moves: the simulated
//! camera of `config/sim.toml` and the GRBL driver against its emulator. Every 250 ms it prints
//! the rate, the frames lost, and the latest frame's number and pose. At the end it snaps a frame
//! with a settled pose.
//!
//! ```text
//! cargo run --release -p siq-acquire --example acquire_live
//! ```
//!
//! It uses the `fast` profile and takes about 4 s. Smoke mode (`SIQ_SMOKE=1`) uses a 1024 × 768
//! sensor.

use std::error::Error;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use siq_acquire::Acquisition;
use siq_buffer::{Backend, Policy, PoolConfig};
use siq_config::model::{CameraConfig, SimProfile, StageConfig};
use siq_devices::{Camera, MoveOpts, Stage, Xyz};
use siq_grbl::{GrblOptions, GrblStage};
use siq_serial::{Framing, duplex};
use siq_sim::camera::{SimCamera, SimCameraSetup};
use siq_sim::grbl::{GrblEmulator, GrblErrors, GrblSettings};
use siq_sim::state::TrueState;
use siq_synth::WorldInfo;
use siq_types::{Clock, FrameMeta, SystemClock};

fn show(meta: &FrameMeta) -> String {
    let pose = meta.pose.map_or("none".into(), |p| {
        format!("({:.1}, {:.1}) µm", p.x.0, p.y.0)
    });
    format!("frame {:>3}: {pose}, {:?}", meta.seq.0, meta.pose_quality)
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn Error>> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config/sim.toml");
    let mut config = siq_config::load(&path)?;
    config.sim.profile = SimProfile::Fast;
    let (CameraConfig::Sim(mut camera), StageConfig::Sim(stage)) =
        (config.camera.clone(), config.stage.clone())
    else {
        return Err("config/sim.toml's camera and stage aren't simulated".into());
    };
    if std::env::var_os("SIQ_SMOKE").is_some() {
        (camera.width, camera.height) = (1024, 768);
    }
    let state = Arc::new(TrueState::new(1));
    let clock: Arc<dyn Clock> = Arc::new(SystemClock::new());

    let errors = GrblErrors::from(&config.sim.errors);
    let grbl = GrblEmulator::new(
        GrblSettings::default(),
        errors,
        config.sim.seed,
        state.clone(),
        clock.clone(),
    );
    let (host, device) = duplex(Framing::default(), "grbl");
    tokio::spawn(grbl.run(device));
    let xyz = GrblStage::connect(host, GrblOptions::from(&stage), clock.clone()).await?;

    let setup = SimCameraSetup::from_config(&camera, &config.objectives, &config.sim);
    let world = WorldInfo::from_config(&config.sim.world, config.sim.seed).build()?;
    let sim = Arc::new(SimCamera::new(
        setup,
        world.into(),
        state.clone(),
        clock.clone(),
    ));
    let pool = PoolConfig {
        name: "camera.pool".into(),
        slots: config.memory.pool_slots,
        slot_bytes: sim.settings().frame_bytes(),
        backend: Backend::Heap,
    };
    let acquisition = Acquisition::new(sim, &pool, clock)?;
    acquisition.follow_stage(xyz.status());
    let mut latest = acquisition.subscribe("printer", Policy::LatestOnly, |_| true);
    acquisition.start_live().await?;

    let printing = async {
        loop {
            tokio::time::sleep(Duration::from_millis(250)).await;
            let stats = acquisition.stats();
            let frame = latest
                .try_recv()
                .map_or(String::new(), |frame| show(frame.meta()));
            println!(
                "{:5.1} frames/s, {} lost: {frame}",
                stats.fps,
                stats.lost_no_slot + stats.lost_device
            );
        }
    };
    let moving = async {
        for (x, y) in [
            (-5_000.0, -2_000.0),
            (-1_000.0, -4_000.0),
            (-3_000.0, -3_000.0),
        ] {
            xyz.move_to(Xyz::from_um(x, y, 0.0), MoveOpts::default())
                .await?;
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        Ok::<_, Box<dyn Error>>(())
    };
    tokio::select! {
        moved = moving => moved?,
        () = printing => {}
    }
    let snap = acquisition.snap(false).await?;
    println!("snap:  {}", show(snap.meta()));
    let truth = state.pose();
    println!(
        "       the true stage: ({:.1}, {:.1}) µm",
        truth.x.0, truth.y.0
    );
    drop(snap);
    acquisition.stop().await?;
    Ok(())
}
