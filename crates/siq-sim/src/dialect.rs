//! A focus drive that speaks a dialect (REQ-SIM-002, ADR-0030): the device end of a serial link,
//! made from the same dialect file that the driver reads.
//!
//! - **Commands:** each line is matched against the dialect's command templates, in turn, and
//!   answered with its reply rendered, or `replies.ok`.
//! - **Errors:** a line that matches no command gets the first error template with code 1, and a
//!   move outside the travel gets code 2.
//! - **Motion:** moves run at their speed with no acceleration. Jogs run until stopped or at the
//!   edge of the travel. Homing runs to the bottom of the travel, at the default speed.
//! - **Completion:** with `motion.done = "reply"`, a move or homing that ends sends
//!   `replies.complete`, and a stopped one doesn't.
//! - **The true focus** follows on every tick.

use std::io;
use std::sync::Arc;
use std::time::Duration;

use siq_ascii::{Command, Dialect, Done, Value, Values};
use siq_types::{Clock, Micrometers, Timestamp};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::state::TrueState;

/// How often the emulator brings its motion and the true focus up to the clock.
const TICK: Duration = Duration::from_millis(2);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Move,
    Jog,
    Home,
}

/// A motion at constant speed, from `from` to `to`, in µm.
#[derive(Debug, Clone, Copy)]
struct Motion {
    kind: Kind,
    from: f64,
    to: f64,
    speed: f64,
    start: f64,
}

impl Motion {
    /// Where it is at `now`, in seconds, and whether it has ended.
    fn at(&self, now: f64) -> (f64, bool) {
        let gone = self.speed * (now - self.start).max(0.0);
        let distance = (self.to - self.from).abs();
        match gone >= distance {
            true => (self.to, true),
            false => (self.from + (self.to - self.from).signum() * gone, false),
        }
    }
}

/// The emulated focus drive.
pub struct DialectEmulator {
    dialect: Arc<Dialect>,
    state: Arc<TrueState>,
    clock: Arc<dyn Clock>,
    position: f64,
    homed: bool,
    motion: Option<Motion>,
}

fn seconds(now: Timestamp) -> f64 {
    now.0 as f64 / 1e9
}

impl DialectEmulator {
    /// A drive that speaks `dialect`, at the bottom of its travel and not homed, moving `state`'s
    /// focus on `clock`.
    pub fn new(dialect: Arc<Dialect>, state: Arc<TrueState>, clock: Arc<dyn Clock>) -> Self {
        let position = dialect.motion.travel[0].0;
        state.set_focus(Micrometers(position));
        Self {
            dialect,
            state,
            clock,
            position,
            homed: false,
            motion: None,
        }
    }

    /// Serves `port`, the device's end of a link, until the link closes.
    pub async fn run<S: AsyncRead + AsyncWrite + Unpin>(mut self, port: S) -> io::Result<()> {
        let (mut reader, mut writer) = tokio::io::split(port);
        let (end, ends) = (
            self.dialect.terminators.write.clone(),
            self.dialect.terminators.read.clone(),
        );
        let (mut line, mut buffer) = (Vec::new(), [0u8; 256]);
        loop {
            let mut out = Vec::new();
            tokio::select! {
                read = reader.read(&mut buffer) => {
                    let count = read?;
                    if count == 0 {
                        return Ok(());
                    }
                    for &byte in &buffer[..count] {
                        line.push(byte);
                        if line.ends_with(end.as_bytes()) {
                            line.truncate(line.len() - end.len());
                            let text = String::from_utf8_lossy(&std::mem::take(&mut line)).into_owned();
                            out.extend(self.command(&text, seconds(self.clock.now())));
                        }
                    }
                }
                () = tokio::time::sleep(TICK) => {}
            }
            out.extend(self.advance(seconds(self.clock.now())));
            for text in out {
                writer.write_all(format!("{text}{ends}").as_bytes()).await?;
            }
            writer.flush().await?;
        }
    }

    /// Brings the motion up to `now`, and gives back the completion reply if one ended.
    fn advance(&mut self, now: f64) -> Option<String> {
        let motion = self.motion?;
        let (position, ended) = motion.at(now);
        self.position = position;
        self.state.set_focus(Micrometers(position));
        if !ended {
            return None;
        }
        self.motion = None;
        self.homed |= motion.kind == Kind::Home;
        let complete = self.dialect.replies.complete.as_ref();
        let notify = motion.kind != Kind::Jog && self.dialect.motion.done == Done::Reply;
        complete
            .filter(|_| notify)
            .map(|complete| complete.render(&Values::new(), 1.0))
    }

    fn start(&mut self, kind: Kind, to: f64, speed: f64, now: f64) {
        let from = self.position;
        self.motion = Some(Motion {
            kind,
            from,
            to,
            speed,
            start: now,
        });
    }

    /// Takes one line at `now`, and gives back the lines to send.
    fn command(&mut self, text: &str, now: f64) -> Vec<String> {
        let mut out: Vec<String> = self.advance(now).into_iter().collect();
        let dialect = self.dialect.clone();
        let (commands, step) = (&dialect.commands, dialect.step.0);
        let [min, max] = dialect.motion.travel.map(|end| end.0);
        let reply = |command: &Command, values: &Values| {
            let template = command.reply.as_ref().unwrap_or(&dialect.replies.ok);
            template.render(values, step)
        };
        let error = |code: i64, message: &str| {
            let values = Values::from([
                ("code".to_owned(), Value::Int(code)),
                ("message".to_owned(), Value::Text(message.to_owned())),
            ]);
            let error = dialect.replies.errors.first();
            error.map(|error| error.render(&values, step))
        };
        let length = |values: &Values, name: &str| match values.get(name) {
            Some(Value::Length(length)) => Some(*length),
            _ => None,
        };
        let none = Values::new();

        if commands
            .init
            .iter()
            .any(|line| line.matches(text, step).is_some())
        {
            out.push(dialect.replies.ok.render(&none, step));
        } else if let Some((command, _)) = matched(commands.identify.as_ref(), text, step) {
            let text = |text: &str| Value::Text(text.to_owned());
            let values = Values::from([
                ("model".to_owned(), text("SIM-FOCUS")),
                ("firmware".to_owned(), text("1.0")),
                ("serial".to_owned(), text("0001")),
            ]);
            out.push(reply(command, &values));
        } else if let Some((command, values)) = matched(Some(&commands.move_to), text, step) {
            let target = length(&values, "target").unwrap_or(self.position);
            let speed = length(&values, "speed").unwrap_or(dialect.motion.speed.0);
            if (min..=max).contains(&target) && speed > 0.0 {
                self.start(Kind::Move, target, speed, now);
                out.push(reply(command, &none));
            } else {
                out.extend(error(2, "out of range"));
            }
        } else if let Some((command, _)) = matched(Some(&commands.status), text, step) {
            let values = Values::from([
                ("position".to_owned(), Value::Length(self.position)),
                ("moving".to_owned(), Value::Flag(self.motion.is_some())),
                ("homed".to_owned(), Value::Flag(self.homed)),
            ]);
            out.push(reply(command, &values));
        } else if let Some((command, _)) = matched(Some(&commands.stop), text, step) {
            self.motion = None;
            out.push(reply(command, &none));
        } else if let Some((command, _)) = matched(commands.home.as_ref(), text, step) {
            self.start(Kind::Home, min, dialect.motion.speed.0, now);
            out.push(reply(command, &none));
        } else if let Some((command, values)) = matched(commands.jog.as_ref(), text, step) {
            let velocity = length(&values, "velocity").unwrap_or(0.0);
            let edge = if velocity > 0.0 { max } else { min };
            self.start(Kind::Jog, edge, velocity.abs(), now);
            out.push(reply(command, &none));
        } else {
            out.extend(error(1, "unknown command"));
        }
        out
    }
}

/// `command` and the values in `text`, if `text` is that command.
fn matched<'a>(
    command: Option<&'a Command>,
    text: &str,
    step: f64,
) -> Option<(&'a Command, Values)> {
    let command = command?;
    Some((command, command.send.matches(text, step)?))
}
