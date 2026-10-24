//! The emulated controller: the line and realtime protocol, the planner, the motion, and the
//! errors injected between where it thinks it is and where the stage really is. It is synchronous
//! and timed by its caller, so a transcript plays the same on every run.

use std::collections::VecDeque;
use std::sync::Arc;

use siq_types::{Micrometers, Timestamp};

use super::{GrblErrors, GrblSettings};
use crate::optics::noise::{gaussian, hash, key};
use crate::state::TrueState;

const BANNER: &str = "Grbl 1.1h ['$' for help]";
const LOCKED: &str = "[MSG:'$H'|'$X' to unlock]";
const HELP: &str = "[HLP:$$ $# $G $I $N $x=val $Nx=line $J=line $SLP $C $X $H ~ ! ? ctrl-x]";

#[derive(Debug, Clone, Copy, PartialEq)]
enum Mode {
    Idle,
    Run,
    Hold,
    Jog,
    Home,
    /// In alarm: with GRBL's code, or none when locked until homed.
    Alarm(Option<u8>),
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Block {
    /// A straight move at up to `speed` mm/s. A jog can be cancelled.
    Move {
        target: [f64; 3],
        speed: f64,
        jog: bool,
    },
    /// G4: a wait after the moves before it, which sends its `ok` when it ends.
    Dwell { seconds: f64 },
    /// `$H`: a seek to machine zero, which sends its `ok` when it ends.
    Home,
}

/// A trapezoidal move along a line: up to speed at the acceleration, then down to a stop.
#[derive(Debug, Clone, Copy)]
struct Profile {
    unit: [f64; 3],
    speed: f64,
    accel: f64,
    ramp: f64,
    duration: f64,
}

impl Profile {
    /// From `from` to `to` at up to `speed` mm/s, within every axis's rate and acceleration.
    fn new(from: [f64; 3], to: [f64; 3], speed: f64, settings: &GrblSettings) -> Self {
        let delta: [f64; 3] = std::array::from_fn(|i| to[i] - from[i]);
        let length = delta.iter().map(|d| d * d).sum::<f64>().sqrt();
        if length == 0.0 {
            return Self {
                unit: [0.0; 3],
                speed: 0.0,
                accel: 1.0,
                ramp: 0.0,
                duration: 0.0,
            };
        }
        let unit = delta.map(|d| d / length);
        let limit = |base: u16, scale: f64| {
            (0..3)
                .filter(|&i| unit[i] != 0.0)
                .map(|i| settings.axis(base, i) * scale / unit[i].abs())
                .fold(f64::INFINITY, f64::min)
        };
        let (mut speed, accel) = (speed.min(limit(110, 1.0 / 60.0)), limit(120, 1.0));
        let mut ramp = speed / accel;
        let duration = if speed * ramp >= length {
            (speed, ramp) = ((accel * length).sqrt(), (length / accel).sqrt());
            2.0 * ramp
        } else {
            2.0 * ramp + (length - speed * ramp) / speed
        };
        Self {
            unit,
            speed,
            accel,
            ramp,
            duration,
        }
    }

    /// The distance along the move, and the speed, `t` seconds after it starts.
    fn at(&self, t: f64) -> (f64, f64) {
        let (t, a) = (t.clamp(0.0, self.duration), self.accel);
        if t < self.ramp {
            (a * t * t / 2.0, a * t)
        } else if t > self.duration - self.ramp {
            let left = self.duration - t;
            let length = self.speed * self.ramp + self.speed * (self.duration - 2.0 * self.ramp);
            (length - a * left * left / 2.0, a * left)
        } else {
            (
                a * self.ramp * self.ramp / 2.0 + self.speed * (t - self.ramp),
                self.speed,
            )
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Running {
    block: Block,
    start: f64,
    from: [f64; 3],
    profile: Profile,
}

impl Running {
    fn end(&self) -> f64 {
        match self.block {
            Block::Dwell { seconds } => self.start + seconds,
            _ => self.start + self.profile.duration,
        }
    }

    /// Where the machine is, and how fast it goes, at `now`.
    fn at(&self, now: f64) -> ([f64; 3], f64) {
        let (along, speed) = self.profile.at(now - self.start);
        (
            std::array::from_fn(|i| self.from[i] + self.profile.unit[i] * along),
            speed,
        )
    }
}

/// The emulated controller.
pub(crate) struct Machine {
    settings: GrblSettings,
    errors: GrblErrors,
    truth: Arc<TrueState>,
    noise_key: u64,
    line: Vec<u8>,
    waiting: VecDeque<String>,
    mode: Mode,
    inches: bool,
    relative: bool,
    rapid: bool,
    feed: Option<f64>,
    /// Where the machine is when idle, or where the running block started.
    position: [f64; 3],
    /// Where the queue ends.
    planned: [f64; 3],
    /// When the machine is free to start the next block.
    free: f64,
    queue: VecDeque<Block>,
    running: Option<Running>,
    held: Option<Block>,
    homed: bool,
    direction: [f64; 3],
    noise: [f64; 3],
    moves: u64,
}

fn seconds(now: Timestamp) -> f64 {
    now.0 as f64 / 1e9
}

impl Machine {
    pub(crate) fn new(
        settings: GrblSettings,
        errors: GrblErrors,
        seed: u64,
        truth: Arc<TrueState>,
    ) -> Self {
        Self {
            settings,
            errors,
            truth,
            noise_key: key(seed, 7),
            line: Vec::new(),
            waiting: VecDeque::new(),
            mode: Mode::Idle,
            inches: false,
            relative: false,
            rapid: true,
            feed: None,
            position: [0.0; 3],
            planned: [0.0; 3],
            free: 0.0,
            queue: VecDeque::new(),
            running: None,
            held: None,
            homed: false,
            direction: [0.0; 3],
            noise: [0.0; 3],
            moves: 0,
        }
    }

    /// What the controller prints when it starts, or after a reset with `alarm`.
    pub(crate) fn boot(&mut self, alarm: Option<u8>) -> Vec<String> {
        self.line.clear();
        self.waiting.clear();
        self.queue.clear();
        (self.running, self.held, self.feed) = (None, None, None);
        (self.inches, self.relative, self.rapid) = (false, false, true);
        self.planned = self.position;
        let locked = alarm.is_some() || (self.settings.on(22) && !self.homed);
        self.mode = if locked {
            Mode::Alarm(alarm)
        } else {
            Mode::Idle
        };
        let mut out: Vec<String> = alarm
            .map(|code| format!("ALARM:{code}"))
            .into_iter()
            .collect();
        out.extend([String::new(), BANNER.into()]);
        out.extend(locked.then(|| LOCKED.into()));
        out
    }

    /// Takes one byte from the link at `now`, and gives back the lines to send.
    pub(crate) fn byte(&mut self, byte: u8, now: Timestamp) -> Vec<String> {
        let (now, mut out) = (seconds(now), Vec::new());
        self.advance(now, &mut out);
        match byte {
            b'?' => out.push(self.status(now)),
            b'!' => self.hold(now),
            b'~' => self.resume(now),
            0x85 => self.cancel_jog(now),
            0x18 => {
                let moving = matches!(self.mode, Mode::Run | Mode::Jog | Mode::Home | Mode::Hold);
                self.stop(now);
                // A reset in motion loses the position, which homing must find again.
                self.homed &= !moving;
                out.extend(self.boot(moving.then_some(3)));
            }
            b'\n' | b'\r' => {
                let line = String::from_utf8_lossy(&std::mem::take(&mut self.line)).into_owned();
                match self.blocked() {
                    true => self.waiting.push_back(line),
                    false => self.execute(&line, now, &mut out),
                }
            }
            byte if byte.is_ascii_graphic() => self.line.push(byte.to_ascii_uppercase()),
            _ => {}
        }
        out
    }

    /// Brings the machine up to `now`: finishes blocks, takes lines that waited on a G4 or `$H`,
    /// and moves the true stage. It gives back the lines to send.
    pub(crate) fn tick(&mut self, now: Timestamp) -> Vec<String> {
        let (now, mut out) = (seconds(now), Vec::new());
        self.advance(now, &mut out);
        while !self.blocked() {
            let Some(line) = self.waiting.pop_front() else {
                break;
            };
            self.execute(&line, now, &mut out);
            self.advance(now, &mut out);
        }
        self.move_truth(now);
        out
    }

    /// Whether a G4 or `$H` holds up the lines after it.
    fn blocked(&self) -> bool {
        let syncs = |block: &Block| matches!(block, Block::Dwell { .. } | Block::Home);
        self.queue.iter().any(syncs) || self.running.is_some_and(|running| syncs(&running.block))
    }

    /// Runs the blocks due by `now`.
    fn advance(&mut self, now: f64, out: &mut Vec<String>) {
        loop {
            if self.running.is_none() {
                if matches!(self.mode, Mode::Hold | Mode::Alarm(_)) {
                    return;
                }
                let Some(block) = self.queue.pop_front() else {
                    if matches!(self.mode, Mode::Run | Mode::Jog | Mode::Home) {
                        self.mode = Mode::Idle;
                    }
                    return;
                };
                let target = match block {
                    Block::Move { target, .. } => target,
                    Block::Home => [0.0; 3],
                    Block::Dwell { .. } => self.position,
                };
                let speed = match block {
                    Block::Move { speed, .. } => speed,
                    _ => self.settings.get(25).unwrap_or(1000.0) / 60.0,
                };
                // An axis that moves takes up its backlash in its new direction.
                let moves = target.iter().zip(&self.position).zip(&mut self.direction);
                for ((to, from), direction) in moves.filter(|((to, from), _)| to != from) {
                    *direction = (to - from).signum();
                }
                let profile = Profile::new(self.position, target, speed, &self.settings);
                self.running = Some(Running {
                    block,
                    start: self.free,
                    from: self.position,
                    profile,
                });
                self.mode = match block {
                    Block::Move { jog: true, .. } => Mode::Jog,
                    Block::Home => Mode::Home,
                    _ => Mode::Run,
                };
            }
            let running = self.running.expect("a running block");
            if running.end() > now {
                return;
            }
            self.running = None;
            let (target, _) = running.at(running.end());
            self.free = running.end();
            if target != self.position {
                self.moves += 1;
                let sigma = self.errors.position_noise.0 / 1000.0;
                self.noise = std::array::from_fn(|axis| {
                    sigma * f64::from(gaussian(hash(self.noise_key, self.moves, axis as u64)))
                });
            }
            self.position = target;
            match running.block {
                Block::Dwell { .. } => out.push("ok".into()),
                Block::Home => {
                    (self.homed, self.position, self.planned) = (true, [0.0; 3], [0.0; 3]);
                    out.push("ok".into());
                }
                Block::Move { .. } => {}
            }
        }
    }

    /// Queues `block`, which starts at `now` if the machine is free.
    fn queue_block(&mut self, block: Block, now: f64) {
        if self.running.is_none() && self.queue.is_empty() && self.held.is_none() {
            self.free = now;
        }
        self.queue.push_back(block);
    }

    /// Stops any motion where it is at `now`.
    fn stop(&mut self, now: f64) {
        if let Some(running) = self.running.take() {
            self.position = running.at(now).0;
        }
        self.queue.clear();
        (self.held, self.planned, self.free) = (None, self.position, now);
    }

    fn hold(&mut self, now: f64) {
        match (self.mode, self.running) {
            (Mode::Jog, _) => self.cancel_jog(now),
            (
                Mode::Run,
                Some(
                    running @ Running {
                        block: Block::Move { .. },
                        ..
                    },
                ),
            ) => {
                self.position = running.at(now).0;
                (self.running, self.held, self.mode) = (None, Some(running.block), Mode::Hold);
            }
            _ => {}
        }
    }

    fn resume(&mut self, now: f64) {
        if self.mode == Mode::Hold {
            if let Some(rest) = self.held.take() {
                self.queue.push_front(rest);
            }
            (self.free, self.mode) = (now, Mode::Run);
        }
    }

    fn cancel_jog(&mut self, now: f64) {
        if self.mode == Mode::Jog {
            self.stop(now);
            self.mode = Mode::Idle;
        }
    }

    fn status(&self, now: f64) -> String {
        let (position, speed) = self
            .running
            .map_or((self.position, 0.0), |running| running.at(now));
        let state = match self.mode {
            Mode::Idle => "Idle",
            Mode::Run => "Run",
            Mode::Hold => "Hold:0",
            Mode::Jog => "Jog",
            Mode::Home => "Home",
            Mode::Alarm(_) => "Alarm",
        };
        // The controller counts steps, so it reports whole steps.
        let steps = |axis: usize| {
            (position[axis] * self.settings.axis(100, axis)).round() / self.settings.axis(100, axis)
                + 0.0
        };
        format!(
            "<{state}|MPos:{:.3},{:.3},{:.3}|FS:{:.0},0>",
            steps(0),
            steps(1),
            steps(2),
            speed * 60.0
        )
    }

    /// Runs one line: `ok`, `error:N`, or nothing yet for a G4 or `$H`, whose `ok` comes when
    /// it ends, and for a line that raised an alarm.
    fn execute(&mut self, line: &str, now: f64, out: &mut Vec<String>) {
        let reply = match line.strip_prefix('$') {
            _ if line.is_empty() => Ok(true),
            Some(command) => self.system(command, now, out),
            None => self.gcode(line, now, out),
        };
        match reply {
            Ok(true) => out.push("ok".into()),
            Ok(false) => {}
            Err(code) => out.push(format!("error:{code}")),
        }
    }

    fn system(&mut self, command: &str, now: f64, out: &mut Vec<String>) -> Result<bool, u8> {
        match command {
            "" => out.push(HELP.into()),
            "$" => out.extend(self.settings.listing()),
            "X" => {
                if matches!(self.mode, Mode::Alarm(_)) {
                    self.mode = Mode::Idle;
                    out.push("[MSG:Caution: Unlocked]".into());
                }
            }
            "H" if !self.settings.on(22) => return Err(5),
            "H" => {
                self.stop(now);
                self.queue_block(Block::Home, now);
                self.mode = Mode::Home;
                return Ok(false);
            }
            _ if command.starts_with("J=") => return self.jog(&command[2..], now),
            _ => {
                let (number, value) = command.split_once('=').ok_or(3u8)?;
                let number: u16 = number.parse().map_err(|_| 3u8)?;
                let value: f64 = value.parse().map_err(|_| 2u8)?;
                if !self.settings.set(number, value) {
                    return Err(3);
                }
            }
        }
        Ok(true)
    }

    fn jog(&mut self, line: &str, now: f64) -> Result<bool, u8> {
        if !matches!(self.mode, Mode::Idle | Mode::Jog) {
            return Err(9);
        }
        let (mut relative, mut inches, mut feed, mut axes) =
            (self.relative, self.inches, None, [None; 3]);
        for (letter, value) in words(line)? {
            match (letter, value) {
                ('G', 90.0) => relative = false,
                ('G', 91.0) => relative = true,
                ('G', 20.0) => inches = true,
                ('G', 21.0) => inches = false,
                ('G', 53.0) => {}
                ('F', _) => feed = Some(value),
                ('X' | 'Y' | 'Z', _) => axes[(letter as u8 - b'X') as usize] = Some(value),
                _ => return Err(20),
            }
        }
        let unit = if inches { 25.4 } else { 1.0 };
        let feed = feed.ok_or(22u8)? * unit / 60.0;
        let target = self.target(axes, relative, unit);
        if self.settings.on(20) && !self.within_travel(target) {
            return Err(15);
        }
        self.queue_block(
            Block::Move {
                target,
                speed: feed,
                jog: true,
            },
            now,
        );
        self.planned = target;
        Ok(true)
    }

    fn gcode(&mut self, line: &str, now: f64, out: &mut Vec<String>) -> Result<bool, u8> {
        if matches!(self.mode, Mode::Alarm(_) | Mode::Jog) {
            return Err(9);
        }
        let (mut axes, mut dwell, mut pause) = ([None; 3], false, None);
        for (letter, value) in words(line)? {
            match (letter, value) {
                ('G', 0.0) => self.rapid = true,
                ('G', 1.0) => self.rapid = false,
                ('G', 4.0) => dwell = true,
                ('G', 20.0) => self.inches = true,
                ('G', 21.0) => self.inches = false,
                ('G', 90.0) => self.relative = false,
                ('G', 91.0) => self.relative = true,
                ('G', 17.0 | 54.0 | 94.0) => {}
                ('F', _) => self.feed = Some(value * if self.inches { 25.4 } else { 1.0 }),
                ('P', _) => pause = Some(value),
                ('N', _) => {}
                ('X' | 'Y' | 'Z', _) => axes[(letter as u8 - b'X') as usize] = Some(value),
                _ => return Err(20),
            }
        }
        if dwell {
            self.queue_block(
                Block::Dwell {
                    seconds: pause.ok_or(28u8)?.max(0.0),
                },
                now,
            );
            return Ok(false);
        }
        if axes.iter().any(Option::is_some) {
            let unit = if self.inches { 25.4 } else { 1.0 };
            let speed = match self.rapid {
                true => f64::INFINITY,
                false => self.feed.ok_or(22u8)? / 60.0,
            };
            let target = self.target(axes, self.relative, unit);
            if self.settings.on(20) && !self.within_travel(target) {
                // A soft limit stops everything, keeping the position (ALARM:2).
                self.stop(now);
                self.mode = Mode::Alarm(Some(2));
                out.push("ALARM:2".into());
                return Ok(false);
            }
            self.queue_block(
                Block::Move {
                    target,
                    speed,
                    jog: false,
                },
                now,
            );
            self.planned = target;
        }
        Ok(true)
    }

    /// The target of the axis words `axes`, in whole steps.
    fn target(&self, axes: [Option<f64>; 3], relative: bool, unit: f64) -> [f64; 3] {
        std::array::from_fn(|axis| {
            let value = match axes[axis] {
                Some(value) if relative => self.planned[axis] + value * unit,
                Some(value) => value * unit,
                None => self.planned[axis],
            };
            let steps = self.settings.axis(100, axis);
            (value * steps).round() / steps
        })
    }

    /// Whether `target` is within the machine's travel: from minus `$13x` to 0 on each axis.
    fn within_travel(&self, target: [f64; 3]) -> bool {
        (0..3).all(|axis| (-self.settings.axis(130, axis) - 1e-9..=1e-9).contains(&target[axis]))
    }

    /// Moves the true stage: the commanded position, scaled, behind by half the backlash in the
    /// direction it last moved, plus the last move's random error.
    fn move_truth(&self, now: f64) {
        let (commanded, _) = self
            .running
            .map_or((self.position, 0.0), |running| running.at(now));
        let truth: [Micrometers; 3] = std::array::from_fn(|axis| {
            let backlash = self.errors.backlash[axis].0 / 1000.0;
            let mm = commanded[axis] * (1.0 + self.errors.scale_error)
                - self.direction[axis] * backlash / 2.0
                + self.noise[axis];
            Micrometers(mm * 1000.0)
        });
        self.truth.set_stage(truth[0], truth[1], truth[2]);
    }
}

/// The words of a line: a letter and a number each. Spaces were dropped as the line came in;
/// comments in parentheses are dropped here.
fn words(line: &str) -> Result<Vec<(char, f64)>, u8> {
    let mut text = String::with_capacity(line.len());
    let mut depth = 0;
    for c in line.chars() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            ';' if depth == 0 => break,
            _ if depth == 0 => text.push(c),
            _ => {}
        }
    }
    let mut words = Vec::new();
    let mut rest = text.as_str();
    while let Some(letter) = rest.chars().next() {
        if !letter.is_ascii_alphabetic() {
            return Err(1);
        }
        let number = &rest[1..];
        let end = number
            .find(|c: char| c.is_ascii_alphabetic())
            .unwrap_or(number.len());
        let value: f64 = number[..end].parse().map_err(|_| 2u8)?;
        words.push((letter, value));
        rest = &number[end..];
    }
    Ok(words)
}
