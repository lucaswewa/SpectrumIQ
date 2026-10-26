//! The engine with the simulated camera and the GRBL driver against its emulator, on a clock that
//! runs ten times faster than real time: poses, context, modes, numbering, captures and lost
//! frames.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use siq_acquire::{Acquisition, AcquisitionContext, Mode};
use siq_buffer::{Backend, Policy, PoolConfig, Subscriber};
use siq_config::model::{SimErrors, SimStageConfig};
use siq_devices::{CameraSettings, MoveOpts, Stage, Xyz};
use siq_grbl::{GrblOptions, GrblStage};
use siq_serial::{Framing, duplex};
use siq_sim::camera::{Lens, SimCamera, SimCameraSetup};
use siq_sim::grbl::{GrblEmulator, GrblErrors, GrblSettings};
use siq_sim::optics::{OpticsModel, SensorModel};
use siq_sim::state::TrueState;
use siq_synth::{Cells, CellsParams};
use siq_types::{
    Affine2, CalibrationId, ChannelId, Clock, FrameMeta, Hertz, ManualClock, Micrometers, Millis,
    ObjectiveId, PixelFormat, PoseQuality, Roi, Timestamp,
};

/// How much faster than real time the clock runs.
const SPEED: u64 = 10;

fn running_clock() -> Arc<ManualClock> {
    let (clock, start) = (Arc::new(ManualClock::new()), Instant::now());
    let ticker = clock.clone();
    tokio::spawn(async move {
        loop {
            ticker.set(Timestamp(start.elapsed().as_nanos() as u64 * SPEED));
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    });
    clock
}

fn setup() -> SimCameraSetup {
    let lens = Lens {
        position: 0,
        magnification: 10.0,
        numerical_aperture: 0.30,
        immersion_index: 1.0,
    };
    SimCameraSetup {
        sensor: (128, 96),
        pixel_format: PixelFormat::Mono14,
        frame_rate: Hertz(9.0),
        pixel_size: Micrometers(3.76),
        lenses: vec![lens],
        optics: OpticsModel::default(),
        sensor_model: SensorModel::default(),
        trigger_latency: Millis(0.1),
        mount: Affine2::IDENTITY,
    }
}

struct Rig {
    acquisition: Acquisition,
    stage: GrblStage,
    state: Arc<TrueState>,
    clock: Arc<ManualClock>,
}

async fn rig(slots: u16) -> Rig {
    let (clock, state) = (running_clock(), Arc::new(TrueState::new(1)));
    let world = Arc::new(Cells::new(CellsParams::default()));
    let camera = Arc::new(SimCamera::new(setup(), world, state.clone(), clock.clone()));
    let errors = GrblErrors::from(&SimErrors::default());
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
    let stage = GrblStage::connect(host, options, clock.clone())
        .await
        .unwrap();
    let pool = PoolConfig {
        name: "camera.pool".into(),
        slots,
        slot_bytes: 128 * 96 * 2,
        backend: Backend::Heap,
    };
    let acquisition = Acquisition::new(camera, &pool, clock.clone()).unwrap();
    acquisition.follow_stage(stage.status());
    Rig {
        acquisition,
        stage,
        state,
        clock,
    }
}

/// Reads `n` frames from `frames`, and gives back their metadata, so that their slots go back.
async fn take(frames: &mut Subscriber, n: usize) -> Vec<FrameMeta> {
    let mut taken = Vec::with_capacity(n);
    while taken.len() < n {
        let frame = tokio::time::timeout(Duration::from_secs(5), frames.recv()).await;
        let frame = frame
            .expect("a frame in time")
            .expect("the fan-out is open");
        taken.push(*frame.meta());
    }
    taken
}

#[tokio::test]
async fn a_still_frame_s_pose_is_the_stage_s_within_the_injected_error_with_its_context() {
    let rig = rig(4).await;
    let context = AcquisitionContext {
        channel: Some(ChannelId(2)),
        objective: Some(ObjectiveId(1)),
        calibration: Some(CalibrationId::generate()),
    };
    rig.acquisition.set_context(context);
    let target = Xyz::from_um(-2_000.0, -1_500.0, 0.0);
    rig.stage
        .move_to(target, MoveOpts::default())
        .await
        .unwrap();
    let frame = rig.acquisition.snap(true).await.unwrap();
    let meta = frame.meta();
    assert_eq!(meta.pose_quality, PoseQuality::Settled);
    assert!(!meta.flags.moving && meta.flags.save);
    let pose = meta.pose.unwrap();
    assert_eq!(
        (pose.x, pose.y),
        (target.x, target.y),
        "the stage's reading"
    );
    // The emulator injects 1 µm of half-backlash, 0.05 % of scale, and 0.5 µm of noise.
    let truth = rig.state.pose();
    for (reported, true_) in [(pose.x, truth.x), (pose.y, truth.y)] {
        let bound = 1.0 + 5e-4 * reported.0.abs() + 3.0 * 0.5;
        assert!(
            (reported.0 - true_.0).abs() <= bound,
            "{reported} vs {true_}"
        );
    }
    let stamped = (meta.channel, meta.objective, meta.calibration);
    assert_eq!(
        stamped,
        (context.channel, context.objective, context.calibration)
    );
}

/// Reads every frame `frames` gets, as it comes, and keeps their metadata.
fn collect(mut frames: Subscriber) -> Arc<Mutex<Vec<FrameMeta>>> {
    let metas = Arc::new(Mutex::new(Vec::new()));
    let kept = metas.clone();
    tokio::spawn(async move {
        while let Some(frame) = frames.recv().await {
            kept.lock().unwrap().push(*frame.meta());
        }
    });
    metas
}

/// Waits until `metas` holds `n` frames.
async fn until(metas: &Mutex<Vec<FrameMeta>>, n: usize) {
    let started = Instant::now();
    while metas.lock().unwrap().len() < n {
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "{n} frames in time"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

#[tokio::test]
async fn live_frames_while_moving_are_flagged_and_interpolated() {
    let rig = rig(6).await;
    let metas = collect(
        rig.acquisition
            .subscribe("test", Policy::Lossless, |_| true),
    );
    rig.acquisition.start_live().await.unwrap();
    until(&metas, 2).await;
    // 40 mm at 50 mm/s: about a second on the clock, so several frames.
    let far = Xyz::from_um(-40_000.0, 0.0, 0.0);
    rig.stage.move_to(far, MoveOpts::default()).await.unwrap();
    rig.acquisition.stop().await.unwrap();
    let metas = metas.lock().unwrap().clone();
    let moving: Vec<_> = metas.iter().filter(|meta| meta.flags.moving).collect();
    assert!(
        moving.len() >= 2,
        "{} of {} frames moving",
        moving.len(),
        metas.len()
    );
    // Each pose is within its uncertainty of the path, even one carried on past the end of the
    // move.
    for meta in moving {
        let x = meta.pose.unwrap().x.0;
        let PoseQuality::Interpolated { uncertainty } = meta.pose_quality else {
            panic!("{meta:?}")
        };
        let off_path = (x - 0.0).max(-40_000.0 - x).max(0.0);
        assert!(off_path <= uncertainty.0, "{meta:?}");
    }
}

#[tokio::test]
async fn frame_numbers_go_on_across_modes_and_reconfiguration() {
    let rig = rig(6).await;
    let acquisition = &rig.acquisition;
    let metas = collect(acquisition.subscribe("all", Policy::Lossless, |_| true));
    acquisition.start_live().await.unwrap();
    until(&metas, 3).await;
    let snapped = *acquisition.snap(false).await.unwrap().meta();
    assert_eq!(acquisition.stats().mode, Mode::Triggered);
    acquisition.start_live().await.unwrap();
    // A new region, while live, stops the camera, reconfigures it and starts it again.
    let roi = Roi {
        width: 64,
        height: 64,
        ..Roi::full(128, 96)
    };
    let full = CameraSettings::full_frame(&setup().capabilities(), Millis(10.0));
    acquisition
        .configure(CameraSettings { roi, ..full })
        .await
        .unwrap();
    assert_eq!(acquisition.stats().mode, Mode::Live);
    let so_far = metas.lock().unwrap().len();
    until(&metas, so_far + 3).await;
    acquisition.stop().await.unwrap();

    let seen = metas.lock().unwrap().clone();
    let numbers: Vec<u64> = seen.iter().map(|meta| meta.seq.0).collect();
    assert!(
        numbers.windows(2).all(|pair| pair[1] > pair[0]),
        "{numbers:?}"
    );
    assert!(
        numbers.contains(&snapped.seq.0),
        "the snap was published too"
    );
    // A gap is a lost frame, and only that.
    let lost = acquisition.stats().lost_no_slot;
    let span = numbers.last().unwrap() + 1 - numbers[0];
    assert_eq!(span, seen.len() as u64 + lost, "{numbers:?}");
    assert_eq!(seen.last().unwrap().roi.width, 64);
}

#[tokio::test]
async fn a_capture_marks_its_frames_and_lost_frames_are_counted() {
    let rig = rig(3).await;
    let acquisition = &rig.acquisition;
    let mut all = acquisition.subscribe("all", Policy::Lossless, |_| true);
    let mut captured = acquisition.capture_next(2, true);
    acquisition.start_live().await.unwrap();
    let mut next = async || {
        let frame = tokio::time::timeout(Duration::from_secs(5), captured.recv()).await;
        frame.expect("a captured frame in time").expect("open")
    };
    let (first, second) = (*next().await.meta(), *next().await.meta());
    assert!(first.flags.save && second.flags.save);
    assert_eq!(second.seq.0, first.seq.0 + 1);

    // A lossless consumer that doesn't read holds every slot: the camera loses frames.
    tokio::time::sleep(Duration::from_millis(300)).await;
    let stats = acquisition.stats();
    assert!(stats.lost_no_slot > 0, "{stats:?}");
    let (_, consumer) = stats
        .consumers
        .iter()
        .find(|(name, _)| name == "all")
        .unwrap();
    assert_eq!(consumer.max_queued, 3, "{stats:?}");
    // Reading again, the next frame comes after a gap of the frames lost meanwhile.
    let held = take(&mut all, 3).await;
    assert_eq!(held[0].seq, first.seq);
    let after = take(&mut all, 1).await.remove(0);
    let gap = after.seq.0 - held[2].seq.0 - 1;
    let lost = acquisition.stats().lost_no_slot;
    assert!(
        (1..=lost).contains(&gap),
        "a gap of {gap}, with {lost} lost"
    );
    assert!(!after.flags.save);
    acquisition.stop().await.unwrap();
}

#[tokio::test]
async fn a_frame_takes_the_context_in_force_at_its_mid_exposure() {
    let rig = rig(6).await;
    let acquisition = &rig.acquisition;
    let metas = collect(acquisition.subscribe("all", Policy::Lossless, |_| true));
    let (first, second) = (ChannelId(1), ChannelId(2));
    let channel = |channel| AcquisitionContext {
        channel: Some(channel),
        ..AcquisitionContext::default()
    };
    acquisition.set_context(channel(first));
    acquisition.start_live().await.unwrap();
    until(&metas, 2).await;
    // The frame being exposed now arrives after the change, and keeps the first channel.
    let before = rig.clock.now();
    acquisition.set_context(channel(second));
    let after = rig.clock.now();
    assert_eq!(acquisition.context().channel, Some(second));
    let so_far = metas.lock().unwrap().len();
    until(&metas, so_far + 3).await;
    acquisition.stop().await.unwrap();
    for meta in metas.lock().unwrap().iter() {
        let t = meta.host_ts_mid_exposure;
        if t < before {
            assert_eq!(meta.channel, Some(first), "{meta:?}");
        } else if t >= after {
            assert_eq!(meta.channel, Some(second), "{meta:?}");
        }
    }
}
