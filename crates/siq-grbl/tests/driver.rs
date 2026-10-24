//! The driver against the GRBL emulator, over an in-memory link, on a clock that runs ten times
//! faster than real time; and against a controller scripted by hand, for what the emulator
//! doesn't do: grblHAL, work positions, silence and hard alarms.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use siq_config::model::{GrblMotion, SimStageConfig};
use siq_devices::{Approach, Device, DeviceError, MotionState, MoveOpts, Stage, Xyz};
use siq_grbl::{GrblOptions, GrblStage};
use siq_serial::{DuplexTransport, Framing, duplex};
use siq_sim::grbl::{GrblEmulator, GrblErrors, GrblSettings};
use siq_sim::state::TrueState;
use siq_types::{Clock, ManualClock, Micrometers, MicrometersPerSecond, Timestamp};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc;

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

fn options(motion: GrblMotion) -> GrblOptions {
    GrblOptions {
        motion,
        ..GrblOptions::from(&SimStageConfig::default())
    }
}

struct Rig {
    stage: GrblStage,
    state: Arc<TrueState>,
    clock: Arc<ManualClock>,
}

async fn emulated(errors: GrblErrors, motion: GrblMotion) -> Rig {
    let (clock, state) = (running_clock(), Arc::new(TrueState::new(0)));
    let (host, device) = duplex(Framing::default(), "grbl");
    let emulator = GrblEmulator::new(
        GrblSettings::default(),
        errors,
        1,
        state.clone(),
        clock.clone(),
    );
    tokio::spawn(emulator.run(device));
    let stage = GrblStage::connect(host, options(motion), clock.clone()).await;
    Rig {
        stage: stage.unwrap(),
        state,
        clock,
    }
}

impl Rig {
    /// Where the true stage is, in µm.
    fn truth(&self) -> [f64; 3] {
        let pose = self.state.pose();
        [pose.x.0, pose.y.0, pose.z_coarse.0]
    }
}

const fn at(x: f64, y: f64, z: f64) -> Xyz {
    Xyz::from_um(x, y, z)
}

fn speeds(x: f64, y: f64, z: f64) -> Xyz<MicrometersPerSecond> {
    Xyz::new(
        MicrometersPerSecond(x),
        MicrometersPerSecond(y),
        MicrometersPerSecond(z),
    )
}

#[track_caller]
fn assert_near(actual: Xyz, expected: Xyz, tolerance: f64) {
    let error = [
        actual.x.0 - expected.x.0,
        actual.y.0 - expected.y.0,
        actual.z.0 - expected.z.0,
    ];
    let close = error.iter().all(|e| e.abs() <= tolerance);
    assert!(
        close,
        "{actual:?} isn't within {tolerance} µm of {expected:?}"
    );
}

const DIRECT: MoveOpts = MoveOpts {
    speed: None,
    approach: Approach::Direct,
};

#[tokio::test]
async fn connecting_unlocks_reads_the_travel_and_publishes_the_status() {
    let rig = emulated(GrblErrors::default(), GrblMotion::default()).await;
    let status = *rig.stage.status().borrow();
    assert_eq!((status.state, status.homed), (MotionState::Idle, false));
    assert_eq!(status.position, at(0.0, 0.0, 0.0));
    let limits = rig.stage.limits();
    assert_eq!(limits.min, at(-100_000.0, -75_000.0, -20_000.0));
    assert_eq!(limits.max, at(0.0, 0.0, 0.0));
    let info = rig.stage.info();
    assert_eq!(info.vendor, "GRBL");
    assert_eq!(info.firmware.as_deref(), Some("Grbl 1.1h ['$' for help]"));

    // Told not to unlock, the driver leaves the controller locked until it is homed.
    let motion = GrblMotion {
        unlock: false,
        ..GrblMotion::default()
    };
    let locked = emulated(GrblErrors::default(), motion).await;
    assert_eq!(locked.stage.status().borrow().state, MotionState::Alarm(0));
    let refused = locked.stage.move_to(at(-1_000.0, 0.0, 0.0), DIRECT).await;
    assert!(matches!(refused, Err(DeviceError::Busy(_))), "{refused:?}");
    let homed = locked.stage.home().await.unwrap();
    assert_eq!((homed.state, homed.homed), (MotionState::Idle, true));
    let moved = locked.stage.move_to(at(-1_000.0, 0.0, 0.0), DIRECT).await;
    assert_near(moved.unwrap().position, at(-1_000.0, 0.0, 0.0), 0.0);
}

#[tokio::test]
async fn a_move_returns_once_the_stage_is_still_at_its_target() {
    let rig = emulated(GrblErrors::default(), GrblMotion::default()).await;
    let (start, target) = (rig.clock.now(), at(-20_000.0, -10_000.0, -1_000.0));
    let status = rig.stage.move_to(target, DIRECT).await.unwrap();
    assert_eq!(status.state, MotionState::Idle);
    assert_near(status.position, target, 0.5);
    let [x, y, z] = rig.truth();
    assert_near(at(x, y, z), target, 0.5);
    // X limits it: 20 mm at 50 mm/s, with ramps of 0.25 s at 200 mm/s², is 0.65 s.
    let took = rig.clock.now().saturating_since(start).as_secs_f64();
    assert!((0.65..1.5).contains(&took), "{took} s");

    // A relative move, at a set speed: 5 mm at 5 mm/s.
    let start = rig.clock.now();
    let slow = MoveOpts {
        speed: Some(MicrometersPerSecond(5_000.0)),
        ..DIRECT
    };
    let status = rig
        .stage
        .move_by(at(5_000.0, 0.0, 0.0), slow)
        .await
        .unwrap();
    assert_near(status.position, at(-15_000.0, -10_000.0, -1_000.0), 0.5);
    let took = rig.clock.now().saturating_since(start).as_secs_f64();
    assert!((1.0..2.0).contains(&took), "{took} s");

    // A target outside the travel isn't sent.
    let outside = rig.stage.move_to(at(1_000.0, 0.0, 0.0), DIRECT).await;
    assert!(
        matches!(outside, Err(DeviceError::InvalidArgument(_))),
        "{outside:?}"
    );
    assert_eq!(rig.stage.status().borrow().state, MotionState::Idle);
}

#[tokio::test]
async fn a_unidirectional_move_arrives_from_the_configured_side() {
    let errors = GrblErrors {
        backlash: [Micrometers(8.0); 3],
        ..GrblErrors::default()
    };
    let rig = emulated(errors, GrblMotion::default()).await;
    let (below, above) = (at(-20_000.0, -20_000.0, -2_000.0), at(0.0, 0.0, 0.0));
    let target = at(-10_000.0, -10_000.0, -1_000.0);
    let one_way = MoveOpts {
        approach: Approach::Unidirectional,
        ..DIRECT
    };
    let [x, y, z] = rig.truth();
    assert_eq!([x, y, z], [0.0; 3]);

    // Directly, the true stage lands a backlash apart from either side.
    rig.stage.move_to(below, DIRECT).await.unwrap();
    rig.stage.move_to(target, DIRECT).await.unwrap();
    let from_below = rig.truth();
    rig.stage.move_to(above, DIRECT).await.unwrap();
    rig.stage.move_to(target, DIRECT).await.unwrap();
    let from_above = rig.truth();
    for axis in 0..3 {
        assert!((from_above[axis] - from_below[axis] - 8.0).abs() < 1e-6);
    }

    // Unidirectionally, it lands as it did from below, the configured side, from either side.
    for start in [below, above] {
        rig.stage.move_to(start, DIRECT).await.unwrap();
        let status = rig.stage.move_to(target, one_way).await.unwrap();
        assert_near(status.position, target, 0.5);
        assert_eq!(rig.truth(), from_below);
    }

    // An axis that last arrived the wrong way goes round again, even if it doesn't move.
    rig.stage
        .move_to(at(-5_000.0, -20_000.0, -2_000.0), DIRECT)
        .await
        .unwrap();
    rig.stage.move_to(target, DIRECT).await.unwrap();
    assert_ne!(rig.truth(), from_below);
    rig.stage.move_to(target, one_way).await.unwrap();
    assert_eq!(rig.truth(), from_below);
}

#[tokio::test]
async fn stop_halts_a_move_and_keeps_the_position() {
    let rig = emulated(GrblErrors::default(), GrblMotion::default()).await;
    rig.stage.home().await.unwrap();
    let moving = rig.stage.move_to(at(-90_000.0, 0.0, 0.0), DIRECT);
    let stopping = async {
        let mut status = rig.stage.status();
        let far = |s: &siq_devices::StageStatus| s.position.x.0 < -10_000.0;
        status.wait_for(far).await.unwrap();
        rig.stage.stop().await
    };
    let (moved, stopped) = tokio::join!(moving, stopping);
    assert!(matches!(moved, Err(DeviceError::Stopped)), "{moved:?}");
    stopped.unwrap();

    // A hold, then a reset once still, keeps the position, and the stage is still homed.
    let status = *rig.stage.status().borrow();
    assert_eq!((status.state, status.homed), (MotionState::Idle, true));
    assert!(
        (-90_000.0..-10_000.0).contains(&status.position.x.0),
        "{status:?}"
    );
    // The controller reports whole steps of 0.625 µm, printed to 1 µm; the emulator holds
    // between steps.
    let [x, y, z] = rig.truth();
    assert_near(at(x, y, z), status.position, 1.0);
    let moved = rig.stage.move_by(at(1_000.0, 0.0, 0.0), DIRECT).await;
    let expected = status.position + at(1_000.0, 0.0, 0.0);
    assert_near(moved.unwrap().position, expected, 0.5);
}

#[tokio::test]
async fn a_jog_runs_until_it_is_cancelled_or_reaches_the_travel() {
    let rig = emulated(GrblErrors::default(), GrblMotion::default()).await;
    let mut status = rig.stage.status();
    rig.stage
        .jog(speeds(-5_000.0, -2_000.0, 0.0))
        .await
        .unwrap();
    let jogging = |s: &siq_devices::StageStatus| s.state == MotionState::Jogging;
    let far = |s: &siq_devices::StageStatus| jogging(s) && s.position.x.0 < -2_000.0;
    status.wait_for(far).await.unwrap();
    rig.stage.jog(speeds(0.0, 0.0, 0.0)).await.unwrap();
    let still = *status
        .wait_for(|s| s.state == MotionState::Idle)
        .await
        .unwrap();
    assert!(
        (still.position.y.0 / still.position.x.0 - 0.4).abs() < 0.01,
        "{still:?}"
    );

    // A move after a jog starts from where the jog stopped.
    let status = rig
        .stage
        .move_by(at(1_000.0, 0.0, 0.0), DIRECT)
        .await
        .unwrap();
    assert_near(status.position, still.position + at(1_000.0, 0.0, 0.0), 0.5);

    // A jog that isn't cancelled stops at the edge of the travel, without an alarm.
    rig.stage.jog(speeds(0.0, 0.0, -10_000.0)).await.unwrap();
    let mut status = rig.stage.status();
    let end = |s: &siq_devices::StageStatus| s.state == MotionState::Idle && s.position.z.0 < 0.0;
    let end = *status.wait_for(end).await.unwrap();
    assert_eq!(end.position.z, Micrometers(-20_000.0));
}

/// A controller scripted by the test. It answers `?` with the report it holds, a reset with
/// `banner`, and each line with what `reply` gives, which may change the report.
fn scripted(
    banner: &'static str,
    mut reply: impl FnMut(&str, &mut String) -> Vec<String> + Send + 'static,
) -> (DuplexTransport, mpsc::UnboundedSender<String>) {
    let (host, mut device) = duplex(Framing::default(), "scripted");
    let (inject, mut injected) = mpsc::unbounded_channel::<String>();
    tokio::spawn(async move {
        let mut report = "<Idle|WPos:1.000,2.000,3.000|WCO:-11.000,-12.000,-13.000>".to_string();
        let (mut line, mut buffer) = (Vec::new(), [0u8; 256]);
        let mut out = vec![String::new(), banner.to_string()];
        loop {
            for text in out.drain(..) {
                device
                    .write_all(format!("{text}\r\n").as_bytes())
                    .await
                    .unwrap();
            }
            tokio::select! {
                read = device.read(&mut buffer) => {
                    let count = read.unwrap_or(0);
                    if count == 0 {
                        return;
                    }
                    for &byte in &buffer[..count] {
                        match byte {
                            b'?' => out.push(report.clone()),
                            0x18 => out.extend([String::new(), banner.to_string()]),
                            b'\n' => {
                                let text = String::from_utf8(std::mem::take(&mut line)).unwrap();
                                out.extend(reply(&text, &mut report));
                            }
                            b'!' | b'~' | b'\r' => {}
                            byte if byte.is_ascii() => line.push(byte),
                            _ => {}
                        }
                    }
                }
                Some(text) = injected.recv() => out.push(text),
            }
        }
    });
    (host, inject)
}

/// The `$$` listing of a stage like the emulator's.
fn listing() -> Vec<String> {
    let settings = [(25, 1000.0), (110, 3000.0), (111, 3000.0), (112, 600.0)];
    let more = [(120, 200.0), (121, 200.0), (122, 100.0)];
    let travel = [(130, 100.0), (131, 75.0), (132, 20.0)];
    let all = settings.iter().chain(&more).chain(&travel);
    let mut lines: Vec<String> = all.map(|(n, v)| format!("${n}={v:.3}")).collect();
    lines.push("ok".into());
    lines
}

#[tokio::test]
async fn a_reply_that_never_comes_times_out_and_answers_nothing_when_it_comes_late() {
    let (mut moves, mut dwells) = (0, 0);
    let (host, _inject) = scripted("GrblHAL 1.1f ['$' or '$HELP' for help]", move |line, _| {
        let replies: &[&str] = match line {
            "$$" => return listing(),
            _ if line.starts_with("G0") => {
                moves += 1;
                // The second move's line first gets the late reply to the dwell before it.
                match moves {
                    2 => &["ok", "error:20"],
                    _ => &["ok"],
                }
            }
            "G4 P0" => {
                dwells += 1;
                if dwells == 1 { &[] } else { &["ok"] }
            }
            _ => &["ok"],
        };
        replies.iter().map(|reply| reply.to_string()).collect()
    });
    let clock = running_clock();
    let stage = GrblStage::connect(host, options(GrblMotion::default()), clock.clone());
    let stage = stage.await.unwrap();
    assert_eq!(stage.info().vendor, "grblHAL");
    // A work position is made a machine one with the work offset.
    let status = *stage.status().borrow();
    assert_near(status.position, at(-10_000.0, -10_000.0, -10_000.0), 1e-9);

    // The first dwell isn't answered: the move fails within its allowance on the clock.
    let start = clock.now();
    let silent = stage
        .move_to(at(-5_000.0, -5_000.0, -5_000.0), DIRECT)
        .await;
    assert!(matches!(silent, Err(DeviceError::Timeout(_))), "{silent:?}");
    let waited = clock.now().saturating_since(start).as_secs_f64();
    assert!((2.0..10.0).contains(&waited), "{waited} s");

    // The late `ok` doesn't answer the next line, whose own reply is an error.
    let next = stage
        .move_to(at(-6_000.0, -5_000.0, -5_000.0), DIRECT)
        .await;
    assert!(
        matches!(&next, Err(DeviceError::InvalidArgument(m)) if m.contains("20")),
        "{next:?}"
    );
    stage
        .move_to(at(-7_000.0, -5_000.0, -5_000.0), DIRECT)
        .await
        .unwrap();
}

#[tokio::test]
async fn an_alarm_fails_the_move_and_loses_the_reference_until_a_stop_recovers() {
    let homing = Arc::new(Mutex::new(0));
    let homes = homing.clone();
    let (host, _inject) = scripted("Grbl 1.1h ['$' for help]", move |line, report| {
        let replies: Vec<&str> = match line {
            "$$" => return listing(),
            "$H" => {
                *homes.lock().unwrap() += 1;
                vec!["ok"]
            }
            // A hard limit, while the move runs.
            "G4 P0" => {
                *report = "<Alarm|MPos:-3.000,0.000,0.000|FS:0,0>".into();
                vec!["ALARM:1"]
            }
            "$X" => {
                *report = "<Idle|MPos:-3.000,0.000,0.000|FS:0,0>".into();
                vec!["[MSG:Caution: Unlocked]", "ok"]
            }
            _ => vec!["ok"],
        };
        replies.into_iter().map(String::from).collect()
    });
    let stage = GrblStage::connect(host, options(GrblMotion::default()), running_clock());
    let stage = stage.await.unwrap();
    assert!(stage.home().await.unwrap().homed);
    assert_eq!(*homing.lock().unwrap(), 1);

    let alarmed = stage.move_to(at(-5_000.0, 0.0, 0.0), DIRECT).await;
    assert!(matches!(alarmed, Err(DeviceError::Alarm(1))), "{alarmed:?}");
    let mut status = stage.status();
    let alarm = *status
        .wait_for(|s| s.state == MotionState::Alarm(1))
        .await
        .unwrap();
    assert!(!alarm.homed);

    // A stop resets the controller, unlocks it and takes the position as it is, unreferenced.
    stage.stop().await.unwrap();
    let status = *stage.status().borrow();
    assert_eq!((status.state, status.homed), (MotionState::Idle, false));
    assert_near(status.position, at(-3_000.0, 0.0, 0.0), 1e-9);
}

#[tokio::test]
async fn a_controller_that_reports_in_inches_is_refused() {
    let (host, _inject) = scripted("Grbl 1.1h ['$' for help]", |line, _| match line {
        "$$" => [vec!["$13=1".to_string()], listing()].concat(),
        _ => vec!["ok".to_string()],
    });
    let connected = GrblStage::connect(host, options(GrblMotion::default()), running_clock());
    let refused = connected.await.err();
    let named = matches!(&refused, Some(DeviceError::InvalidArgument(m)) if m.contains("$13=0"));
    assert!(named, "{refused:?}");
}
