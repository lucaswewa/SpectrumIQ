//! A dialect: what a device's ASCII protocol looks like, read from a TOML file (ADR-0030).

use std::path::Path;

use serde::Deserialize;
use siq_devices::DeviceError;
use siq_serial::Framing;
use siq_types::{Hertz, Micrometers, MicrometersPerSecond, Millis};

use crate::template::{Template, Value, Values};

/// A device's protocol.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Dialect {
    /// The device's name, as its `DeviceInfo` vendor.
    pub name: String,
    /// The length of a step, for `steps` fields.
    pub step: Micrometers,
    /// What ends each line.
    #[serde(default)]
    pub terminators: Terminators,
    /// How long the device may take.
    #[serde(default)]
    pub timeouts: Timeouts,
    /// How the device moves.
    pub motion: Motion,
    /// The replies that every command may get.
    pub replies: Replies,
    /// The commands.
    pub commands: Commands,
}

/// What ends each line.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Terminators {
    /// After each line sent.
    pub write: String,
    /// The one character that ends each line received.
    pub read: String,
}

impl Default for Terminators {
    fn default() -> Self {
        Self {
            write: "\r".into(),
            read: "\r".into(),
        }
    }
}

/// How long the device may take.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Timeouts {
    /// To reply to a command, in real time.
    pub reply: Millis,
    /// To home, on the clock.
    pub home: Millis,
}

impl Default for Timeouts {
    fn default() -> Self {
        Self {
            reply: Millis(500.0),
            home: Millis(60_000.0),
        }
    }
}

/// How the device moves.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Motion {
    /// Where it may go: `[min, max]`.
    pub travel: [Micrometers; 2],
    /// The speed of a move that gives none.
    pub speed: MicrometersPerSecond,
    /// How the end of a move shows.
    #[serde(default)]
    pub done: Done,
    /// How often the driver asks for the status.
    #[serde(default = "poll_rate")]
    pub poll_hz: Hertz,
    /// How far a unidirectional move first goes below a target it would reach going down.
    #[serde(default = "overshoot")]
    pub overshoot: Micrometers,
}

fn poll_rate() -> Hertz {
    Hertz(10.0)
}

fn overshoot() -> Micrometers {
    Micrometers(10.0)
}

/// How the end of a move shows.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Done {
    /// The status's `moving` flag clears.
    #[default]
    Poll,
    /// The device sends `replies.complete`.
    Reply,
}

/// The replies that every command may get.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Replies {
    /// The acknowledgement, for a command whose reply isn't given.
    pub ok: Template,
    /// The errors, with optional fields `{code:int}` and `{message:text}`.
    #[serde(default)]
    pub errors: Vec<Template>,
    /// What the device sends when a move or homing ends, for `motion.done = "reply"`.
    pub complete: Option<Template>,
}

/// The commands. Those without a reply are answered with `replies.ok`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Commands {
    /// Lines sent on connecting, each answered with `replies.ok`.
    #[serde(default)]
    pub init: Vec<Template>,
    /// Who the device is: a reply with `{model:text}`, `{firmware:text}` and `{serial:text}`.
    pub identify: Option<Command>,
    /// A move to `{target}`, at `{speed}`.
    #[serde(rename = "move")]
    pub move_to: Command,
    /// The status: a reply with `{position}`, and the flags `{moving:flag}` and `{homed:flag}`.
    pub status: Command,
    /// Stops any motion.
    pub stop: Command,
    /// Homes the drive.
    pub home: Option<Command>,
    /// Moves at `{velocity}` until stopped.
    pub jog: Option<Command>,
}

/// A command, and its reply.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Command {
    /// What is sent.
    pub send: Template,
    /// The reply, or `replies.ok` if none.
    pub reply: Option<Template>,
}

/// Why a dialect can't be used: every problem, with where it is.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{origin}: {}", problems.join("\n"))]
pub struct DialectError {
    /// The file, or what the text came from.
    pub origin: String,
    /// Each problem, as `path: message`.
    pub problems: Vec<String>,
}

impl Dialect {
    /// Reads and checks the dialect in `path`.
    pub fn load(path: &Path) -> Result<Self, DialectError> {
        let origin = path.display().to_string();
        match std::fs::read_to_string(path) {
            Ok(text) => Self::parse(&text, &origin),
            Err(error) => Err(DialectError {
                origin,
                problems: vec![error.to_string()],
            }),
        }
    }

    /// Parses and checks the dialect in `text`, which came from `origin`.
    pub fn parse(text: &str, origin: &str) -> Result<Self, DialectError> {
        let fail = |problems| DialectError {
            origin: origin.to_owned(),
            problems,
        };
        let dialect: Self = toml::from_str(text).map_err(|error| fail(vec![error.to_string()]))?;
        let problems = dialect.problems();
        if problems.is_empty() {
            Ok(dialect)
        } else {
            Err(fail(problems))
        }
    }

    /// The text of the example focus drive's dialect, `config/dialects/example-focus.toml`, which
    /// the simulated focus drive speaks by default.
    pub const EXAMPLE: &str = include_str!("../../../config/dialects/example-focus.toml");

    /// The example focus drive's dialect.
    pub fn example() -> Self {
        Self::parse(Self::EXAMPLE, "example-focus.toml").expect("the example dialect is valid")
    }

    /// The link's framing.
    pub fn framing(&self) -> Framing {
        Framing {
            write: self.terminators.write.clone(),
            read: self.terminators.read.bytes().next().unwrap_or(b'\r'),
        }
    }

    /// Whether the status reply has field `name`.
    pub fn reports(&self, name: &str) -> bool {
        let reply = self.commands.status.reply.as_ref();
        reply.is_some_and(|reply| reply.has(name))
    }

    /// The error that `line` reports, if it is one of the dialect's errors.
    pub fn error(&self, line: &str) -> Option<DeviceError> {
        let values = self
            .replies
            .errors
            .iter()
            .find_map(|error| error.matches(line, 1.0))?;
        let code = match values.get("code") {
            Some(Value::Int(code)) => format!(" {code}"),
            _ => String::new(),
        };
        let message = match values.get("message") {
            Some(Value::Text(message)) => format!(": {message}"),
            _ => String::new(),
        };
        Some(DeviceError::Fault(format!(
            "{} error{code}{message}",
            self.name
        )))
    }

    /// Every broken rule, as `path: message`.
    fn problems(&self) -> Vec<String> {
        let mut p = Problems::default();
        let (motion, commands, replies) = (&self.motion, &self.commands, &self.replies);
        p.check(positive(self.step.0), "step", "must be positive");
        p.check(
            !self.terminators.write.is_empty(),
            "terminators.write",
            "can't be empty",
        );
        let one = self.terminators.read.len() == 1;
        p.check(one, "terminators.read", "must be one ASCII character");
        let [min, max] = motion.travel;
        let ordered = min.0.is_finite() && max.0.is_finite() && min < max;
        p.check(
            ordered,
            "motion.travel",
            "must be [min, max], with min below max",
        );
        p.check(positive(motion.speed.0), "motion.speed", "must be positive");
        let polling = motion.poll_hz.0 > 0.0 && motion.poll_hz.0 <= 100.0;
        p.check(
            polling,
            "motion.poll_hz",
            "must be above 0 and at most 100 Hz",
        );
        let overshoot = motion.overshoot.0.is_finite() && motion.overshoot.0 >= 0.0;
        p.check(overshoot, "motion.overshoot", "must be 0 or more");
        p.check(
            positive(self.timeouts.reply.0),
            "timeouts.reply",
            "must be positive",
        );
        p.check(
            positive(self.timeouts.home.0),
            "timeouts.home",
            "must be positive",
        );

        // Each template's fields, with what each holds, and those it needs.
        const LENGTH: &str = "a length";
        let none: Fields = &[];
        p.fields("replies.ok", Some(&replies.ok), none, &[]);
        p.fields("replies.complete", replies.complete.as_ref(), none, &[]);
        for (i, error) in replies.errors.iter().enumerate() {
            let allowed = [("code", "a number"), ("message", "text")];
            p.fields(&format!("replies.errors[{i}]"), Some(error), &allowed, &[]);
        }
        for (i, line) in commands.init.iter().enumerate() {
            p.fields(&format!("commands.init[{i}]"), Some(line), none, &[]);
        }
        let identity = [("model", "text"), ("firmware", "text"), ("serial", "text")];
        let flags = [
            ("position", LENGTH),
            ("moving", "a flag"),
            ("homed", "a flag"),
        ];
        let table: [Rule; 6] = [
            (
                "identify",
                commands.identify.as_ref(),
                none,
                &[],
                &identity,
                &[],
            ),
            (
                "move",
                Some(&commands.move_to),
                &[("target", LENGTH), ("speed", LENGTH)],
                &["target"],
                none,
                &[],
            ),
            (
                "status",
                Some(&commands.status),
                none,
                &[],
                &flags,
                &["position"],
            ),
            ("stop", Some(&commands.stop), none, &[], none, &[]),
            ("home", commands.home.as_ref(), none, &[], none, &[]),
            (
                "jog",
                commands.jog.as_ref(),
                &[("velocity", LENGTH)],
                &["velocity"],
                none,
                &[],
            ),
        ];
        for (name, command, send, needs, reply, reply_needs) in table {
            let Some(command) = command else { continue };
            p.fields(
                &format!("commands.{name}.send"),
                Some(&command.send),
                send,
                needs,
            );
            let path = format!("commands.{name}.reply");
            p.fields(&path, command.reply.as_ref(), reply, reply_needs);
        }
        let status = commands.status.reply.as_ref();
        p.check(
            status.is_some(),
            "commands.status.reply",
            "must say how the status reads",
        );
        match motion.done {
            Done::Poll => {
                let moving = self.reports("moving");
                let message = "needs {moving:flag} when motion.done is \"poll\"";
                p.check(moving || status.is_none(), "commands.status.reply", message);
            }
            Done::Reply => {
                let message = "must be given when motion.done is \"reply\"";
                p.check(replies.complete.is_some(), "replies.complete", message);
            }
        }
        p.0
    }
}

fn positive(value: f64) -> bool {
    value.is_finite() && value > 0.0
}

/// A template's allowed fields, each with what it holds.
type Fields<'a> = &'a [(&'a str, &'a str)];

/// A command's rules: its name, the command, its send's fields and those it needs, and its
/// reply's.
type Rule<'a> = (
    &'a str,
    Option<&'a Command>,
    Fields<'a>,
    &'a [&'a str],
    Fields<'a>,
    &'a [&'a str],
);

#[derive(Default)]
struct Problems(Vec<String>);

impl Problems {
    fn check(&mut self, ok: bool, path: &str, message: &str) {
        if !ok {
            self.0.push(format!("{path}: {message}"));
        }
    }

    /// Checks that `template`'s fields are among `allowed`, each with its class, and that the
    /// `required` ones are there.
    fn fields(
        &mut self,
        path: &str,
        template: Option<&Template>,
        allowed: Fields,
        required: &[&str],
    ) {
        let Some(template) = template else { return };
        for (name, kind) in template.fields() {
            match allowed.iter().find(|(allowed, _)| *allowed == name) {
                None => self.0.push(format!("{path}: has no field {{{name}}}")),
                Some((_, class)) if *class != kind.class() => {
                    self.0.push(format!(
                        "{path}: {{{name}}} must be {class}, not {}",
                        kind.class()
                    ));
                }
                Some(_) => {}
            }
        }
        for name in required.iter().filter(|name| !template.has(name)) {
            self.0.push(format!("{path}: needs a field {{{name}}}"));
        }
    }
}

/// Values for a template: lengths and speeds by name, in µm and µm/s.
pub fn lengths<const N: usize>(values: [(&str, f64); N]) -> Values {
    values
        .into_iter()
        .map(|(name, value)| (name.to_owned(), Value::Length(value)))
        .collect()
}
