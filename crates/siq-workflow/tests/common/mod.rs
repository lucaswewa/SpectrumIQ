//! What the workflow tests share: the simulated instrument in brightfield, its stage through the
//! GRBL driver against its emulator, its focus drive the simulator's instant one, and its
//! calibration the simulator's own mount.

#![allow(dead_code, reason = "each test binary uses some of it")]

use std::path::Path;
use std::sync::Arc;

use siq_acquire::{Acquisition, AcquisitionContext};
use siq_buffer::{Backend, PoolConfig};
use siq_calib::{CalibrationRecord, CalibrationRegistry, Fit};
use siq_config::model::{CameraConfig, SimStageConfig};
use siq_devices::{Stage, Travel};
use siq_grbl::{GrblOptions, GrblStage};
use siq_serial::{Framing, duplex};
use siq_sim::camera::{SimCamera, SimCameraSetup};
use siq_sim::grbl::{GrblEmulator, GrblErrors, GrblSettings};
use siq_sim::peripherals::{SimChanger, SimFocus, SimIllumination};
use siq_sim::state::TrueState;
use siq_storage::Snapshot;
use siq_synth::{World, WorldInfo};
use siq_types::{Affine2, Clock, Micrometers, ObjectiveId, SystemClock};
use siq_workflow::DeviceSet;
use siq_workflow::engine::DatasetTarget;

/// The simulated instrument: its devices, its true state, where its datasets go, and the folders
/// that hold them.
pub struct Rig {
    pub devices: DeviceSet,
    pub state: Arc<TrueState>,
    pub target: DatasetTarget,
    pub folders: [tempfile::TempDir; 2],
}

/// The simulated instrument of `config/sim.toml`, its camera `size` pixels looking at `world`
/// (the configured one if `None`), calibrated with the simulator's own mount.
pub async fn instrument(size: (u32, u32), world: Option<Arc<dyn World>>) -> Rig {
    instrument_in(size, world, None).await
}

/// [`instrument`], its camera's sensor a colour one if `cfa` is given.
pub async fn instrument_in(
    size: (u32, u32),
    world: Option<Arc<dyn World>>,
    cfa: Option<siq_types::Cfa>,
) -> Rig {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config/sim.toml");
    let mut config = siq_config::load(&path).unwrap();
    let CameraConfig::Sim(camera) = &mut config.camera else {
        panic!("a simulated camera")
    };
    (camera.width, camera.height) = size;
    camera.cfa = cfa;
    let clock: Arc<dyn Clock> = Arc::new(SystemClock::new());
    let peripherals = &config.peripherals;
    let sources = peripherals.illumination.as_ref().unwrap().sources.clone();
    let filters = peripherals.filter_wheel.as_ref().unwrap().filters.clone();
    let state = Arc::new(TrueState::new(sources.len()));
    let world = world.unwrap_or_else(|| {
        let built = WorldInfo::from_config(&config.sim.world, config.sim.seed).build();
        built.unwrap().into()
    });
    let setup = SimCameraSetup::from_config(camera, &config.objectives, &config.sim);
    let l = setup
        .mount
        .compose(&Affine2::scale(10.0 / 3.76, 10.0 / 3.76));
    let camera = Arc::new(SimCamera::new(setup, world, state.clone(), clock.clone()));
    let errors = GrblErrors::from(&config.sim.errors);
    let emulator = GrblEmulator::new(
        GrblSettings::default(),
        errors,
        1,
        state.clone(),
        clock.clone(),
    );
    let (host, device) = duplex(Framing::default(), "grbl");
    tokio::spawn(emulator.run(device));
    let options = GrblOptions::from(&SimStageConfig::default());
    let stage: Arc<dyn Stage> = Arc::new(
        GrblStage::connect(host, options, clock.clone())
            .await
            .unwrap(),
    );
    let pool = PoolConfig {
        name: "camera.pool".into(),
        slots: 8,
        slot_bytes: size.0 as usize * size.1 as usize * 2,
        backend: Backend::Heap,
    };
    let acquisition = Acquisition::new(camera.clone(), &pool, clock.clone()).unwrap();
    acquisition.follow_stage(stage.status());
    let travel = Travel {
        min: Micrometers(-500.0),
        max: Micrometers(500.0),
    };
    let focus = Arc::new(SimFocus::new(travel, state.clone(), clock.clone()));
    // Not followed: it reports only when it moves, so a snap would wait for a reading. The tests
    // read where it is at each frame.
    let folders = [tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap()];
    let calibrations = Arc::new(CalibrationRegistry::open(folders[0].path()).unwrap());
    let fit = Fit {
        affine: l,
        residuals: Vec::new(),
    };
    let record = CalibrationRecord::new(ObjectiveId(1), &fit, None, "the simulator's mount");
    calibrations.save(&record, &[]).unwrap();
    acquisition.set_context(AcquisitionContext {
        objective: Some(ObjectiveId(1)),
        calibration: Some(record.id),
        ..acquisition.context()
    });
    let devices = DeviceSet {
        acquisition,
        camera,
        stage,
        focus,
        illumination: Some(Arc::new(SimIllumination::new(
            sources,
            state.clone(),
            clock.clone(),
        ))),
        filter_wheel: Some(Arc::new(SimChanger::filter_wheel(
            filters,
            state.clone(),
            clock.clone(),
        ))),
        turret: None,
        modality: None,
        channels: config.channels.clone(),
        calibrations,
        clock,
    };
    let target = DatasetTarget {
        data_root: folders[1].path().to_owned(),
        min_free: 0,
        catalog: None,
        instrument: Some(Snapshot::of(&config).unwrap()),
    };
    // In brightfield: the light at the channel's level, not saturating the frames.
    let brightfield = devices.channels[0].clone();
    siq_workflow::routines::channel::apply(&devices, &brightfield)
        .await
        .unwrap();
    Rig {
        devices,
        state,
        target,
        folders,
    }
}
