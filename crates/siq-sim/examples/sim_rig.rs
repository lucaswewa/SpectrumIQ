//! The simulated rig of `config/sim.toml`, in real time, with the real drivers. The pieces:
//!
//! - the simulated camera;
//! - the GRBL driver against the GRBL emulator;
//! - the ASCII focus driver against the dialect emulator;
//! - the peripheral simulators.
//!
//! It sets up the first channel, homes, then moves the stage and the focus. After each move it
//! prints what the drivers report, where the stage and focus truly are, and the latest frame's
//! mean and sharpness. A frame shows the true pose: its texture changes as the stage moves, and
//! its sharpness falls as the focus leaves the specimen. This is M1's exit criterion.
//!
//! ```text
//! cargo run --release -p siq-sim --example sim_rig
//! ```
//!
//! It uses the `fast` profile and takes about 3 s. Smoke mode (`SIQ_SMOKE=1`) uses a 1024 × 768
//! sensor.

use std::error::Error;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use siq_ascii::{AsciiFocusDrive, Dialect};
use siq_buffer::{Backend, FillGuard, FramePool, PoolConfig};
use siq_config::model::{CameraConfig, FocusConfig, SimProfile, StageConfig};
use siq_devices::{
    Camera, CameraSettings, Changer, FocusDrive, FrameSink, Illumination, MoveOpts, Stage, Xyz,
};
use siq_grbl::{GrblOptions, GrblStage};
use siq_serial::{Framing, duplex};
use siq_sim::camera::{SimCamera, SimCameraSetup};
use siq_sim::dialect::DialectEmulator;
use siq_sim::grbl::{GrblEmulator, GrblErrors, GrblSettings};
use siq_sim::peripherals::{SimChanger, SimIllumination};
use siq_sim::state::TrueState;
use siq_synth::WorldInfo;
use siq_types::{Clock, FrameMeta, Micrometers, Millis, SystemClock};

/// The latest frame's number, mean and sharpness.
#[derive(Default)]
struct Latest(Mutex<(u64, f64, f64)>);

impl FrameSink for Latest {
    fn frame(&self, slot: FillGuard, meta: FrameMeta) {
        let lease = slot.publish(meta).expect("the frame fits");
        let image = lease.image::<u16>().expect("Mono16 words");
        // The mean, and the mean squared difference between neighbours: a gradient energy.
        let (mut sum, mut energy, mut count) = (0.0, 0.0, 0.0);
        for row in image.rows().step_by(4) {
            for pair in row.windows(2).step_by(4) {
                let (a, b) = (f64::from(pair[0]), f64::from(pair[1]));
                (sum, energy, count) = (sum + a, energy + (b - a).powi(2), count + 1.0);
            }
        }
        *self.0.lock().expect("the latest") = (meta.seq.0, sum / count, energy / count);
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn Error>> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config/sim.toml");
    let mut config = siq_config::load(&path)?;
    config.sim.profile = SimProfile::Fast;
    let (CameraConfig::Sim(mut camera), StageConfig::Sim(stage), FocusConfig::Sim(focus)) = (
        config.camera.clone(),
        config.stage.clone(),
        config.focus.clone(),
    ) else {
        return Err("config/sim.toml's camera, stage and focus aren't all simulated".into());
    };
    if std::env::var_os("SIQ_SMOKE").is_some() {
        (camera.width, camera.height) = (1024, 768);
    }
    let peripherals = &config.peripherals;
    let sources = peripherals
        .illumination
        .as_ref()
        .map(|i| i.sources.clone())
        .unwrap_or_default();
    let state = Arc::new(TrueState::new(sources.len()));
    let clock: Arc<dyn Clock> = Arc::new(SystemClock::new());

    // The stage and the focus: the real drivers, against their emulators.
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
    let dialect = Arc::new(match &focus.dialect {
        Some(path) => Dialect::load(path)?,
        None => Dialect::example(),
    });
    let (host, device) = duplex(dialect.framing(), "focus");
    let emulator = DialectEmulator::new(dialect.clone(), state.clone(), clock.clone());
    let in_focus = config.sim.focus_at;
    tokio::spawn(emulator.in_focus_at(Micrometers(in_focus)).run(device));
    let z = AsciiFocusDrive::connect(host, dialect, clock.clone()).await?;

    // The first channel's light and filter, and the first objective.
    let channel = &config.channels[0];
    let light = SimIllumination::new(sources.clone(), state.clone(), clock.clone());
    let lit = channel.illumination.as_ref();
    for (source, name) in sources.iter().enumerate() {
        let level = lit
            .filter(|lit| lit.source == *name)
            .map_or(0.0, |lit| lit.intensity);
        light.set_level(source, level).await?;
    }
    if let Some(wheel) = &peripherals.filter_wheel {
        let filters = SimChanger::filter_wheel(wheel.filters.clone(), state.clone(), clock.clone());
        let filter = wheel
            .filters
            .iter()
            .position(|f| Some(f) == channel.filter.as_ref());
        filters.select(filter.unwrap_or(0) as u8).await?;
    }
    if let Some(turret) = &peripherals.objective_turret {
        let turret = SimChanger::objective_turret(turret.positions, state.clone(), clock.clone());
        turret
            .select(config.objectives[0].turret_position.unwrap_or(0))
            .await?;
    }
    println!(
        "channel {}: {:?}, filter {:?}",
        channel.name,
        lit.map(|l| (&l.source, l.intensity)),
        state.filter()
    );

    // The camera, free-running.
    let setup = SimCameraSetup::from_config(&camera, &config.objectives, &config.sim);
    let world = WorldInfo::from_config(&config.sim.world, config.sim.seed).build()?;
    let sim = SimCamera::new(setup, world.into(), state.clone(), clock.clone());
    let settings = CameraSettings {
        exposure: Millis(10.0),
        ..sim.settings()
    };
    sim.configure(settings).await?;
    let pool_config = PoolConfig {
        name: "camera.pool".into(),
        slots: 4,
        slot_bytes: settings.frame_bytes(),
        backend: Backend::Heap,
    };
    let latest = Arc::new(Latest::default());
    sim.start(
        FramePool::new(&pool_config, clock.clone(), None)?,
        latest.clone(),
    )
    .await?;

    xyz.home().await?;
    z.home().await?;
    let opts = MoveOpts::default();
    // In focus, 30 µm out, then in focus elsewhere.
    for (x, y, out) in [
        (-2_000.0, -1_500.0, 0.0),
        (-2_000.0, -1_500.0, 30.0),
        (-2_600.0, -1_100.0, 0.0),
    ] {
        let at = xyz.move_to(Xyz::from_um(x, y, 0.0), opts).await?.position;
        let f = z.move_to(Micrometers(in_focus + out), opts).await?.position;
        // A new pose renders a draft first, then full detail once it holds.
        let seq = latest.0.lock().expect("the latest").0;
        while latest.0.lock().expect("the latest").0 < seq + 3 {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let (seq, mean, sharpness) = *latest.0.lock().expect("the latest");
        let truth = state.pose();
        println!(
            "stage ({:.1}, {:.1}) µm, truly ({:.1}, {:.1}); focus {:.2} µm, truly {:.2}: frame {seq}, mean {mean:.0} ADU, sharpness {sharpness:.0}",
            at.x.0, at.y.0, truth.x.0, truth.y.0, f.0, truth.focus.0
        );
    }
    sim.stop().await?;
    Ok(())
}
