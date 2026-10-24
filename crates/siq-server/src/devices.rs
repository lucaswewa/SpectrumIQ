//! The devices, chosen from the configuration (ADR-0034): for each, the simulator or the real one.
//!
//! - **The camera:** `kind = "sim"` is `siq-sim`'s camera, looking at the configured world. The
//!   Euresys camera comes with P10.1.
//! - **The stage:** the GRBL driver, against the GRBL emulator over an in-memory link for
//!   `"sim"`, or on a serial port for `"grbl"`.
//! - **The focus drive:** the ASCII driver, against the dialect emulator for `"sim"`, or on a
//!   serial port for `"ascii"`, in the configured dialect.
//!
//! The simulators share one `TrueState`: the emulators move it, and the camera sees it.

use std::sync::Arc;

use anyhow::{Context, bail};
use siq_ascii::{AsciiFocusDrive, Dialect};
use siq_buffer::Backend;
use siq_config::model::{CameraConfig, FocusConfig, InstrumentConfig, StageConfig};
use siq_devices::{Camera, FocusDrive, Stage};
use siq_grbl::{GrblOptions, GrblStage};
use siq_serial::{Framing, PortSettings, duplex};
use siq_sim::camera::{SimCamera, SimCameraSetup};
use siq_sim::dialect::DialectEmulator;
use siq_sim::grbl::{GrblEmulator, GrblErrors, GrblSettings};
use siq_sim::state::TrueState;
use siq_synth::WorldInfo;
use siq_types::Clock;

/// The instrument's devices.
pub struct Devices {
    /// The camera.
    pub camera: Arc<dyn Camera>,
    /// Where the frame pool's memory comes from: locked in RAM for a real camera, the heap for
    /// the simulator.
    pub backend: Backend,
    /// The XYZ stage.
    pub stage: Arc<dyn Stage>,
    /// The focus drive.
    pub focus: Arc<dyn FocusDrive>,
    /// The simulated instrument's true state.
    pub state: Arc<TrueState>,
}

fn port(baud: u32) -> PortSettings {
    PortSettings {
        baud,
        ..PortSettings::default()
    }
}

/// Opens every device of `config`, on `clock`, connecting the drivers.
pub async fn open(config: &InstrumentConfig, clock: Arc<dyn Clock>) -> anyhow::Result<Devices> {
    let sources = config
        .peripherals
        .illumination
        .as_ref()
        .map_or(1, |light| light.sources.len());
    let state = Arc::new(TrueState::new(sources));
    let (camera, backend): (Arc<dyn Camera>, _) = match &config.camera {
        CameraConfig::Sim(camera) => {
            let setup = SimCameraSetup::from_config(camera, &config.objectives, &config.sim);
            let world = WorldInfo::from_config(&config.sim.world, config.sim.seed).build()?;
            let camera = SimCamera::new(setup, world.into(), state.clone(), clock.clone());
            (Arc::new(camera), Backend::Heap)
        }
        CameraConfig::Euresys(_) => {
            bail!("the Euresys camera comes with P10.1: use [camera] kind = \"sim\"")
        }
    };
    let stage: Arc<dyn Stage> = match &config.stage {
        StageConfig::Sim(stage) => {
            let errors = GrblErrors::from(&config.sim.errors);
            let seed = config.sim.seed;
            let emulator = GrblEmulator::new(
                GrblSettings::default(),
                errors,
                seed,
                state.clone(),
                clock.clone(),
            );
            let (host, device) = duplex(Framing::default(), "grbl");
            tokio::spawn(emulator.run(device));
            Arc::new(GrblStage::connect(host, GrblOptions::from(stage), clock.clone()).await?)
        }
        StageConfig::Grbl(stage) => {
            let link = siq_serial::open(&stage.port, &port(stage.baud), Framing::default())?;
            let connected = GrblStage::connect(link, GrblOptions::from(stage), clock.clone()).await;
            Arc::new(connected.with_context(|| format!("the stage on {}", stage.port))?)
        }
    };
    let focus: Arc<dyn FocusDrive> = match &config.focus {
        FocusConfig::Sim(focus) => {
            let dialect = Arc::new(match &focus.dialect {
                Some(path) => Dialect::load(path)?,
                None => Dialect::example(),
            });
            let (host, device) = duplex(dialect.framing(), "focus");
            tokio::spawn(
                DialectEmulator::new(dialect.clone(), state.clone(), clock.clone()).run(device),
            );
            Arc::new(AsciiFocusDrive::connect(host, dialect, clock).await?)
        }
        FocusConfig::Ascii(focus) => {
            let dialect = Arc::new(Dialect::load(&focus.dialect)?);
            let link = siq_serial::open(&focus.port, &port(focus.baud), dialect.framing())?;
            let connected = AsciiFocusDrive::connect(link, dialect, clock).await;
            Arc::new(connected.with_context(|| format!("the focus drive on {}", focus.port))?)
        }
    };
    Ok(Devices {
        camera,
        backend,
        stage,
        focus,
        state,
    })
}
