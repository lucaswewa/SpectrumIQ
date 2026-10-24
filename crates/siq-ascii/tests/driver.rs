//! The driver against the dialect emulator, over an in-memory link, on a clock that runs ten
//! times faster than real time: the example dialect, its completion-reply variant, and pairs of
//! dialects that disagree, for errors and timeouts.

use std::sync::Arc;
use std::time::{Duration, Instant};

use siq_ascii::{AsciiFocusDrive, Dialect};
use siq_devices::{
    Approach, Device, DeviceError, DeviceResult, FocusDrive, FocusStatus, MotionState, MoveOpts,
};
use siq_serial::{DuplexTransport, duplex};
use siq_sim::dialect::DialectEmulator;
use siq_sim::state::TrueState;
use siq_types::{Clock, ManualClock, Micrometers, MicrometersPerSecond, Timestamp};

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

/// The example dialect, with each `(from, to)` replaced.
fn variant(changes: &[(&str, &str)]) -> Dialect {
    let mut text = Dialect::EXAMPLE.to_owned();
    for (from, to) in changes {
        assert!(text.contains(from), "{from}");
        text = text.replace(from, to);
    }
    Dialect::parse(&text, "variant.toml").unwrap()
}

/// The changes that make the example wait for `DONE`, without a `moving` flag.
const REPLYING: [(&str, &str); 2] = [
    ("done = \"poll\"", "done = \"reply\""),
    (" {moving:flag} {homed:flag}", " {homed:flag}"),
];

struct Rig {
    drive: AsciiFocusDrive<DuplexTransport>,
    state: Arc<TrueState>,
    clock: Arc<ManualClock>,
}

async fn connect(driver: Dialect, device: Dialect) -> DeviceResult<Rig> {
    let (clock, state) = (running_clock(), Arc::new(TrueState::new(0)));
    let (host, port) = duplex(driver.framing(), "focus");
    let emulator = DialectEmulator::new(Arc::new(device), state.clone(), clock.clone());
    tokio::spawn(emulator.run(port));
    let drive = AsciiFocusDrive::connect(host, Arc::new(driver), clock.clone()).await?;
    Ok(Rig {
        drive,
        state,
        clock,
    })
}

async fn rig(driver: Dialect, device: Dialect) -> Rig {
    connect(driver, device).await.unwrap()
}

impl Rig {
    fn focus(&self) -> f64 {
        self.state.pose().focus.0
    }

    /// Runs `motion`, and gives back its result, how long it took on the clock, and the lowest
    /// true focus on the way.
    async fn timed<T>(&self, motion: impl Future<Output = T>) -> (T, f64, f64) {
        let (start, mut lowest) = (self.clock.now(), self.focus());
        tokio::pin!(motion);
        let result = loop {
            tokio::select! {
                result = &mut motion => break result,
                () = tokio::time::sleep(Duration::from_millis(1)) => lowest = lowest.min(self.focus()),
            }
        };
        let took = self.clock.now().saturating_since(start).as_secs_f64();
        (result, took, lowest.min(self.focus()))
    }
}

const DIRECT: MoveOpts = MoveOpts {
    speed: None,
    approach: Approach::Direct,
};

fn at_speed(um_per_s: f64) -> MoveOpts {
    MoveOpts {
        speed: Some(MicrometersPerSecond(um_per_s)),
        ..DIRECT
    }
}

#[track_caller]
fn assert_idle_at(status: FocusStatus, um: f64) {
    assert_eq!(status.state, MotionState::Idle, "{status:?}");
    assert!(
        (status.position.0 - um).abs() < 1e-6,
        "{status:?} isn't at {um} µm"
    );
}

#[tokio::test]
async fn connecting_identifies_the_drive_and_publishes_its_status() {
    let rig = rig(Dialect::example(), Dialect::example()).await;
    let info = rig.drive.info();
    assert_eq!(
        (info.vendor.as_str(), info.model.as_str()),
        ("Example focus drive", "SIM-FOCUS")
    );
    assert_eq!(
        (info.firmware.as_deref(), info.serial.as_deref()),
        (Some("1.0"), Some("0001"))
    );
    let status = *rig.drive.status().borrow();
    assert_idle_at(status, 0.0);
    assert!(!status.homed);
    let limits = rig.drive.limits();
    assert_eq!(
        (limits.min, limits.max),
        (Micrometers(0.0), Micrometers(12_000.0))
    );
}

#[tokio::test]
async fn a_move_returns_once_the_drive_is_still_at_its_target() {
    for (name, dialect) in [("poll", Dialect::example()), ("reply", variant(&REPLYING))] {
        let rig = rig(dialect.clone(), dialect).await;
        // 1000 µm at the dialect's 2000 µm/s: 0.5 s.
        let (moved, took, _) = rig
            .timed(rig.drive.move_to(Micrometers(1_000.0), DIRECT))
            .await;
        assert_idle_at(moved.unwrap(), 1_000.0);
        assert_eq!(rig.focus(), 1_000.0, "{name}");
        assert!((0.5..1.2).contains(&took), "{name}: {took} s");

        // 250 µm down at 500 µm/s: 0.5 s.
        let (moved, took, _) = rig
            .timed(rig.drive.move_by(Micrometers(-250.0), at_speed(500.0)))
            .await;
        assert_idle_at(moved.unwrap(), 750.0);
        assert!((0.5..1.2).contains(&took), "{name}: {took} s");

        let outside = rig.drive.move_to(Micrometers(12_001.0), DIRECT).await;
        assert!(
            matches!(outside, Err(DeviceError::InvalidArgument(_))),
            "{outside:?}"
        );
    }
}

#[tokio::test]
async fn a_unidirectional_move_arrives_going_up() {
    let rig = rig(Dialect::example(), Dialect::example()).await;
    let one_way = MoveOpts {
        approach: Approach::Unidirectional,
        ..at_speed(200.0)
    };
    rig.drive
        .move_to(Micrometers(1_100.0), DIRECT)
        .await
        .unwrap();
    // Going down, it first goes 10 µm below, then comes up.
    let (moved, _, lowest) = rig
        .timed(rig.drive.move_to(Micrometers(1_000.0), one_way))
        .await;
    assert_idle_at(moved.unwrap(), 1_000.0);
    assert!((989.0..992.0).contains(&lowest), "{lowest}");
    // Going up, it goes straight there.
    rig.drive.move_to(Micrometers(900.0), DIRECT).await.unwrap();
    let (moved, _, lowest) = rig
        .timed(rig.drive.move_to(Micrometers(1_000.0), one_way))
        .await;
    assert_idle_at(moved.unwrap(), 1_000.0);
    assert_eq!(lowest, 900.0);
}

#[tokio::test]
async fn stop_interrupts_a_move_and_a_jog_runs_until_stopped() {
    let rig = rig(Dialect::example(), Dialect::example()).await;
    let moving = rig.drive.move_to(Micrometers(10_000.0), DIRECT);
    let stopping = async {
        let mut status = rig.drive.status();
        status.wait_for(|s| s.position.0 > 1_000.0).await.unwrap();
        // Two stops at once both succeed.
        tokio::join!(rig.drive.stop(), rig.drive.stop())
    };
    let (moved, (stopped, again)) = tokio::join!(moving, stopping);
    assert!(matches!(moved, Err(DeviceError::Stopped)), "{moved:?}");
    stopped.unwrap();
    again.unwrap();
    let status = *rig.drive.status().borrow();
    assert_eq!(status.state, MotionState::Idle);
    assert!(
        (1_000.0..10_000.0).contains(&status.position.0),
        "{status:?}"
    );
    assert!(
        (rig.focus() - status.position.0).abs() <= 0.025,
        "within half a step"
    );

    let from = status.position.0;
    rig.drive.jog(MicrometersPerSecond(1_000.0)).await.unwrap();
    let mut status = rig.drive.status();
    let jogging = |s: &FocusStatus| s.state == MotionState::Jogging && s.position.0 > from + 100.0;
    status.wait_for(jogging).await.unwrap();
    rig.drive.jog(MicrometersPerSecond(0.0)).await.unwrap();
    let still = *rig.drive.status().borrow();
    assert_eq!(still.state, MotionState::Idle);
    assert!(still.position.0 > from + 100.0);
}

#[tokio::test]
async fn homing_references_the_drive_by_poll_or_by_reply() {
    for (name, dialect) in [("poll", Dialect::example()), ("reply", variant(&REPLYING))] {
        let rig = rig(dialect.clone(), dialect).await;
        rig.drive
            .move_to(Micrometers(3_000.0), DIRECT)
            .await
            .unwrap();
        assert!(!rig.drive.status().borrow().homed);
        // Homing runs to the bottom of the travel at 2000 µm/s: 1.5 s.
        let (homed, took, _) = rig.timed(rig.drive.home()).await;
        let homed = homed.unwrap();
        assert_idle_at(homed, 0.0);
        assert!(homed.homed, "{name}");
        assert!((1.5..2.5).contains(&took), "{name}: {took} s");
    }
}

#[tokio::test]
async fn device_errors_and_silences_fail_with_what_happened() {
    // The device doesn't know the home command.
    let homeless = variant(&[("home = { send = \"HOME\" }\n", "")]);
    let unhomed = rig(Dialect::example(), homeless).await;
    let error = unhomed.drive.home().await.unwrap_err();
    let unknown = "device fault: Example focus drive error 1: unknown command";
    assert_eq!(error.to_string(), unknown);
    // The driver checks a target before it sends it; the device checks it too.
    let wider = variant(&[("travel = [0.0, 12000.0]", "travel = [0.0, 20000.0]")]);
    let narrow = rig(wider, Dialect::example()).await;
    let error = narrow
        .drive
        .move_to(Micrometers(15_000.0), DIRECT)
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "device fault: Example focus drive error 2: out of range"
    );

    // A completion reply that never comes: the move times out on the clock.
    let finishing = variant(&[
        REPLYING[0],
        REPLYING[1],
        ("complete = \"DONE\"", "complete = \"FIN\""),
    ]);
    let unfinished = rig(variant(&REPLYING), finishing).await;
    // 1000 µm at 2000 µm/s has twice 0.5 s, and 2 s more.
    let (moved, took, _) = unfinished
        .timed(unfinished.drive.move_to(Micrometers(1_000.0), DIRECT))
        .await;
    assert!(matches!(moved, Err(DeviceError::Timeout(_))), "{moved:?}");
    assert!((3.0..4.0).contains(&took), "{took} s");

    // A device that never answers: connecting times out after the dialect's 300 ms.
    let (host, _port) = duplex(Dialect::example().framing(), "silent");
    let start = Instant::now();
    let silent = AsciiFocusDrive::connect(host, Arc::new(Dialect::example()), running_clock());
    let error = silent.await.err().unwrap();
    assert_eq!(
        error.to_string(),
        "timed out: Example focus drive didn't answer `ECHO 0`"
    );
    assert!((0.3..1.0).contains(&start.elapsed().as_secs_f64()));
}
