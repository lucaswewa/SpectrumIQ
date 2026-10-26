//! A plan run by the workflow engine on the simulated instrument of `config/sim.toml`: twice, at
//! three places, a frame in brightfield and in DAPI, as a small time-lapse of a multi-position
//! acquisition. It prints each step as it is done, then the dataset's frames.
//!
//! ```text
//! cargo run --release -p siq-workflow --example run_plan_sim [-- --out <data root>]
//! ```
//!
//! The camera is 1024 × 768; the stage is the GRBL driver against its emulator, with the
//! configured backlash and errors; the lights and the filter wheel are simulated. The dataset
//! goes in a temporary data root removed at the end, or under `<data root>` with `--out`. Smoke
//! mode (`SIQ_SMOKE=1`) uses a 256 × 192 camera.

use std::error::Error;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use siq_acquire::Acquisition;
use siq_buffer::{Backend, PoolConfig};
use siq_calib::CalibrationRegistry;
use siq_config::model::{CameraConfig, SimStageConfig};
use siq_devices::mock::MockFocusDrive;
use siq_devices::{Stage, Travel};
use siq_grbl::{GrblOptions, GrblStage};
use siq_serial::{Framing, duplex};
use siq_sim::camera::{SimCamera, SimCameraSetup};
use siq_sim::grbl::{GrblEmulator, GrblErrors, GrblSettings};
use siq_sim::peripherals::{SimChanger, SimIllumination};
use siq_sim::state::TrueState;
use siq_storage::{Dataset, DatasetReader, Snapshot};
use siq_synth::WorldInfo;
use siq_types::{ChannelId, Clock, Micrometers, SystemClock};
use siq_workflow::DeviceSet;
use siq_workflow::engine::{
    DatasetTarget, ErrorPolicy, Index, Plan, RunControl, RunEvent, Step, run,
};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn Error>> {
    let smoke = std::env::var_os("SIQ_SMOKE").is_some();
    let args: Vec<String> = std::env::args().collect();
    let out = args
        .iter()
        .position(|arg| arg == "--out")
        .and_then(|at| args.get(at + 1))
        .map(PathBuf::from);
    let temporary = tempfile::tempdir()?;
    let root = out.as_deref().unwrap_or(temporary.path()).to_owned();

    // The simulated instrument.
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config/sim.toml");
    let mut config = siq_config::load(&path)?;
    let CameraConfig::Sim(camera) = &mut config.camera else {
        return Err("config/sim.toml has a simulated camera".into());
    };
    (camera.width, camera.height) = if smoke { (256, 192) } else { (1024, 768) };
    let (width, height) = (camera.width, camera.height);
    let clock: Arc<dyn Clock> = Arc::new(SystemClock::new());
    let peripherals = &config.peripherals;
    let sources = peripherals.illumination.as_ref().map(|l| l.sources.clone());
    let filters = peripherals.filter_wheel.as_ref().map(|w| w.filters.clone());
    let state = Arc::new(TrueState::new(sources.as_ref().map_or(0, Vec::len)));
    let world = WorldInfo::from_config(&config.sim.world, config.sim.seed).build()?;
    let setup = SimCameraSetup::from_config(camera, &config.objectives, &config.sim);
    let camera = Arc::new(SimCamera::new(
        setup,
        world.into(),
        state.clone(),
        clock.clone(),
    ));
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
    let options = GrblOptions::from(&SimStageConfig::default());
    let stage: Arc<dyn Stage> = Arc::new(GrblStage::connect(host, options, clock.clone()).await?);
    let pool = PoolConfig {
        name: "camera.pool".into(),
        slots: 8,
        slot_bytes: width as usize * height as usize * 2,
        backend: Backend::Heap,
    };
    let acquisition = Acquisition::new(camera.clone(), &pool, clock.clone())?;
    acquisition.follow_stage(stage.status());
    let travel = Travel {
        min: Micrometers(0.0),
        max: Micrometers(10_000.0),
    };
    let settings = tempfile::tempdir()?;
    let devices = DeviceSet {
        acquisition,
        camera,
        stage,
        focus: Arc::new(MockFocusDrive::new(travel, clock.clone())),
        illumination: sources.map(|sources| {
            Arc::new(SimIllumination::new(sources, state.clone(), clock.clone())) as _
        }),
        filter_wheel: filters.map(|filters| {
            Arc::new(SimChanger::filter_wheel(
                filters,
                state.clone(),
                clock.clone(),
            )) as _
        }),
        turret: None,
        modality: None,
        channels: config.channels.clone(),
        calibrations: Arc::new(CalibrationRegistry::open(settings.path())?),
        clock: clock.clone(),
    };
    let target = DatasetTarget {
        data_root: root,
        min_free: 0,
        catalog: None,
        instrument: Some(Snapshot::of(&config)?),
    };

    // Twice, at three places a field apart: brightfield, then DAPI.
    let places = [
        (-20_000.0, -15_000.0),
        (-19_600.0, -15_000.0),
        (-19_600.0, -14_700.0),
    ];
    let mut visit = Vec::new();
    for (p, (x, y)) in (0..).zip(places) {
        visit.extend([
            Step::Index {
                index: Index::P,
                value: p,
            },
            Step::MoveTo { x, y, z: None },
            Step::Channel { id: ChannelId(1) },
            Step::Snap,
            Step::Channel { id: ChannelId(2) },
            Step::Snap,
        ]);
    }
    visit.push(Step::Wait { ms: 500.0 });
    let plan = Plan {
        name: "run-plan-sim".to_owned(),
        steps: vec![Step::Repeat {
            count: 2,
            index: Some(Index::T),
            steps: visit,
        }],
        on_error: ErrorPolicy::default(),
    };
    let began = Instant::now();
    let observe = |event: RunEvent| match event {
        RunEvent::Progress { progress } => println!(
            "{:>3}/{} {:<28} eta {:>5.1} s",
            progress.done,
            progress.total,
            progress.step,
            progress.eta_s.unwrap_or(0.0)
        ),
        RunEvent::Problem { step, error, .. } => println!("    {step} failed: {error}"),
        _ => {}
    };
    let report = run(&devices, &target, &plan, &RunControl::new(), &observe).await?;
    println!(
        "{:?} in {:.1} s: {} frames, {} of {} steps",
        report.outcome,
        began.elapsed().as_secs_f64(),
        report.frames,
        report.steps[0],
        report.steps[1]
    );
    let dataset = Dataset::open(&report.path)?;
    let reader = DatasetReader::open(&dataset)?;
    let mut frames: Vec<_> = reader.frames().iter().map(|frame| frame.sidecar).collect();
    frames.sort_by_key(|frame| frame.key);
    for frame in frames {
        let pose = frame.meta.pose.unwrap_or_default();
        let channel = frame.meta.channel.map_or(0, |channel| channel.0);
        println!(
            "{}  channel {channel}  at ({:.1}, {:.1}) µm",
            frame.key, pose.x.0, pose.y.0
        );
    }
    if out.is_some() {
        println!("kept in {}", report.path.display());
    }
    Ok(())
}
