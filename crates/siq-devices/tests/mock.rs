//! The mocks behave as drivers do, at once, and status watches carry the latest status.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use siq_buffer::{Backend, FillGuard, FrameLease, FramePool, PoolConfig};
use siq_devices::mock::*;
use siq_devices::*;
use siq_types::{
    Binning, Clock, FrameMeta, ManualClock, Micrometers, MicrometersPerSecond, Millis, ModalityMode,
};

fn clock() -> Arc<ManualClock> {
    Arc::new(ManualClock::new())
}

fn at(x: f64, y: f64, z: f64) -> Xyz {
    Xyz::from_um(x, y, z)
}

// REQ-STG-003
#[tokio::test]
async fn a_mock_stage_moves_homes_jogs_and_stops_and_refuses_what_a_driver_would() {
    let clock = clock();
    let travel = Travel {
        min: at(-1000.0, -1000.0, -100.0),
        max: at(1000.0, 1000.0, 0.0),
    };
    let stage = MockStage::new(travel, clock.clone());
    let direct = MoveOpts::default();
    assert!(!stage.status().borrow().homed);

    clock.advance(Duration::from_millis(5));
    let homed = stage.home().await.unwrap();
    assert!(homed.homed && homed.is_idle());
    assert_eq!((homed.position, homed.ts), (Xyz::default(), clock.now()));

    let moved = stage
        .move_to(at(100.0, -50.0, -10.0), direct)
        .await
        .unwrap();
    assert_eq!(moved.position, at(100.0, -50.0, -10.0));
    assert!(moved.homed, "a move keeps the reference");
    let opts = MoveOpts {
        speed: Some(MicrometersPerSecond(500.0)),
        approach: Approach::Unidirectional,
    };
    let by = stage.move_by(at(1.0, 1.0, 0.0), opts).await.unwrap();
    assert_eq!(by.position, at(101.0, -49.0, -10.0));

    let refused = stage.move_to(at(0.0, 0.0, 1.0), direct).await;
    assert!(
        matches!(refused, Err(DeviceError::InvalidArgument(_))),
        "{refused:?}"
    );
    assert_eq!(
        stage.status().borrow().position,
        by.position,
        "a refused move stays put"
    );
    assert_eq!(stage.limits(), travel);

    let slow = MicrometersPerSecond(100.0);
    let zero = MicrometersPerSecond::ZERO;
    stage.jog(Xyz::new(slow, zero, zero)).await.unwrap();
    assert_eq!(stage.status().borrow().state, MotionState::Jogging);
    stage.stop().await.unwrap();
    assert_eq!(stage.status().borrow().state, MotionState::Idle);

    stage.fail_next(DeviceError::Alarm(2));
    let origin = Xyz::default();
    assert_eq!(
        stage.move_to(origin, direct).await,
        Err(DeviceError::Alarm(2))
    );
    assert!(
        stage.move_to(origin, direct).await.is_ok(),
        "only the next one fails"
    );
}

// REQ-STG-003
#[tokio::test]
async fn a_status_watch_starts_at_the_latest_status_and_wakes_for_the_next() {
    let clock = clock();
    let travel = Travel {
        min: Micrometers(-100.0),
        max: Micrometers(100.0),
    };
    let focus = MockFocusDrive::new(travel, clock.clone());
    let direct = MoveOpts::default();
    let mut status = focus.status();
    assert_eq!(status.borrow().position, Micrometers(0.0));
    assert!(
        !status.has_changed().unwrap(),
        "a new receiver has seen the latest status"
    );

    // A waiting reader wakes for the change, and sees when the position was true.
    let waiting = tokio::spawn(async move {
        status.changed().await.unwrap();
        *status.borrow_and_update()
    });
    tokio::task::yield_now().await;
    clock.advance(Duration::from_millis(10));
    focus.move_to(Micrometers(12.5), direct).await.unwrap();
    let seen = waiting.await.unwrap();
    assert_eq!((seen.position, seen.ts), (Micrometers(12.5), clock.now()));

    // Changes a reader misses collapse into the latest.
    let mut late = focus.status();
    for target in [1.0, 2.0, 3.0] {
        focus.move_to(Micrometers(target), direct).await.unwrap();
    }
    assert!(late.has_changed().unwrap());
    assert_eq!(late.borrow_and_update().position, Micrometers(3.0));
    assert!(!late.has_changed().unwrap());
    assert_eq!(focus.status().borrow().position, Micrometers(3.0));

    // When the device goes, the stream ends.
    drop(focus);
    assert!(late.changed().await.is_err());
}

/// Publishes the frames a camera hands over, as the acquisition engine does, and keeps them.
#[derive(Default)]
struct Keep(Mutex<Vec<FrameLease>>);

impl FrameSink for Keep {
    fn frame(&self, slot: FillGuard, meta: FrameMeta) {
        let lease = slot.publish(meta).expect("the frame fits");
        self.0.lock().unwrap().push(lease);
    }
}

impl Keep {
    fn metas(&self) -> Vec<FrameMeta> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .map(|lease| *lease.meta())
            .collect()
    }
}

fn pool(slots: u16, slot_bytes: usize) -> FramePool {
    let config = PoolConfig {
        name: "test.pool".into(),
        slots,
        slot_bytes,
        backend: Backend::Heap,
    };
    FramePool::new(&config, clock(), None).unwrap()
}

// REQ-ACQ-005
#[tokio::test]
async fn a_mock_camera_delivers_a_frame_per_trigger_and_counts_what_it_loses() {
    let clock = clock();
    let camera = MockCamera::new(small_sensor(), clock.clone());
    let frames = Arc::new(Keep::default());
    let settings = CameraSettings {
        trigger: TriggerMode::Software,
        ..camera.settings()
    };
    camera.configure(settings).await.unwrap();
    let not_started = camera.trigger().await;
    assert!(matches!(not_started, Err(DeviceError::InvalidArgument(_))));

    let too_small = camera.start(pool(2, 64), frames.clone()).await;
    assert!(matches!(too_small, Err(DeviceError::InvalidArgument(_))));
    let pool = pool(2, settings.frame_bytes());
    camera.start(pool.clone(), frames.clone()).await.unwrap();
    let again = camera.start(pool.clone(), frames.clone()).await;
    assert!(matches!(again, Err(DeviceError::Busy(_))));

    clock.advance(Duration::from_millis(7));
    for _ in 0..3 {
        camera.trigger().await.unwrap(); // the third finds both slots held
    }
    camera.lose_frame();
    let metas = frames.metas();
    assert_eq!(
        metas.iter().map(|meta| meta.seq.0).collect::<Vec<_>>(),
        [0, 1]
    );
    assert_eq!(metas[0].host_ts_mid_exposure, clock.now());
    assert_eq!(
        (metas[0].exposure, metas[0].roi),
        (settings.exposure, settings.roi)
    );
    let second = frames.0.lock().unwrap()[1].clone();
    let bytes = second.as_bytes().iter().take(settings.frame_bytes());
    assert!(
        bytes.into_iter().all(|&byte| byte == 1),
        "a frame's bytes are its number"
    );
    drop(second);
    let lost = CameraStats {
        delivered: 2,
        lost_no_slot: 1,
        lost_device: 1,
    };
    assert_eq!(camera.stats(), lost);

    frames.0.lock().unwrap().clear();
    camera.trigger().await.unwrap();
    assert_eq!(
        frames.metas()[0].seq.0,
        4,
        "lost frames leave a gap in the numbers"
    );
    camera.stop().await.unwrap();
    assert_eq!(pool.stats().leased, 1, "the frames kept are still valid");
}

// REQ-ACQ-003
#[tokio::test]
async fn while_acquiring_only_the_exposure_and_gain_change_from_the_next_frame() {
    let camera = MockCamera::new(small_sensor(), clock());
    let frames = Arc::new(Keep::default());
    let pool = pool(4, camera.settings().frame_bytes());
    camera.start(pool, frames.clone()).await.unwrap();
    camera.emit().unwrap();
    let longer = CameraSettings {
        exposure: Millis(25.0),
        gain: 2.0,
        ..camera.settings()
    };
    camera.configure(longer).await.unwrap();
    camera.emit().unwrap();
    let binned = CameraSettings {
        binning: Binning::new(2, 2).unwrap(),
        ..longer
    };
    assert!(matches!(
        camera.configure(binned).await,
        Err(DeviceError::Busy(_))
    ));
    let free_run = camera.trigger().await;
    assert!(
        matches!(free_run, Err(DeviceError::InvalidArgument(_))),
        "no trigger in free-run"
    );
    let exposures: Vec<_> = frames
        .metas()
        .iter()
        .map(|m| (m.exposure, m.gain))
        .collect();
    assert_eq!(exposures, [(Millis(10.0), 1.0), (Millis(25.0), 2.0)]);

    camera.stop().await.unwrap();
    camera.configure(binned).await.unwrap();
    assert_eq!(camera.settings().image_size(), (32, 24));
    assert!(camera.emit().is_err(), "a stopped camera takes no frames");
}

// REQ-DEV-001
#[tokio::test]
async fn mock_peripherals_change_at_once_and_refuse_what_they_cannot_do() {
    let light = MockIllumination::new(vec!["white".into(), "470nm".into()]);
    let mut levels = light.status();
    light.set_level(1, 0.75).await.unwrap();
    assert!(levels.has_changed().unwrap());
    assert_eq!(levels.borrow_and_update().levels, [0.0, 0.75]);
    for (source, level) in [(2, 0.5), (0, 1.5), (0, f64::NAN)] {
        assert!(
            light.set_level(source, level).await.is_err(),
            "{source}: {level}"
        );
    }

    let wheel = MockChanger::new(vec!["empty".into(), "GFP".into()]);
    assert_eq!(wheel.status().borrow().position, None);
    wheel.select(1).await.unwrap();
    let selected = ChangerStatus {
        position: Some(1),
        moving: false,
    };
    assert_eq!(*wheel.status().borrow(), selected);
    assert!(wheel.select(2).await.is_err());
    wheel.fail_next(DeviceError::Timeout("the wheel didn't arrive".into()));
    assert!(matches!(
        wheel.select(0).await,
        Err(DeviceError::Timeout(_))
    ));

    let modes = vec![
        ModalityMode::Widefield,
        ModalityMode::StructuredIllumination,
    ];
    let patterns = PatternSet {
        orientations: 3,
        phases: 3,
    };
    let unit = MockModalityUnit::new(modes, Some(patterns));
    assert_eq!(unit.status().borrow().modality, Modality::WIDEFIELD);
    let structured = Modality {
        mode: ModalityMode::StructuredIllumination,
        pattern: Some(Pattern {
            orientation: 1,
            phase: 2,
        }),
    };
    unit.set_modality(structured).await.unwrap();
    assert_eq!(unit.status().borrow().modality, structured);
    let light_field = Modality {
        mode: ModalityMode::LightField,
        pattern: None,
    };
    assert!(unit.set_modality(light_field).await.is_err());
    assert_eq!(
        unit.status().borrow().modality,
        structured,
        "a refusal changes nothing"
    );
}
