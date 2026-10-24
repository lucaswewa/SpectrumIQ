//! The dialect emulator through a real line transport, on a manual clock that the test sets: the
//! example dialect's commands, replies and errors, its motion and the true focus, and the
//! completion reply of its `done = "reply"` variant.

use std::sync::Arc;
use std::time::Duration;

use siq_ascii::Dialect;
use siq_serial::{DuplexTransport, LineTransport, SerialError, duplex};
use siq_sim::dialect::DialectEmulator;
use siq_sim::state::TrueState;
use siq_types::{ManualClock, Timestamp};

struct Link {
    host: DuplexTransport,
    clock: Arc<ManualClock>,
    state: Arc<TrueState>,
}

fn connect(dialect: Dialect) -> Link {
    let (clock, state) = (Arc::new(ManualClock::new()), Arc::new(TrueState::new(0)));
    let (host, device) = duplex(dialect.framing(), "focus");
    let emulator = DialectEmulator::new(Arc::new(dialect), state.clone(), clock.clone());
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

    /// Sends `line`, and checks the reply.
    async fn ask(&mut self, line: &str, reply: &str) {
        self.host.write_line(line).await.unwrap();
        self.expect(reply, line).await;
    }

    async fn expect(&mut self, reply: &str, context: &str) {
        let received = self.host.read_line(Duration::from_secs(2)).await;
        assert_eq!(
            received.as_deref().ok(),
            Some(reply),
            "{context}: {received:?}"
        );
    }

    async fn quiet(&mut self, context: &str) {
        let received = self.host.read_line(Duration::from_millis(60)).await;
        assert!(
            matches!(received, Err(SerialError::Timeout)),
            "{context}: {received:?}"
        );
    }

    fn focus(&self) -> f64 {
        self.state.pose().focus.0
    }
}

#[tokio::test]
async fn commands_get_their_replies_and_others_an_error() {
    let mut link = connect(Dialect::example());
    link.ask("ECHO 0", "OK").await;
    link.ask("ID?", "ID SIM-FOCUS 1.0 0001").await;
    link.ask("ST?", "ST 0 0 0").await;
    link.ask("HELLO", "ERR 1 unknown command").await;
    // 300000 steps of 0.05 µm is 15 mm, beyond the 12 mm travel.
    link.ask("MA 300000 40000", "ERR 2 out of range").await;
    link.ask("MA 1000 0", "ERR 2 out of range").await;
    link.quiet("nothing more").await;
    assert_eq!(link.focus(), 0.0);
}

#[tokio::test]
async fn moves_jogs_and_homing_run_at_their_speeds_and_the_true_focus_follows() {
    let mut link = connect(Dialect::example());
    // 1000 µm at 1000 µm/s, in steps of 0.05 µm.
    link.ask("MA 20000 20000", "OK").await;
    link.at(0.5).await;
    link.ask("ST?", "ST 10000 1 0").await;
    assert_eq!(link.focus(), 500.0);
    link.at(1.0).await;
    link.ask("ST?", "ST 20000 0 0").await;
    assert_eq!(link.focus(), 1_000.0);

    // A stop holds where the move has got to.
    link.ask("MA 0 20000", "OK").await;
    link.at(1.25).await;
    link.ask("HALT", "OK").await;
    link.at(2.0).await;
    link.ask("ST?", "ST 15000 0 0").await;

    // A jog runs up at 2000 µm/s until the top of the travel, 5.625 s later.
    link.ask("JOG 40000", "OK").await;
    link.at(3.0).await;
    link.ask("ST?", "ST 55000 1 0").await;
    link.at(8.0).await;
    link.ask("ST?", "ST 240000 0 0").await;
    assert_eq!(link.focus(), 12_000.0);

    // Homing runs to the bottom at the dialect's 2000 µm/s, 6 s, and references the drive.
    link.ask("HOME", "OK").await;
    link.at(13.9).await;
    link.ask("ST?", "ST 4000 1 0").await;
    link.at(14.0).await;
    link.ask("ST?", "ST 0 0 1").await;
    link.quiet("no completion reply when polled").await;
}

#[tokio::test]
async fn with_done_by_reply_a_move_or_homing_that_ends_sends_its_completion() {
    let replying = Dialect::EXAMPLE.replace("done = \"poll\"", "done = \"reply\"");
    let mut link = connect(Dialect::parse(&replying, "replying.toml").unwrap());
    link.ask("MA 20000 20000", "OK").await;
    link.at(0.9).await;
    link.quiet("still moving").await;
    link.at(1.0).await;
    link.expect("DONE", "the move ended").await;

    // A stopped move, and a jog, end without it.
    link.ask("MA 0 20000", "OK").await;
    link.ask("HALT", "OK").await;
    link.ask("JOG -20000", "OK").await;
    link.at(3.0).await;
    link.quiet("a stop and a jog").await;
    link.ask("ST?", "ST 0 0 0").await;

    link.ask("HOME", "OK").await;
    link.at(3.1).await;
    link.expect("DONE", "homing ended").await;
    link.ask("ST?", "ST 0 0 1").await;
}
