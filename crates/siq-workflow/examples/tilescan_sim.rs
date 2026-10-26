//! A tile scan on the simulated instrument of `config/sim.toml`: a region planned into a snake of
//! tiles with 10 % overlap, from the simulator's own stage–camera mount, its estimate, then the run
//! through the workflow engine, a frame per tile in brightfield and in DAPI.
//!
//! ```text
//! cargo run --release -p siq-workflow --example tilescan_sim [-- --out <data root>]
//! ```
//!
//! The scan is 5 × 5 tiles of 2048 × 1536 pixels; the stage is the GRBL driver against its
//! emulator. The dataset goes in a temporary data root removed at the end, or under
//! `<data root>` with `--out`: its `layout.json` holds the grid. Smoke mode (`SIQ_SMOKE=1`) scans
//! 3 × 3 tiles with the `fast` profile, in one channel.

use std::error::Error;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use siq_acquire::{Acquisition, AcquisitionContext};
use siq_buffer::{Backend, PoolConfig};
use siq_calib::{CalibrationRecord, CalibrationRegistry, Fit};
use siq_config::model::{CameraConfig, SimProfile, SimStageConfig};
use siq_devices::mock::MockFocusDrive;
use siq_devices::{Stage, Travel};
use siq_grbl::{GrblOptions, GrblStage};
use siq_serial::{Framing, duplex};
use siq_sim::camera::{SimCamera, SimCameraSetup};
use siq_sim::grbl::{GrblEmulator, GrblErrors, GrblSettings};
use siq_sim::peripherals::{SimChanger, SimIllumination};
use siq_sim::state::TrueState;
use siq_storage::Snapshot;
use siq_synth::WorldInfo;
use siq_types::{Affine2, ChannelId, Clock, Micrometers, ObjectiveId, SystemClock};
use siq_workflow::DeviceSet;
use siq_workflow::engine::{
    DatasetTarget, ErrorPolicy, LAYOUT, Plan, RunControl, RunEvent, Step, TileFocus, run,
};
use siq_workflow::scan::{Focusing, Order, Region, Timing, estimate, plan};

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
    let (tiles, channels) = match smoke {
        true => {
            (camera.width, camera.height) = (1024, 768);
            config.sim.profile = SimProfile::Fast;
            (3.0, vec![ChannelId(1)])
        }
        false => {
            (camera.width, camera.height) = (2048, 1536);
            (5.0, vec![ChannelId(1), ChannelId(2)])
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

    // The simulator's own mount as the calibration in force: a stage–camera calibration would
    // find it (P5.2).
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
        min: Micrometers(0.0),
        max: Micrometers(10_000.0),
    };
    let devices = DeviceSet {
        acquisition,
        camera,
        stage,
        focus: Arc::new(MockFocusDrive::new(travel, clock.clone())),
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
    let target = DatasetTarget {
        data_root: root,
        min_free: 0,
        catalog: None,
        instrument: Some(Snapshot::of(&config)?),
    };

    // A region `tiles` across and down, at 10 % overlap: (tiles − 0.4) steps of 0.9 of a frame.
    let field = [f64::from(width), f64::from(height)];
    let step = [
        0.9 * field[0] * 0.376 / 1.002,
        0.9 * field[1] * 0.376 / 1.002,
    ];
    let half = [(tiles - 0.4) / 2.0 * step[0], (tiles - 0.4) / 2.0 * step[1]];
    let centre = [-20_000.0, -15_000.0];
    let region = Region::Rect {
        x0: centre[0] - half[0],
        y0: centre[1] - half[1],
        x1: centre[0] + half[0],
        y1: centre[1] + half[1],
    };
    let grid = plan(&region, &l, field, 0.1, Order::Snake)?;
    let exposures: Vec<f64> = channels
        .iter()
        .filter_map(|id| devices.channels.iter().find(|c| c.id == *id))
        .map(|channel| channel.exposure.0 / 1000.0)
        .collect();
    let at = devices.stage.status().borrow().position;
    let bytes = u64::from(width) * u64::from(height) * 2;
    let estimated = estimate(
        &grid,
        [at.x.0, at.y.0],
        0.05,
        &exposures,
        bytes,
        &Focusing::default(),
        &Timing::default(),
    );
    println!(
        "{} tiles of {width} × {height}, {} frames: about {:.1} s and {:.1} MB",
        estimated.tiles,
        estimated.frames,
        estimated.seconds,
        estimated.bytes as f64 / 1e6
    );

    let scan = Step::TileScan {
        region,
        overlap: 0.1,
        order: Order::Snake,
        channels,
        settle_ms: 50.0,
        focus: TileFocus::None,
        z_stack: None,
    };
    let plan = Plan {
        name: "tilescan-sim".to_owned(),
        steps: vec![scan],
        on_error: ErrorPolicy::default(),
    };
    let began = Instant::now();
    let observe = |event: RunEvent| {
        if let RunEvent::Frame { c, p, .. } = event {
            println!("  tile p{p:<3} channel {c}");
        }
    };
    let report = run(&devices, &target, &plan, &RunControl::new(), &observe).await?;
    println!(
        "{:?} in {:.1} s, estimated {:.1} s: {} frames",
        report.outcome,
        began.elapsed().as_secs_f64(),
        estimated.seconds,
        report.frames
    );
    let layout = std::fs::read_to_string(report.path.join(LAYOUT))?;
    let layout: serde_json::Value = serde_json::from_str(&layout)?;
    let steps = &layout["scans"][0]["steps"];
    println!(
        "the grid's steps, in µm: along a row {}, down a column {}",
        steps[0], steps[1]
    );
    if out.is_some() {
        println!("kept in {}", report.path.display());
    }
    Ok(())
}
