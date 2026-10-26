//! The GRBL emulator through a real line transport, on a manual clock: golden transcripts of the
//! protocol, then the motion's timing and the errors it injects into the true stage.
//!
//! A transcript's lines are:
//!
//! - `> text`: send a line;
//! - `! c`: send realtime byte `c`, or `! 0x18`;
//! - `< text`: the next line received must be `text`;
//! - `.`: no line arrives for a while;
//! - `@ 1.5`: move the clock to 1.5 s, and give the emulator time to catch up;
//! - `# …` and blank lines are comments.

use std::sync::Arc;
use std::time::Duration;

use siq_serial::{DuplexTransport, Framing, LineTransport, SerialError, duplex};
use siq_sim::grbl::{GrblEmulator, GrblErrors, GrblSettings};
use siq_sim::state::TrueState;
use siq_types::{ManualClock, Micrometers, Timestamp};

struct Link {
    host: DuplexTransport,
    clock: Arc<ManualClock>,
    state: Arc<TrueState>,
}

fn connect(errors: GrblErrors) -> Link {
    let (clock, state) = (Arc::new(ManualClock::new()), Arc::new(TrueState::new(0)));
    let (host, device) = duplex(Framing::default(), "grbl");
    let emulator = GrblEmulator::new(
        GrblSettings::default(),
        errors,
        1,
        state.clone(),
        clock.clone(),
    );
    tokio::spawn(emulator.run(device));
    Link { host, clock, state }
}

impl Link {
    /// Lets the emulator take what was sent at the old time, moves the clock, and lets it catch up.
    async fn at(&self, seconds: f64) {
        tokio::time::sleep(Duration::from_millis(20)).await;
        self.clock.set(Timestamp((seconds * 1e9).round() as u64));
        tokio::time::sleep(Duration::from_millis(30)).await;
    }

    async fn expect(&mut self, expected: &str, context: &str) {
        let line = self.host.read_line(Duration::from_secs(2)).await;
        assert_eq!(line.as_deref().ok(), Some(expected), "{context}: {line:?}");
    }

    async fn send(&mut self, line: &str) {
        self.host.write_line(line).await.unwrap();
    }

    async fn status(&mut self) -> String {
        self.host.write_realtime(b'?').await.unwrap();
        self.host.read_line(Duration::from_secs(2)).await.unwrap()
    }
}

async fn play(name: &str) {
    let path = format!("{}/tests/transcripts/{name}", env!("CARGO_MANIFEST_DIR"));
    let transcript = std::fs::read_to_string(&path).unwrap();
    let mut link = connect(GrblErrors::default());
    for (number, line) in (1..).zip(transcript.lines()) {
        let context = format!("{name}:{number}: {line}");
        match line.split_at(line.len().min(2)) {
            ("> ", text) => link.send(text).await,
            (">", "") => link.send("").await,
            ("! ", byte) => {
                let byte = match byte.strip_prefix("0x") {
                    Some(hex) => u8::from_str_radix(hex, 16).unwrap(),
                    None => byte.as_bytes()[0],
                };
                link.host.write_realtime(byte).await.unwrap();
            }
            ("< ", text) => link.expect(text, &context).await,
            ("<", "") => link.expect("", &context).await,
            (".", "") => {
                let quiet = link.host.read_line(Duration::from_millis(100)).await;
                assert!(
                    matches!(quiet, Err(SerialError::Timeout)),
                    "{context}: {quiet:?}"
                );
            }
            ("@ ", time) => link.at(time.parse().unwrap()).await,
            _ if line.is_empty() || line.starts_with('#') => {}
            _ => panic!("{context}: not a transcript line"),
        }
    }
}

// REQ-SIM-002, REQ-STG-001
#[tokio::test]
async fn the_protocol_follows_its_transcript() {
    play("protocol.txt").await;
}

// REQ-SIM-002, REQ-STG-001
#[tokio::test]
async fn moves_and_dwells_follow_their_transcript() {
    play("motion.txt").await;
}

// REQ-SIM-002
#[tokio::test]
async fn soft_limits_resets_and_homing_follow_their_transcript() {
    play("alarms.txt").await;
}

// REQ-SIM-002
#[tokio::test]
async fn feed_hold_resume_and_jogs_follow_their_transcript() {
    play("hold_jog.txt").await;
}

/// Unlocks a freshly connected controller.
async fn unlock(link: &mut Link) {
    for expected in ["", "Grbl 1.1h ['$' for help]", "[MSG:'$H'|'$X' to unlock]"] {
        link.expect(expected, "boot").await;
    }
    link.send("$X").await;
    link.expect("[MSG:Caution: Unlocked]", "unlock").await;
    link.expect("ok", "unlock").await;
}

// REQ-SIM-002, REQ-STG-001
#[tokio::test]
async fn a_diagonal_move_takes_its_limiting_axis_s_time_and_the_true_stage_follows() {
    let mut link = connect(GrblErrors::default());
    unlock(&mut link).await;
    // 50 mm along (-0.6, -0.8) at a requested 100 mm/s: Y limits the speed to 62.5 mm/s and the
    // acceleration to 250 mm/s², so 0.25 s up, 0.55 s at speed, 0.25 s down: 1.05 s.
    link.send("G1 X-30 Y-40 F6000").await;
    link.expect("ok", "move").await;
    link.at(0.5).await;
    let along = 250.0 * 0.25 * 0.25 / 2.0 + 62.5 * 0.25;
    let (x, y) = (-0.6 * along, -0.8 * along);
    assert_eq!(
        link.status().await,
        format!("<Run|MPos:{x:.3},{y:.3},0.000|FS:3750,0>")
    );
    let truth = link.state.pose();
    assert!(
        (truth.x.0 - x * 1000.0).abs() < 1e-6 && (truth.y.0 - y * 1000.0).abs() < 1e-6,
        "{truth:?}"
    );
    link.at(1.04).await;
    assert!(link.status().await.starts_with("<Run|"));
    link.at(1.06).await;
    assert_eq!(
        link.status().await,
        "<Idle|MPos:-30.000,-40.000,0.000|FS:0,0>"
    );
    assert_eq!(
        (link.state.pose().x, link.state.pose().y),
        (Micrometers(-30_000.0), Micrometers(-40_000.0))
    );
}

// REQ-SIM-002
#[tokio::test]
async fn the_true_stage_carries_backlash_scale_error_and_random_error() {
    let errors = GrblErrors {
        backlash: [Micrometers(10.0); 3],
        scale_error: 0.001,
        ..GrblErrors::default()
    };
    let mut link = connect(errors);
    unlock(&mut link).await;
    let mut time = 0.0;
    let mut go = async |link: &mut Link, x: f64| -> f64 {
        link.send(&format!("G0 X{x}")).await;
        link.expect("ok", "move").await;
        time += 1.0;
        link.at(time).await;
        link.state.pose().x.0
    };
    // Moving to -10 mm in -X: scaled by 1.001, and 5 µm behind, so -10.005 mm.
    assert!((go(&mut link, -10.0).await + 10_005.0).abs() < 1e-6);
    // Back up to -5 mm, in +X: -5.005 scaled, 5 µm behind the other way.
    assert!((go(&mut link, -5.0).await + 5_010.0).abs() < 1e-6);
    // Reaching -5 from the same side again lands in the same place: the approach that P1.9 uses.
    go(&mut link, -8.0).await;
    assert!((go(&mut link, -5.0).await + 5_010.0).abs() < 1e-6);

    // Random error: each move ends within its σ, and the same place differs between visits.
    let noisy = GrblErrors {
        position_noise: Micrometers(0.5),
        ..GrblErrors::default()
    };
    let mut link = connect(noisy);
    unlock(&mut link).await;
    let mut ends = Vec::new();
    for visit in 0..40 {
        link.send(&format!("G0 X{}", if visit % 2 == 0 { -1.0 } else { -2.0 }))
            .await;
        link.expect("ok", "move").await;
        link.at(f64::from(visit + 1) * 0.5).await;
        if visit % 2 == 0 {
            ends.push(link.state.pose().x.0 + 1000.0);
        }
    }
    let spread = (ends.iter().map(|e| e * e).sum::<f64>() / ends.len() as f64).sqrt();
    assert!(
        (0.25..1.0).contains(&spread),
        "{spread} µm RMS for a σ of 0.5 µm: {ends:?}"
    );
    // An axis that doesn't move keeps its place: the moves in x leave y and z as they were.
    let (y, z) = (link.state.pose().y.0, link.state.pose().z_coarse.0);
    link.send("G0 X-3").await;
    link.expect("ok", "move").await;
    link.at(21.0).await;
    assert_eq!(
        (link.state.pose().y.0, link.state.pose().z_coarse.0),
        (y, z)
    );
    // Nor does one that went out and came back, as an overshoot does: it lands on its target, not
    // a rounding error from it. These were a focus map's first moves, which did.
    let mut link = connect(noisy);
    unlock(&mut link).await;
    for (time, to) in [
        (1.0, "X-20.223125 Y-15.18 Z-0.05"),
        (2.0, "X-20.173125 Y-15.13 Z0"),
    ] {
        link.send(&format!("G0 {to}")).await;
        link.expect("ok", "move").await;
        link.at(time).await;
    }
    let z = link.state.pose().z_coarse.0;
    link.send("G0 X-20").await;
    link.expect("ok", "move").await;
    link.at(3.0).await;
    assert_eq!(link.state.pose().z_coarse.0, z);
}
