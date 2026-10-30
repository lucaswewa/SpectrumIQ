//! The devices, chosen from the configuration (ADR-0034): for each, the simulator or the real one.
//!
//! - **The camera:** `kind = "sim"` is `siq-sim`'s camera, looking at the configured world. The
//!   Euresys camera comes with P10.1.
//! - **The stage:** the GRBL driver, against the GRBL emulator over an in-memory link for
//!   `"sim"`, or on a serial port for `"grbl"`.
//! - **The focus drive:** the ASCII driver, against the dialect emulator for `"sim"`, or on a
//!   serial port for `"ascii"`, in the configured dialect.
//!
//! - **The peripherals,** each one only if `[peripherals]` names it: simulated, until their
//!   models are chosen (P10.5).
//!
//! The simulators share one `TrueState`: the emulators and the peripherals move it, and the camera
//! sees it.

use std::sync::Arc;

use anyhow::Context;
use siq_ascii::{AsciiFocusDrive, Dialect};
use siq_buffer::Backend;
use siq_config::model::{
    CameraConfig, FocusConfig, InstrumentConfig, PeripheralsConfig, StageConfig,
};
use siq_devices::{
    Camera, FilterWheel, FocusDrive, Illumination, ModalityUnit, ObjectiveTurret, PatternSet, Stage,
};
use siq_grbl::{GrblOptions, GrblStage};
use siq_serial::{Framing, PortSettings, duplex};
use siq_sim::camera::{SimCamera, SimCameraSetup};
use siq_sim::dialect::DialectEmulator;
use siq_sim::grbl::{GrblEmulator, GrblErrors, GrblSettings};
use siq_sim::peripherals::{SimChanger, SimIllumination, SimModalityUnit};
use siq_sim::state::TrueState;
use siq_synth::WorldInfo;
use siq_types::{Clock, Micrometers, ModalityMode};

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
    /// The peripherals that are fitted.
    pub peripherals: Peripherals,
    /// The simulated instrument's true state.
    pub state: Arc<TrueState>,
}

/// The peripherals, each `None` if it isn't fitted.
#[derive(Clone, Default)]
pub struct Peripherals {
    /// The light sources.
    pub illumination: Option<Arc<dyn Illumination>>,
    /// The filter wheel.
    pub filter_wheel: Option<Arc<dyn FilterWheel>>,
    /// The objective turret.
    pub turret: Option<Arc<dyn ObjectiveTurret>>,
    /// The modality unit.
    pub modality: Option<Arc<dyn ModalityUnit>>,
}

/// The structured-illumination patterns of the simulated modality unit: 3 orientations of 3 phases.
const PATTERNS: PatternSet = PatternSet {
    orientations: 3,
    phases: 3,
};

/// The peripherals of `config`, simulated, moving `state`.
fn peripherals(
    config: &PeripheralsConfig,
    state: &Arc<TrueState>,
    clock: &Arc<dyn Clock>,
) -> Peripherals {
    let (state, clock) = (state.clone(), clock.clone());
    let illumination = config
        .illumination
        .as_ref()
        .map(|light| -> Arc<dyn Illumination> {
            Arc::new(SimIllumination::new(
                light.sources.clone(),
                state.clone(),
                clock.clone(),
            ))
        });
    let filter_wheel = config
        .filter_wheel
        .as_ref()
        .map(|wheel| -> Arc<dyn FilterWheel> {
            Arc::new(SimChanger::filter_wheel(
                wheel.filters.clone(),
                state.clone(),
                clock.clone(),
            ))
        });
    let turret = config
        .objective_turret
        .as_ref()
        .map(|turret| -> Arc<dyn ObjectiveTurret> {
            Arc::new(SimChanger::objective_turret(
                turret.positions,
                state.clone(),
                clock.clone(),
            ))
        });
    let modality = config
        .modality
        .as_ref()
        .map(|unit| -> Arc<dyn ModalityUnit> {
            let structured = unit.modes.contains(&ModalityMode::StructuredIllumination);
            let patterns = structured.then_some(PATTERNS);
            Arc::new(SimModalityUnit::new(
                unit.modes.clone(),
                patterns,
                state.clone(),
                clock.clone(),
            ))
        });
    Peripherals {
        illumination,
        filter_wheel,
        turret,
        modality,
    }
}

fn port(baud: u32) -> PortSettings {
    PortSettings {
        baud,
        ..PortSettings::default()
    }
}

/// The Euresys camera (ADR-0070), named by its feature map. Its pool is locked in RAM, for DMA.
#[cfg(feature = "euresys")]
fn euresys(
    config: &siq_config::model::EuresysCameraConfig,
    clock: Arc<dyn Clock>,
) -> anyhow::Result<(Arc<dyn Camera>, Backend)> {
    use siq_euresys::{EGrabber, EuresysCamera, FeatureMap, Strobe};
    use siq_types::Millis;
    let map = match &config.features {
        Some(path) => FeatureMap::load(path)?,
        None => FeatureMap::default(),
    };
    let grabber = EGrabber::open(None, config.interface, config.device)?;
    let mut camera = EuresysCamera::new(grabber, map, clock).context("the Euresys camera")?;
    if let Some(strobe) = &config.strobe {
        let strobe = Strobe {
            line: strobe.line.clone(),
            delay: Millis(strobe.delay_ms),
            duration: strobe.duration_ms.map(Millis),
        };
        camera = camera.with_strobe(strobe).context("the camera's strobe")?;
    }
    Ok((Arc::new(camera), Backend::WindowsLocked))
}

#[cfg(not(feature = "euresys"))]
fn euresys(
    _: &siq_config::model::EuresysCameraConfig,
    _: Arc<dyn Clock>,
) -> anyhow::Result<(Arc<dyn Camera>, Backend)> {
    anyhow::bail!(
        "this build has no Euresys camera: build spectrumiq-server with `--features euresys`"
    )
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
        CameraConfig::Euresys(camera) => euresys(camera, clock.clone())?,
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
            let emulator = DialectEmulator::new(dialect.clone(), state.clone(), clock.clone());
            let at = Micrometers(config.sim.focus_at);
            tokio::spawn(emulator.in_focus_at(at).run(device));
            Arc::new(AsciiFocusDrive::connect(host, dialect, clock.clone()).await?)
        }
        FocusConfig::Ascii(focus) => {
            let dialect = Arc::new(Dialect::load(&focus.dialect)?);
            let link = siq_serial::open(&focus.port, &port(focus.baud), dialect.framing())?;
            let connected = AsciiFocusDrive::connect(link, dialect, clock.clone()).await;
            Arc::new(connected.with_context(|| format!("the focus drive on {}", focus.port))?)
        }
    };
    Ok(Devices {
        camera,
        backend,
        stage,
        focus,
        peripherals: peripherals(&config.peripherals, &state, &clock),
        state,
    })
}
