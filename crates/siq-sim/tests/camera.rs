//! The simulated camera on a manual clock: its rate, triggers, lost frames, settings, and the true
//! state it follows.

use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::Duration;

use siq_buffer::{Backend, FillGuard, FrameLease, FramePool, PoolConfig};
use siq_devices::*;
use siq_sim::camera::{Lens, SimCamera, SimCameraSetup};
use siq_sim::optics::{OpticsModel, SensorModel};
use siq_sim::state::TrueState;
use siq_synth::{Cells, CellsParams};
use siq_types::{
    Affine2, Binning, Clock, FrameMeta, Hertz, ManualClock, Micrometers, Millis, PixelFormat, Roi,
    StagePose, Timestamp,
};

fn setup() -> SimCameraSetup {
    SimCameraSetup {
        sensor: (256, 192),
        pixel_format: PixelFormat::Mono14,
        frame_rate: Hertz(9.0),
        pixel_size: Micrometers(3.76),
        lenses: vec![
            Lens {
                position: 0,
                magnification: 10.0,
                numerical_aperture: 0.30,
                immersion_index: 1.0,
            },
            Lens {
                position: 1,
                magnification: 20.0,
                numerical_aperture: 0.75,
                immersion_index: 1.0,
            },
        ],
        optics: OpticsModel::default(),
        sensor_model: SensorModel::default(),
        trigger_latency: Millis(0.1),
        mount: Affine2::IDENTITY,
    }
}

struct Rig {
    camera: SimCamera,
    clock: Arc<ManualClock>,
    state: Arc<TrueState>,
    frames: Arc<Frames>,
    pool: FramePool,
}

fn rig(slots: u16, keep: bool) -> Rig {
    let clock = Arc::new(ManualClock::new());
    let state = Arc::new(TrueState::new(1));
    let world = Arc::new(Cells::new(CellsParams {
        seed: 2,
        spacing: Micrometers(20.0),
        radius: [Micrometers(4.0), Micrometers(8.0)],
        ..CellsParams::default()
    }));
    let camera = SimCamera::new(setup(), world, state.clone(), clock.clone());
    let config = PoolConfig {
        name: "camera.pool".into(),
        slots,
        slot_bytes: 256 * 192 * 2,
        backend: Backend::Heap,
    };
    let pool = FramePool::new(&config, clock.clone(), None).unwrap();
    Rig {
        camera,
        clock,
        state,
        frames: Arc::new(Frames {
            keep,
            ..Frames::default()
        }),
        pool,
    }
}

/// Each frame's metadata and mean, and the leases kept.
type Seen = (Vec<(FrameMeta, f64)>, Vec<FrameLease>);

/// The frames a camera delivers: their metadata and mean, and their leases if kept.
#[derive(Default)]
struct Frames {
    keep: bool,
    seen: Mutex<Seen>,
    arrived: Condvar,
}

impl FrameSink for Frames {
    fn frame(&self, slot: FillGuard, meta: FrameMeta) {
        let lease = slot.publish(meta).expect("the frame fits");
        let image = lease.image::<u16>().expect("Mono16 words");
        let mean = image.rows().flatten().map(|&v| f64::from(v)).sum::<f64>()
            / (image.width() * image.height()) as f64;
        let mut seen = self.seen.lock().unwrap();
        seen.0.push((meta, mean));
        if self.keep {
            seen.1.push(lease);
        }
        self.arrived.notify_all();
    }
}

impl Frames {
    /// Waits, in real time, until `count` frames have arrived.
    fn wait_for(&self, count: usize) -> Vec<(FrameMeta, f64)> {
        let seen = self.seen.lock().unwrap();
        let (seen, _) = self
            .arrived
            .wait_timeout_while(seen, Duration::from_secs(3), |seen| seen.0.len() < count)
            .unwrap();
        let stamps: Vec<_> = seen
            .0
            .iter()
            .map(|(meta, _)| (meta.seq.0, ms(meta.host_ts_mid_exposure)))
            .collect();
        assert!(
            seen.0.len() >= count,
            "{count} frames due, {} arrived: {stamps:?}",
            seen.0.len()
        );
        seen.0.clone()
    }

    fn count(&self) -> usize {
        self.seen.lock().unwrap().0.len()
    }

    fn release(&self) {
        self.seen.lock().unwrap().1.clear();
    }
}

fn ms(t: Timestamp) -> f64 {
    t.0 as f64 / 1e6
}

/// The readout of `rows` rows on the test sensor: 1 ms, plus the line time that gives 9 frames/s
/// over 192 rows.
fn readout(rows: f64) -> f64 {
    1.0 + (1000.0 / 9.0 - 1.0) / 192.0 * rows
}

/// Advances the clock to `until` ms in 5 ms steps, waiting at each for the frames due by then.
fn run_to(rig: &Rig, until: f64, due: impl Fn(f64) -> usize) {
    while ms(rig.clock.now()) < until {
        rig.clock.advance(Duration::from_millis(5));
        rig.frames.wait_for(due(ms(rig.clock.now())));
    }
}

async fn settle(rig: &Rig, settings: CameraSettings) {
    rig.camera.configure(settings).await.unwrap();
    rig.camera
        .start(rig.pool.clone(), rig.frames.clone())
        .await
        .unwrap();
}

// REQ-SIM-001, REQ-ACQ-003, REQ-ACQ-006
#[tokio::test]
async fn free_running_frames_come_at_the_model_s_rate_for_the_full_frame_and_an_roi() {
    let rig = rig(4, false);
    let full = CameraSettings {
        exposure: Millis(1.0),
        ..rig.camera.settings()
    };
    settle(&rig, full).await;
    // Frame n starts at n × period and is ready an exposure and a readout later.
    let period = readout(192.0);
    let due = |now: f64| {
        if now < 1.0 + period {
            0
        } else {
            ((now - 1.0 - period) / period).floor() as usize + 1
        }
    };
    run_to(&rig, 10_000.0, due);
    let frames = rig.frames.wait_for(due(10_000.0));
    assert_eq!(frames.len(), 89);
    for (n, (meta, _)) in frames.iter().enumerate() {
        assert_eq!(meta.seq.0, n as u64);
        assert!(
            (ms(meta.host_ts_mid_exposure) - (n as f64 * period + 0.5)).abs() < 1e-3,
            "frame {n}"
        );
        assert_eq!(
            meta.device_ts.unwrap().0,
            (meta.host_ts_mid_exposure.0 / 1000),
            "the device counts µs from the start"
        );
    }
    let rate =
        88.0 / (ms(frames[88].0.host_ts_mid_exposure) - ms(frames[0].0.host_ts_mid_exposure)) * 1e3;
    assert!((rate - 9.0).abs() < 1e-3, "{rate} frames/s");
    rig.camera.stop().await.unwrap();

    // A 64-row ROI reads three times as fast.
    let roi = CameraSettings {
        roi: Roi {
            offset_x: 0,
            offset_y: 64,
            width: 256,
            height: 64,
        },
        ..full
    };
    let fast = CameraSettings::full_frame(rig.camera.capabilities(), Millis(1.0)).frame_bytes()
        >= roi.frame_bytes();
    assert!(fast);
    let before = rig.frames.count();
    settle(&rig, roi).await;
    let start = ms(rig.clock.now());
    let period = readout(64.0);
    run_to(&rig, start + 2_000.0, |now| {
        before
            + if now < start + 1.0 + period {
                0
            } else {
                ((now - start - 1.0 - period) / period).floor() as usize + 1
            }
    });
    let frames = rig.frames.wait_for(before + 2);
    let spacing =
        ms(frames[before + 1].0.host_ts_mid_exposure) - ms(frames[before].0.host_ts_mid_exposure);
    assert!(
        (spacing - period).abs() < 1e-3 && 1000.0 / spacing > 2.5 * 9.0,
        "{spacing} ms"
    );
    assert_eq!(frames[before].0.seq.0, 0, "numbers count from 0 again");
    rig.camera.stop().await.unwrap();
}

// REQ-SIM-001, REQ-ACQ-006
#[tokio::test]
async fn a_software_trigger_starts_an_exposure_after_its_latency() {
    let rig = rig(4, false);
    let software = CameraSettings {
        exposure: Millis(5.0),
        trigger: TriggerMode::Software,
        ..rig.camera.settings()
    };
    settle(&rig, software).await;
    rig.clock.set(Timestamp(1_000_000_000));
    rig.camera.trigger().await.unwrap();
    assert!(
        matches!(rig.camera.trigger().await, Err(DeviceError::Busy(_))),
        "one trigger at a time"
    );
    // Ready at 1000 + 0.1 latency + 5 exposure + the readout.
    let ready = 1000.0 + 0.1 + 5.0 + readout(192.0);
    rig.clock.set(Timestamp(((ready - 0.5) * 1e6) as u64));
    thread::sleep(Duration::from_millis(100));
    assert_eq!(rig.frames.count(), 0, "not before it is read out");
    rig.clock.set(Timestamp(((ready + 0.5) * 1e6) as u64));
    let frames = rig.frames.wait_for(1);
    assert!(
        (ms(frames[0].0.host_ts_mid_exposure) - 1002.6).abs() < 1e-3,
        "{:?}",
        frames[0].0
    );
    rig.camera.stop().await.unwrap();

    let free = CameraSettings {
        trigger: TriggerMode::FreeRun,
        ..software
    };
    rig.camera.configure(free).await.unwrap();
    assert!(matches!(
        rig.camera.trigger().await,
        Err(DeviceError::InvalidArgument(_))
    ));
}

// REQ-ACQ-005
#[tokio::test]
async fn frames_are_lost_and_counted_while_consumers_hold_every_slot() {
    let rig = rig(2, true);
    settle(
        &rig,
        CameraSettings {
            exposure: Millis(1.0),
            ..rig.camera.settings()
        },
    )
    .await;
    let period = readout(192.0);
    // Ten frames are due by 1.2 s; only two slots exist, and both leases are kept.
    // Step to 1.2 s, waiting at each step for every frame due, delivered or lost.
    let due = |now: f64| {
        if now < 1.0 + period {
            0
        } else {
            ((now - 1.0 - period) / period).floor() as u64 + 1
        }
    };
    while ms(rig.clock.now()) < 1200.0 {
        rig.clock.advance(Duration::from_millis(5));
        let (expected, waited) = (due(ms(rig.clock.now())), std::time::Instant::now());
        while rig.camera.stats().delivered + rig.camera.stats().lost_no_slot < expected
            && waited.elapsed() < Duration::from_secs(10)
        {
            thread::sleep(Duration::from_millis(1));
        }
    }
    let due = due(1200.0);
    assert_eq!(
        rig.camera.stats(),
        CameraStats {
            delivered: 2,
            lost_no_slot: due - 2,
            lost_device: 0
        }
    );
    rig.frames.release();
    rig.clock.advance(Duration::from_millis(200));
    let frames = rig.frames.wait_for(3);
    assert_eq!(
        frames[2].0.seq.0, due,
        "the lost frames' numbers are skipped"
    );
    rig.camera.stop().await.unwrap();
}

// REQ-SIM-001
#[tokio::test]
async fn a_settings_change_takes_effect_at_the_next_frame() {
    let rig = rig(4, false);
    let short = CameraSettings {
        exposure: Millis(1.0),
        ..rig.camera.settings()
    };
    settle(&rig, short).await;
    rig.clock.advance(Duration::from_millis(115));
    rig.frames.wait_for(1);
    // The camera has begun frame 1 with the old settings; frame 2 takes the new ones.
    thread::sleep(Duration::from_millis(50));
    rig.camera
        .configure(CameraSettings {
            exposure: Millis(4.0),
            ..short
        })
        .await
        .unwrap();
    let binned = CameraSettings {
        binning: Binning::new(2, 2).unwrap(),
        ..short
    };
    assert!(matches!(
        rig.camera.configure(binned).await,
        Err(DeviceError::Busy(_))
    ));
    // Frames 1 and 2 are ready at 223.2 and 337.3 ms.
    run_to(&rig, 345.0, |now| {
        1 + usize::from(now >= 224.0) + usize::from(now >= 338.0)
    });
    let frames = rig.frames.wait_for(3);
    let exposures: Vec<f64> = frames[..3]
        .iter()
        .map(|(meta, _)| meta.exposure.0)
        .collect();
    assert_eq!(exposures, [1.0, 1.0, 4.0]);
    let (dim, bright) = (frames[1].1 - 100.0, frames[2].1 - 100.0);
    assert!(
        (bright / dim - 4.0).abs() < 0.2,
        "four times the light: {dim} and {bright} ADU over the offset"
    );
    rig.camera.stop().await.unwrap();
}

// REQ-SIM-001
#[tokio::test]
async fn the_camera_sees_the_true_state() {
    let rig = rig(4, false);
    let trigger = CameraSettings {
        exposure: Millis(2.0),
        trigger: TriggerMode::Software,
        ..rig.camera.settings()
    };
    settle(&rig, trigger).await;
    let snap = |count: usize| {
        rig.camera_trigger_now();
        rig.clock.advance(Duration::from_millis(200));
        rig.frames.wait_for(count)[count - 1].1
    };
    let lit = snap(1);
    rig.state.set_level(0, 0.0);
    let dark = snap(2);
    assert!(
        lit > 500.0 && (dark - 100.0).abs() < 5.0,
        "lit {lit}, dark {dark}: the light went out"
    );
    rig.state.set_level(0, 1.0);
    rig.state.set_pose(StagePose {
        x: Micrometers(500.0),
        ..StagePose::default()
    });
    let moved = snap(3);
    assert!(
        (moved - lit).abs() > 1.0,
        "another place, another image: {lit} and {moved}"
    );
    rig.camera.stop().await.unwrap();
}

impl Rig {
    fn camera_trigger_now(&self) {
        futures_block(self.camera.trigger()).unwrap();
    }
}

/// Runs a future that is already ready.
fn futures_block<T>(future: BoxFuture<'_, T>) -> T {
    let waker = std::task::Waker::noop();
    let mut future = future;
    match future
        .as_mut()
        .poll(&mut std::task::Context::from_waker(waker))
    {
        std::task::Poll::Ready(value) => value,
        std::task::Poll::Pending => panic!("the trigger resolves at once"),
    }
}

#[test]
fn a_pose_is_never_seen_half_written() {
    let state = Arc::new(TrueState::new(2));
    let writer = {
        let state = state.clone();
        thread::spawn(move || {
            for k in 0..200_000 {
                let v = Micrometers(f64::from(k));
                state.set_pose(StagePose {
                    x: v,
                    y: v,
                    z_coarse: v,
                    focus: v,
                });
            }
        })
    };
    let readers: Vec<_> = (0..3)
        .map(|_| {
            let state = state.clone();
            thread::spawn(move || {
                for _ in 0..200_000 {
                    let pose = state.pose();
                    assert!(
                        pose.x == pose.y && pose.y == pose.z_coarse && pose.z_coarse == pose.focus,
                        "{pose:?}"
                    );
                }
            })
        })
        .collect();
    writer.join().unwrap();
    readers
        .into_iter()
        .for_each(|reader| reader.join().unwrap());

    state.set_stage(Micrometers(1.0), Micrometers(2.0), Micrometers(3.0));
    state.set_focus(Micrometers(4.0));
    assert_eq!(
        state.pose(),
        StagePose {
            x: Micrometers(1.0),
            y: Micrometers(2.0),
            z_coarse: Micrometers(3.0),
            focus: Micrometers(4.0)
        }
    );
    state.set_level(1, 0.25);
    assert_eq!((state.light(), state.level(5)), (1.25, 0.0));
    state.set_filter(Some(3));
    state.set_objective(None);
    assert_eq!((state.filter(), state.objective()), (Some(3), None));
    let structured = Modality {
        mode: ModalityMode::StructuredIllumination,
        pattern: Some(Pattern {
            orientation: 2,
            phase: 4,
        }),
    };
    state.set_modality(structured);
    assert_eq!(state.modality(), structured);
    assert_eq!(
        TrueState::new(0).light(),
        1.0,
        "no simulated light: full light"
    );
}
