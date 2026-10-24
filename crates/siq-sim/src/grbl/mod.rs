//! A GRBL 1.1 emulator (REQ-SIM-002, ADR-0027): the device end of a serial link, which the real
//! GRBL driver talks to as it would to the controller.
//!
//! - **Lines:** `G0`, `G1`, `G4`, `G20`/`G21`, `G90`/`G91`, `F`, and the system commands `$`,
//!   `$$`, `$N=V`, `$X`, `$H` and `$J=…`. Replies are `ok` and `error:N`.
//! - **Realtime bytes:** `?` (a status report), `!` (feed hold), `~` (resume), 0x18 (soft reset)
//!   and 0x85 (jog cancel).
//! - **Planner:** a motion line is queued and answered `ok` at once. A `G4` or `$H` holds up the
//!   lines after it, and is answered when it ends, as GRBL does: so `G4 P0` answers when the moves
//!   before it have finished.
//! - **Motion:** trapezoidal moves along a line, within each axis's `$11x` rate and `$12x`
//!   acceleration, with soft limits over `$13x` (ALARM:2) and homing to machine zero.
//! - **The true stage** (`TrueState`) is the commanded position with the injected errors.
//! - **Reply latency** delays every reply, on the clock.
//!
//! Hold and jog cancel stop at once rather than decelerating, and there are no missed steps.

use std::collections::VecDeque;
use std::io;
use std::sync::Arc;
use std::time::Duration;

use siq_config::model::SimErrors;
use siq_types::{Clock, Micrometers, Millis, Timestamp};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

mod machine;
mod settings;

pub use settings::GrblSettings;

use crate::state::TrueState;
use machine::Machine;

/// The errors between where the controller thinks the stage is and where it really is.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct GrblErrors {
    /// The lost motion of each axis when it reverses.
    pub backlash: [Micrometers; 3],
    /// The axes' relative scale error.
    pub scale_error: f64,
    /// The random error of each move's end, as a standard deviation.
    pub position_noise: Micrometers,
    /// How long the controller takes to reply.
    pub reply_latency: Millis,
}

impl From<&SimErrors> for GrblErrors {
    /// The configuration's errors, with the same backlash on every axis.
    fn from(errors: &SimErrors) -> Self {
        Self {
            backlash: [errors.backlash; 3],
            scale_error: errors.scale_error,
            position_noise: errors.position_noise,
            reply_latency: errors.reply_latency,
        }
    }
}

/// How often the emulator brings its motion and the true stage up to the clock.
const TICK: Duration = Duration::from_millis(2);

/// The emulated controller.
pub struct GrblEmulator {
    machine: Machine,
    clock: Arc<dyn Clock>,
    latency: Duration,
}

impl GrblEmulator {
    /// A controller with `settings` and `errors`, its random errors from `seed`, moving `state`'s
    /// stage on `clock`. The stage axes are machine millimetres × 1000.
    pub fn new(
        settings: GrblSettings,
        errors: GrblErrors,
        seed: u64,
        state: Arc<TrueState>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        let latency = Duration::from_secs_f64(errors.reply_latency.0.max(0.0) / 1e3);
        Self {
            machine: Machine::new(settings, errors, seed, state),
            clock,
            latency,
        }
    }

    /// Serves `port`, the device's end of a link, until the link closes. It starts by printing
    /// the banner, as the controller does when the port opens.
    pub async fn run<S: AsyncRead + AsyncWrite + Unpin>(mut self, port: S) -> io::Result<()> {
        let (mut reader, mut writer) = tokio::io::split(port);
        let mut pending: VecDeque<(Timestamp, String)> = VecDeque::new();
        let send = |pending: &mut VecDeque<_>, at: Timestamp, lines: Vec<String>| {
            pending.extend(lines.into_iter().map(|line| (at, line)));
        };
        send(
            &mut pending,
            self.clock.now() + self.latency,
            self.machine.boot(None),
        );
        let mut buffer = [0u8; 256];
        loop {
            tokio::select! {
                read = reader.read(&mut buffer) => {
                    let (count, now) = (read?, self.clock.now());
                    if count == 0 {
                        return Ok(());
                    }
                    for &byte in &buffer[..count] {
                        send(&mut pending, now + self.latency, self.machine.byte(byte, now));
                    }
                }
                () = tokio::time::sleep(TICK) => {}
            }
            let now = self.clock.now();
            send(&mut pending, now + self.latency, self.machine.tick(now));
            while pending.front().is_some_and(|(at, _)| *at <= now) {
                let (_, line) = pending.pop_front().expect("a reply is due");
                writer.write_all(line.as_bytes()).await?;
                writer.write_all(b"\r\n").await?;
            }
            writer.flush().await?;
        }
    }
}
