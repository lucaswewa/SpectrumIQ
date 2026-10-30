//! The link task: the transport's one owner.
//!
//! - It sends one line at a time, and waits for its `ok` or error until the line's deadline.
//! - A line queued before a reset isn't sent: the reset has stopped what it belonged to.
//! - A reply that comes after its line timed out is dropped, so that it can't answer the next.
//! - It sends realtime bytes and resets at once, ahead of a line that waits.
//! - It polls for status on the clock, and keeps the status watch. A report's timestamp is its
//!   arrival less half the round trip.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use siq_devices::{DeviceError, DeviceResult, MotionState, StageStatus, Xyz, watch};
use siq_serial::{LineTransport, SerialError};
use siq_types::{Clock, Micrometers, Timestamp};
use tokio::sync::{mpsc, oneshot};
use tracing::{debug, warn};

use crate::protocol::{self, State};

/// A line to send, answered when its reply comes, or failed at `deadline` on the clock.
pub(crate) struct Line {
    pub text: String,
    /// The resets sent before it was queued.
    pub epoch: u64,
    pub deadline: Timestamp,
    pub reply: oneshot::Sender<DeviceResult<()>>,
}

/// What goes out at once, ahead of a line that waits.
pub(crate) enum Urgent {
    /// A realtime byte: feed hold, resume, jog cancel.
    Byte(u8),
    /// A status report now, rather than at the next poll.
    Poll,
    /// A soft reset, answered when the banner comes back.
    Reset(oneshot::Sender<()>),
}

/// What the link task shares with the stage.
pub(crate) struct Shared {
    pub status: watch::Sender<StageStatus>,
    pub homed: AtomicBool,
    /// The last alarm's code: 0 for none, or for the lock until homed.
    pub alarm: AtomicU32,
    /// How many resets the link has sent.
    pub resets: AtomicU64,
    /// The status's round trips: how many, and their least, sum and most.
    pub round_trips: Mutex<Option<(u64, [Duration; 3])>>,
}

/// How long the task waits for a line before it looks at the clock and its queues again.
const STEP: Duration = Duration::from_millis(5);

struct Link {
    shared: Arc<Shared>,
    clock: Arc<dyn Clock>,
    /// The line sent and not yet answered.
    waiting: Option<Line>,
    /// The reset sent, answered when the banner comes.
    resetting: Option<oneshot::Sender<()>>,
    /// When the status report not yet received was asked for.
    polled: Option<Timestamp>,
    /// Replies still due to lines that timed out.
    late: u32,
    /// The last work offset the controller sent, in mm.
    offset: [f64; 3],
}

/// Runs the link until the stage goes away or the transport closes.
pub(crate) async fn run<T: LineTransport>(
    mut transport: T,
    shared: Arc<Shared>,
    clock: Arc<dyn Clock>,
    poll: Duration,
    mut lines: mpsc::UnboundedReceiver<Line>,
    mut urgent: mpsc::UnboundedReceiver<Urgent>,
) {
    let mut link = Link {
        shared,
        clock,
        waiting: None,
        resetting: None,
        polled: None,
        late: 0,
        offset: [0.0; 3],
    };
    let mut next_poll = link.clock.now();
    loop {
        let now = link.clock.now();
        if now >= next_poll {
            if transport.write_realtime(b'?').await.is_err() {
                break;
            }
            link.polled.get_or_insert(now);
            next_poll = now + poll;
        }
        if link
            .waiting
            .as_ref()
            .is_some_and(|line| now >= line.deadline)
        {
            let line = link.waiting.take().expect("a line waits");
            link.late += 1;
            let error = DeviceError::Timeout(format!("no reply to {}", line.text));
            let _ = line.reply.send(Err(error));
        }
        tokio::select! {
            request = urgent.recv() => {
                let sent = match request {
                    None => return,
                    Some(Urgent::Byte(byte)) => transport.write_realtime(byte).await,
                    Some(Urgent::Poll) => {
                        link.polled.get_or_insert(link.clock.now());
                        transport.write_realtime(b'?').await
                    }
                    Some(Urgent::Reset(reply)) => {
                        link.shared.resets.fetch_add(1, Ordering::AcqRel);
                        link.resetting = Some(reply);
                        transport.write_realtime(0x18).await
                    }
                };
                if sent.is_err() {
                    break;
                }
            }
            request = lines.recv(), if link.waiting.is_none() => {
                let Some(line) = request else { return };
                if line.epoch != link.shared.resets.load(Ordering::Acquire) {
                    let _ = line.reply.send(Err(DeviceError::Stopped));
                } else if transport.write_line(&line.text).await.is_err() {
                    let _ = line.reply.send(Err(DeviceError::Disconnected("the stage's link failed".into())));
                    break;
                } else {
                    link.waiting = Some(line);
                }
            }
            received = transport.read_line(STEP) => match received {
                Ok(text) => link.handle(&text),
                Err(SerialError::Timeout) => {}
                Err(_) => break,
            },
        }
    }
    // The link is gone: the line that waits fails, and the status says so.
    link.answer(Err(DeviceError::Disconnected(
        "the stage's link closed".into(),
    )));
    let now = link.clock.now();
    link.shared
        .status
        .send_modify(|status| (status.state, status.ts) = (MotionState::Disconnected, now));
}

impl Link {
    fn answer(&mut self, result: DeviceResult<()>) {
        if let Some(line) = self.waiting.take() {
            let _ = line.reply.send(result);
        }
    }

    /// Answers the line that waits, or drops a late reply.
    fn reply(&mut self, result: DeviceResult<()>) {
        match self.late {
            0 => self.answer(result),
            _ => self.late -= 1,
        }
    }

    /// Takes one line from the controller.
    fn handle(&mut self, text: &str) {
        let shared = &self.shared;
        if let Some(report) = protocol::parse_status(text, &mut self.offset) {
            let now = self.clock.now();
            let polled = self.polled.take();
            let trip = now.saturating_since(polled.unwrap_or(now));
            let ts = Timestamp(now.0 - trip.as_nanos() as u64 / 2);
            // A report asked for: one that comes alone has no round trip.
            if polled.is_some() {
                let mut trips = shared
                    .round_trips
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner);
                let (count, [min, sum, max]) = trips.unwrap_or((0, [trip, Duration::ZERO, trip]));
                *trips = Some((count + 1, [min.min(trip), sum + trip, max.max(trip)]));
            }
            let state = match report.state {
                State::Idle | State::Check | State::Sleep => MotionState::Idle,
                State::Run => MotionState::Moving,
                State::Jog => MotionState::Jogging,
                State::Home => MotionState::Homing,
                State::Hold | State::Door => MotionState::Holding,
                State::Alarm => MotionState::Alarm(shared.alarm.load(Ordering::Relaxed)),
            };
            let [x, y, z] = report.position.map(|mm| Micrometers(mm * 1000.0));
            let homed = shared.homed.load(Ordering::Relaxed);
            shared.status.send_replace(StageStatus {
                position: Xyz::new(x, y, z),
                state,
                homed,
                ts,
            });
        } else if text == "ok" {
            self.reply(Ok(()));
        } else if let Some(code) = text
            .strip_prefix("error:")
            .and_then(|code| code.parse().ok())
        {
            self.reply(Err(protocol::error(code)));
        } else if let Some(code) = text
            .strip_prefix("ALARM:")
            .and_then(|code| code.parse::<u32>().ok())
        {
            // An alarm aborts the line that waits: it will never be answered.
            warn!(code, "the stage's controller raised an alarm");
            shared.alarm.store(code, Ordering::Relaxed);
            if protocol::loses_position(code) {
                shared.homed.store(false, Ordering::Relaxed);
            }
            self.answer(Err(DeviceError::Alarm(code)));
        } else if text.starts_with("Grbl") {
            // A reset: the controller has forgotten every line.
            debug!(banner = text, "the stage's controller restarted");
            self.late = 0;
            self.answer(Err(DeviceError::Stopped));
            if let Some(reply) = self.resetting.take() {
                let _ = reply.send(());
            }
        }
    }
}
