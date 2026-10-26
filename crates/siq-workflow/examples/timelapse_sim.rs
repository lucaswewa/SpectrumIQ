//! A time-lapse on the simulated instrument of `config/sim.toml` (ADR-0055): every 4 s, three
//! times, two positions, a z-stack of five planes 2 µm apart at each, in brightfield, through the
//! workflow engine. Each frame is printed as it comes, then each time point's schedule.
//!
//! ```text
//! cargo run --release -p siq-workflow --example timelapse_sim [-- --out <data root>]
//! ```
//!
//! The camera is 512 × 384 pixels; the stage is the GRBL driver against its emulator, and the
//! focus drive the simulator's instant one. The dataset goes in a temporary data root removed at
//! the end, or under `<data root>` with `--out`: its `layout.json` holds the positions and the
//! points. Smoke mode (`SIQ_SMOKE=1`) takes two points 2 s apart, of three planes, with a
//! 256 × 192 camera and the `fast` profile.

use std::error::Error;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use siq_acquire::{Acquisition, AcquisitionContext};
use siq_buffer::{Backend, PoolConfig};
use siq_calib::{CalibrationRecord, CalibrationRegistry, Fit};
use siq_config::model::{CameraConfig, SimProfile, SimStageConfig};
use siq_devices::{Stage, Travel};
use siq_grbl::{GrblOptions, GrblStage};
use siq_serial::{Framing, duplex};
use siq_sim::camera::{SimCamera, SimCameraSetup};
use siq_sim::grbl::{GrblEmulator, GrblErrors, GrblSettings};
use siq_sim::peripherals::{SimChanger, SimFocus, SimIllumination};
use siq_sim::state::TrueState;
use siq_storage::Snapshot;
use siq_synth::WorldInfo;
use siq_types::{Affine2, Clock, Micrometers, ObjectiveId, SystemClock};
use siq_workflow::DeviceSet;
use siq_workflow::engine::{
    DatasetTarget, ErrorPolicy, LAYOUT, Layout, Overrun, Plan, Position, RunControl, RunEvent,
    Step, ZDirection, ZPlanes, run,
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

    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config/sim.toml");
    let mut config = siq_config::load(&path)?;
    let CameraConfig::Sim(camera) = &mut config.camera else {
        return Err("config/sim.toml has a simulated camera".into());
    };
    let (interval_s, points, range_um) = match smoke {
        true => {
            (camera.width, camera.height) = (256, 192);
            config.sim.profile = SimProfile::Fast;
            (2.0, 2, 4.0)
        }
        false => {
            (camera.width, camera.height) = (512, 384);
            (4.0, 3, 8.0)
        }
    };
    let clock: Arc<dyn Clock> = Arc::new(SystemClock::new());
    let peripherals = &config.peripherals;
    let sources = peripherals
        .illumination
        .as_ref()
        .map_or_else(Vec::new, |l| l.sources.clone());
    let filters = peripherals
        .filter_wheel
        .as_ref()
        .map_or_else(Vec::new, |w| w.filters.clone());
    let state = Arc::new(TrueState::new(sources.len()));
    let world = WorldInfo::from_config(&config.sim.world, config.sim.seed).build()?;
    let setup = SimCameraSetup::from_config(camera, &config.objectives, &config.sim);
    let (width, height) = setup.sensor;
    let l = setup
        .mount
        .compose(&Affine2::scale(10.0 / 3.76, 10.0 / 3.76));
    let camera = Arc::new(SimCamera::new(
        setup,
        world.into(),
        state.clone(),
        clock.clone(),
    ));
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
    let stage: Arc<dyn Stage> = Arc::new(GrblStage::connect(host, options, clock.clone()).await?);
    let pool = PoolConfig {
        name: "camera.pool".into(),
        slots: 8,
        slot_bytes: width as usize * height as usize * 2,
        backend: Backend::Heap,
    };
    let acquisition = Acquisition::new(camera.clone(), &pool, clock.clone())?;
    acquisition.follow_stage(stage.status());
    let settings = tempfile::tempdir()?;
    let calibrations = Arc::new(CalibrationRegistry::open(settings.path())?);
    let fit = Fit {
        affine: l,
        residuals: Vec::new(),
    };
    let record = CalibrationRecord::new(ObjectiveId(1), &fit, None, "the simulator's mount");
    calibrations.save(&record, &[])?;
    acquisition.set_context(AcquisitionContext {
        objective: Some(ObjectiveId(1)),
        calibration: Some(record.id),
        ..acquisition.context()
    });
    let travel = Travel {
        min: Micrometers(-500.0),
        max: Micrometers(500.0),
    };
    let devices = DeviceSet {
        acquisition,
        camera,
        stage,
        focus: Arc::new(SimFocus::new(travel, state.clone(), clock.clone())),
        illumination: Some(Arc::new(SimIllumination::new(
            sources,
            state.clone(),
            clock.clone(),
        ))),
        filter_wheel: Some(Arc::new(SimChanger::filter_wheel(
            filters,
            state,
            clock.clone(),
        ))),
        turret: None,
        modality: None,
        channels: config.channels.clone(),
        calibrations,
        clock,
    };
    let brightfield = devices.channels[0].clone();
    siq_workflow::routines::channel::apply(&devices, &brightfield).await?;
    let target = DatasetTarget {
        data_root: root,
        min_free: 0,
        catalog: None,
        instrument: Some(Snapshot::of(&config)?),
    };

    let at = |name: &str, x: f64, y: f64| Position {
        name: name.to_owned(),
        x,
        y,
        z: None,
        focus: None,
    };
    let stack = Step::ZStack {
        planes: ZPlanes {
            range_um,
            step_um: 2.0,
            direction: ZDirection::Up,
        },
        steps: Vec::new(),
    };
    let positions = Step::MultiPosition {
        positions: vec![
            at("left", -20_000.0, -15_000.0),
            at("right", -19_000.0, -15_000.0),
        ],
        autofocus: None,
        steps: vec![stack],
    };
    let plan = Plan {
        name: "timelapse-sim".to_owned(),
        steps: vec![Step::TimeLapse {
            interval_s,
            count: Some(points),
            duration_s: None,
            overrun: Overrun::Skip,
            steps: vec![positions],
        }],
        on_error: ErrorPolicy::default(),
    };
    println!("{points} points every {interval_s} s, at two positions, {range_um} µm stacks");
    let began = Instant::now();
    let observe = |event: RunEvent| match event {
        RunEvent::Frame { t, z, p, .. } => {
            let s = began.elapsed().as_secs_f64();
            println!("  {s:6.2} s  t{t} p{p} z{z}");
        }
        RunEvent::Problem { step, error, .. } => println!("  {step}: {error}"),
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
    let layout: Layout = serde_json::from_str(&std::fs::read_to_string(report.path.join(LAYOUT))?)?;
    for lapse in &layout.time_lapses {
        for point in &lapse.points {
            println!(
                "  t{}: due at {:.2} s, began at {:.2} s",
                point.t, point.due_s, point.began_s
            );
        }
        if !lapse.skipped.is_empty() {
            println!("  skipped: {:?}", lapse.skipped);
        }
    }
    if out.is_some() {
        println!("kept in {}", report.path.display());
    }
    Ok(())
}
