//! The workflow engine (ADR-0052) on the simulated camera, with mock motion and peripherals: steps
//! in order, frames keyed by their indices, pause and resume, cancelling mid-move and
//! mid-capture, the error policies, and the manifest. REQ-WF-005, REQ-DATA-001.

use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use siq_acquire::{Acquisition, AcquisitionContext};
use siq_buffer::{Backend, PoolConfig};
use siq_calib::CalibrationRegistry;
use siq_devices::mock::{MockChanger, MockFocusDrive, MockIllumination, MockStage};
use siq_devices::{
    BoxFuture, CameraSettings, Device, DeviceError, DeviceInfo, DeviceResult, MoveOpts, Stage,
    StageStatus, Travel, Xyz, watch,
};
use siq_sim::camera::{Lens, SimCamera, SimCameraSetup};
use siq_sim::optics::{OpticsModel, SensorModel};
use siq_sim::state::TrueState;
use siq_storage::{Dataset, DatasetKind, DatasetReader, Snapshot};
use siq_synth::{Cells, CellsParams};
use siq_types::{
    Affine2, ChannelId, Clock, Hertz, Micrometers, MicrometersPerSecond, Millis, PixelFormat,
    SystemClock,
};
use siq_workflow::DeviceSet;
use siq_workflow::engine::*;

/// A mock stage whose moves take `delay`, and which counts its stops.
struct SlowStage {
    inner: MockStage,
    delay: Duration,
    stops: AtomicUsize,
}

impl Device for SlowStage {
    fn info(&self) -> &DeviceInfo {
        self.inner.info()
    }
}

impl Stage for SlowStage {
    fn move_to(&self, target: Xyz, opts: MoveOpts) -> BoxFuture<'_, DeviceResult<StageStatus>> {
        Box::pin(async move {
            tokio::time::sleep(self.delay).await;
            self.inner.move_to(target, opts).await
        })
    }

    fn move_by(&self, delta: Xyz, opts: MoveOpts) -> BoxFuture<'_, DeviceResult<StageStatus>> {
        self.inner.move_by(delta, opts)
    }

    fn jog(&self, velocity: Xyz<MicrometersPerSecond>) -> BoxFuture<'_, DeviceResult<()>> {
        self.inner.jog(velocity)
    }

    fn stop(&self) -> BoxFuture<'_, DeviceResult<()>> {
        self.stops.fetch_add(1, Ordering::Relaxed);
        self.inner.stop()
    }

    fn home(&self) -> BoxFuture<'_, DeviceResult<StageStatus>> {
        self.inner.home()
    }

    fn status(&self) -> watch::Receiver<StageStatus> {
        self.inner.status()
    }

    fn limits(&self) -> Travel<Xyz> {
        self.inner.limits()
    }
}

struct Rig {
    devices: DeviceSet,
    stage: Arc<SlowStage>,
    target: DatasetTarget,
    _folders: [tempfile::TempDir; 2],
}

/// The simulated camera on 128 × 96 pixels, the mocks, and the channels of `config/sim.toml`.
fn rig(move_delay: Duration) -> Rig {
    let clock: Arc<dyn Clock> = Arc::new(SystemClock::new());
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config/sim.toml");
    let config = siq_config::load(&path).unwrap();
    let lens = Lens {
        position: 0,
        magnification: 10.0,
        numerical_aperture: 0.30,
        immersion_index: 1.0,
    };
    let setup = SimCameraSetup {
        sensor: (128, 96),
        pixel_format: PixelFormat::Mono14,
        frame_rate: Hertz(9.0),
        pixel_size: Micrometers(3.76),
        lenses: vec![lens],
        optics: OpticsModel::default(),
        sensor_model: SensorModel::default(),
        trigger_latency: Millis(0.1),
        mount: Affine2::IDENTITY,
    };
    let world = Arc::new(Cells::new(CellsParams::default()));
    let state = Arc::new(TrueState::new(1));
    let camera = Arc::new(SimCamera::new(setup, world, state, clock.clone()));
    let pool = PoolConfig {
        name: "camera.pool".into(),
        slots: 8,
        slot_bytes: 128 * 96 * 2,
        backend: Backend::Heap,
    };
    let acquisition = Acquisition::new(camera.clone(), &pool, clock.clone()).unwrap();
    let um = |x: f64, y: f64, z: f64| Xyz {
        x: Micrometers(x),
        y: Micrometers(y),
        z: Micrometers(z),
    };
    let travel = Travel {
        min: um(-100_000.0, -75_000.0, -10_000.0),
        max: um(0.0, 0.0, 0.0),
    };
    let stage = Arc::new(SlowStage {
        inner: MockStage::new(travel, clock.clone()),
        delay: move_delay,
        stops: AtomicUsize::new(0),
    });
    acquisition.follow_stage(stage.status());
    let focus_travel = Travel {
        min: Micrometers(0.0),
        max: Micrometers(10_000.0),
    };
    let focus = Arc::new(MockFocusDrive::new(focus_travel, clock.clone()));
    let sources = ["white", "365nm", "470nm", "565nm"]
        .map(String::from)
        .to_vec();
    let filters = ["empty", "DAPI", "GFP", "TRITC"].map(String::from).to_vec();
    let folders = [tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap()];
    let devices = DeviceSet {
        acquisition,
        camera,
        stage: stage.clone(),
        focus,
        illumination: Some(Arc::new(MockIllumination::new(sources))),
        filter_wheel: Some(Arc::new(MockChanger::new(filters))),
        turret: None,
        modality: None,
        channels: config.channels.clone(),
        calibrations: Arc::new(CalibrationRegistry::open(folders[0].path()).unwrap()),
        clock,
    };
    let target = DatasetTarget {
        data_root: folders[1].path().to_owned(),
        min_free: 0,
        catalog: None,
        instrument: Some(Snapshot::of(&config).unwrap()),
    };
    Rig {
        devices,
        stage,
        target,
        _folders: folders,
    }
}

/// Every event, as it came.
#[derive(Default)]
struct Events(Mutex<Vec<RunEvent>>);

impl Events {
    fn all(&self) -> Vec<RunEvent> {
        self.0.lock().unwrap().clone()
    }

    fn steps(&self) -> Vec<String> {
        let progress = self.all().into_iter().filter_map(|event| match event {
            RunEvent::Progress { progress } => Some(progress.step),
            _ => None,
        });
        progress.collect()
    }
}

fn plan(steps: Vec<Step>) -> Plan {
    Plan {
        name: "engine".to_owned(),
        steps,
        on_error: ErrorPolicy::default(),
    }
}

/// The raw frames' keys, as `t c z p`, in the order they were written.
fn keys(report: &RunReport) -> Vec<String> {
    let dataset = Dataset::open(&report.path).unwrap();
    let reader = DatasetReader::open(&dataset).unwrap();
    let mut frames: Vec<_> = reader.frames().iter().map(|f| f.sidecar).collect();
    frames.sort_by_key(|frame| frame.meta.seq);
    let key = |k: siq_storage::FrameKey| format!("t{} c{} z{} p{}", k.t, k.c, k.z, k.p);
    frames.into_iter().map(|frame| key(frame.key)).collect()
}

#[tokio::test]
async fn steps_run_in_order_and_frames_are_keyed_by_their_indices() {
    let rig = rig(Duration::ZERO);
    let steps = vec![
        Step::MoveTo {
            x: -100.0,
            y: -200.0,
            z: None,
        },
        Step::Channel { id: ChannelId(1) },
        Step::Repeat {
            count: 3,
            index: Some(Index::Z),
            steps: vec![Step::Focus { z: 5000.0 }, Step::Snap],
        },
        Step::Index {
            index: Index::P,
            value: 4,
        },
        Step::Channel { id: ChannelId(2) },
        Step::Snap,
    ];
    assert_eq!(count(&steps), 10);
    let events = Events::default();
    let observe = |event| events.0.lock().unwrap().push(event);
    let control = RunControl::new();
    let report = run(&rig.devices, &rig.target, &plan(steps), &control, &observe)
        .await
        .unwrap();
    assert_eq!(report.outcome, Outcome::Finished, "{report:?}");
    assert_eq!((report.steps, report.frames), ([10, 10], 4));
    let mut expected = vec!["move to (-100, -200) µm".to_owned(), "channel 1".to_owned()];
    for z in 0..3 {
        expected.extend(["focus to 5000 µm".to_owned(), format!("snap t0 c1 z{z} p0")]);
    }
    expected.extend(["channel 2".to_owned(), "snap t0 c2 z0 p4".to_owned()]);
    assert_eq!(events.steps(), expected);
    // A repeat's index counts within it only; an index set holds for the steps after it.
    assert_eq!(
        keys(&report),
        ["t0 c1 z0 p0", "t0 c1 z1 p0", "t0 c1 z2 p0", "t0 c2 z0 p4"]
    );
    let at = rig.stage.status().borrow().position;
    assert_eq!((at.x.0, at.y.0), (-100.0, -200.0));
}

#[tokio::test]
async fn a_paused_run_waits_before_its_next_step_then_goes_on() {
    let rig = rig(Duration::ZERO);
    let control = RunControl::new();
    let events = Arc::new(Events::default());
    let observe = {
        let (events, control) = (events.clone(), control.clone());
        move |event: RunEvent| {
            // Paused once the first step is done.
            if matches!(&event, RunEvent::Progress { progress } if progress.done == 1) {
                control.pause();
            }
            events.0.lock().unwrap().push(event);
        }
    };
    let steps = vec![Step::Repeat {
        count: 3,
        index: Some(Index::P),
        steps: vec![Step::Snap],
    }];
    let running = {
        let (devices, target, control) = (rig.devices.clone(), rig.target.clone(), control.clone());
        tokio::spawn(async move { run(&devices, &target, &plan(steps), &control, &observe).await })
    };
    // Once paused, nothing more happens until it is resumed.
    let paused = async {
        while events.all().last() != Some(&RunEvent::Paused) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    };
    tokio::time::timeout(Duration::from_secs(10), paused)
        .await
        .expect("paused");
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(control.command(), Command::Pause);
    assert_eq!(events.steps().len(), 1, "no step while paused");
    assert_eq!(events.all().last(), Some(&RunEvent::Paused));
    control.resume();
    let report = running.await.unwrap().unwrap();
    assert_eq!(
        (report.outcome, report.frames),
        (Outcome::Finished, 3),
        "{report:?}"
    );
    assert!(events.all().contains(&RunEvent::Resumed));
}

#[tokio::test]
async fn a_cancelled_run_abandons_its_move_halts_the_stage_and_keeps_its_frames() {
    let rig = rig(Duration::from_secs(30));
    let control = RunControl::new();
    let observe = {
        let control = control.clone();
        // Cancelled once the first frame is taken, while the stage is moving.
        move |event: RunEvent| {
            if matches!(event, RunEvent::Frame { .. }) {
                let control = control.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    control.cancel();
                });
            }
        }
    };
    let steps = vec![
        Step::Snap,
        Step::MoveTo {
            x: -5000.0,
            y: -5000.0,
            z: None,
        },
        Step::Channel { id: ChannelId(1) },
        Step::Snap,
    ];
    let report = tokio::time::timeout(
        Duration::from_secs(10),
        run(&rig.devices, &rig.target, &plan(steps), &control, &observe),
    )
    .await
    .expect("the move is abandoned at once")
    .unwrap();
    assert_eq!(report.outcome, Outcome::Cancelled);
    assert_eq!((report.steps, report.frames), ([1, 4], 1));
    assert_eq!(
        rig.stage.stops.load(Ordering::Relaxed),
        1,
        "the stage halted"
    );
    let at = rig.stage.status().borrow().position;
    assert_eq!((at.x.0, at.y.0), (0.0, 0.0), "the move didn't happen");
}

#[tokio::test]
async fn a_capture_under_way_finishes_before_a_cancelled_run_stops() {
    let rig = rig(Duration::ZERO);
    // A long exposure, cancelled during the first.
    let settings = CameraSettings {
        exposure: Millis(400.0),
        ..rig.devices.camera.settings()
    };
    rig.devices.acquisition.configure(settings).await.unwrap();
    let control = RunControl::new();
    let cancel = {
        let control = control.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(150)).await;
            control.cancel();
        })
    };
    let steps = vec![Step::Repeat {
        count: 3,
        index: Some(Index::P),
        steps: vec![Step::Snap],
    }];
    let report = run(&rig.devices, &rig.target, &plan(steps), &control, &|_| {})
        .await
        .unwrap();
    cancel.await.unwrap();
    assert_eq!(report.outcome, Outcome::Cancelled);
    assert_eq!((report.steps, report.frames), ([1, 3], 1));
    // The camera is left ready: a snap after works.
    let after = rig.devices.acquisition.snap(false).await.unwrap();
    assert_eq!(after.meta().exposure, Millis(400.0));
}

#[tokio::test]
async fn a_failed_step_is_tried_again_skipped_or_ends_the_run() {
    let rig = rig(Duration::ZERO);
    let move_to = || Step::MoveTo {
        x: -10.0,
        y: -10.0,
        z: None,
    };
    let fault = || DeviceError::Timeout("no reply".to_owned());
    let with = |on_error| Plan {
        on_error,
        ..plan(vec![move_to(), Step::Snap])
    };
    let problems = |events: &Events| -> Vec<(u32, Then)> {
        let all = events.all().into_iter();
        let problems = all.filter_map(|event| match event {
            RunEvent::Problem { attempt, then, .. } => Some((attempt, then)),
            _ => None,
        });
        problems.collect()
    };
    let control = RunControl::new();
    // The plan run with `policy`, its move failing once: the report and the problems.
    let attempt = async |policy| {
        let events = Events::default();
        rig.stage.inner.fail_next(fault());
        let observe = |event| events.0.lock().unwrap().push(event);
        let report = run(&rig.devices, &rig.target, &with(policy), &control, &observe).await;
        (report.unwrap(), problems(&events))
    };

    let retried = ErrorPolicy {
        retries: 1,
        skip: false,
    };
    let (report, seen) = attempt(retried).await;
    assert_eq!((report.outcome, report.frames), (Outcome::Finished, 1));
    assert_eq!(seen, [(1, Then::Retry)]);

    let skipped = ErrorPolicy {
        retries: 0,
        skip: true,
    };
    let (report, seen) = attempt(skipped).await;
    assert_eq!((report.outcome, report.steps), (Outcome::Finished, [2, 2]));
    assert_eq!(seen, [(1, Then::Skip)]);

    let (report, seen) = attempt(ErrorPolicy::default()).await;
    assert_eq!((report.outcome, report.frames), (Outcome::Failed, 0));
    assert_eq!(seen, [(1, Then::Stop)]);
    let error = report.error.unwrap();
    assert!(
        error.contains("move to (-10, -10) µm") && error.contains("no reply"),
        "{error}"
    );

    // A group's own policy wins over the plan's.
    rig.stage.inner.fail_next(fault());
    let steps = vec![Step::Policy {
        on_error: skipped,
        steps: vec![move_to()],
    }];
    let report = run(&rig.devices, &rig.target, &plan(steps), &control, &|_| {}).await;
    assert_eq!(report.unwrap().outcome, Outcome::Finished);
}

#[tokio::test]
async fn the_manifest_records_the_plan_the_instrument_and_the_calibration() {
    let rig = rig(Duration::ZERO);
    let steps = vec![Step::Repeat {
        count: 2,
        index: Some(Index::T),
        steps: vec![Step::Wait { ms: 5.0 }, Step::Snap],
    }];
    // A calibration in force, as the server stamps the frames with.
    let registry = &rig.devices.calibrations;
    let moves: Vec<siq_calib::Move> = [[100.0, 0.0], [0.0, 100.0], [-100.0, -100.0]]
        .into_iter()
        .map(|stage| siq_calib::Move {
            stage,
            shift: [-stage[0] * 2.66, -stage[1] * 2.66],
        })
        .collect();
    let fit = siq_calib::fit(&moves).unwrap();
    let objective = siq_types::ObjectiveId(1);
    let record = siq_calib::CalibrationRecord::new(objective, &fit, None, "stage_camera");
    registry.save(&record, &[]).unwrap();
    let context = AcquisitionContext {
        objective: Some(objective),
        calibration: Some(record.id),
        ..rig.devices.acquisition.context()
    };
    rig.devices.acquisition.set_context(context);

    let plan = plan(steps);
    let control = RunControl::new();
    let report = run(&rig.devices, &rig.target, &plan, &control, &|_| {})
        .await
        .unwrap();
    assert_eq!(keys(&report), ["t0 c0 z0 p0", "t1 c0 z0 p0"]);
    let manifest = Dataset::open(&report.path).unwrap().manifest().clone();
    assert_eq!(manifest.kind, DatasetKind::Sequence);
    assert_eq!(manifest.name, "engine");
    let recorded: Plan = serde_json::from_value(manifest.plan.unwrap()).unwrap();
    assert_eq!(recorded, plan);
    assert!(manifest.instrument.unwrap().value["channels"].is_array());
    assert_eq!(
        manifest.calibration.unwrap().value["id"],
        serde_json::json!(record.id)
    );

    // A plan that can't run makes no dataset.
    let unknown = self::plan(vec![Step::Channel { id: ChannelId(9) }]);
    let refused = run(&rig.devices, &rig.target, &unknown, &control, &|_| {}).await;
    assert!(matches!(refused, Err(RunError::Invalid(why)) if why.contains("no channel 9")));
    // Two frames at one place, unnumbered: the second is refused.
    let twice = self::plan(vec![Step::Snap, Step::Snap]);
    let report = run(&rig.devices, &rig.target, &twice, &control, &|_| {}).await;
    let report = report.unwrap();
    assert_eq!((report.outcome, report.frames), (Outcome::Failed, 1));
    let error = report.error.unwrap();
    assert!(
        error.contains("already taken at t0000_c00_z000_p00000"),
        "{error}"
    );
    let empty = self::plan(vec![]);
    let refused = run(&rig.devices, &rig.target, &empty, &control, &|_| {}).await;
    assert!(matches!(refused, Err(RunError::Invalid(_))));
    let datasets = std::fs::read_dir(rig.target.data_root.join("datasets")).unwrap();
    assert_eq!(datasets.count(), 2);
}
