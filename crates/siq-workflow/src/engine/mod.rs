//! The workflow engine (ADR-0052): one executor for every acquisition plan.
//!
//! - **A [`Plan`]** is its dataset's name and steps: moves of the stage and the focus, channels,
//!   snaps and waits, autofocus and focus maps (ADR-0054), and repeats of steps that number the
//!   frames' `t`, `z` or `p`. A group of steps may have an error policy of its own. A tile scan
//!   (ADR-0053) is made into its moves, focusing and snaps when the run starts, by the calibration
//!   in force. It is serde, with a JSON Schema.
//! - **Sequences** (ADR-0055) compose: z-stacks about the focus, multiple positions, and
//!   time-lapses on a schedule, each around any steps. `p` numbers every position and tile of a
//!   plan in order, `z` every stack's planes from the bottom, and `t` a time-lapse's points.
//! - **[`run`]** runs it on a [`DeviceSet`](crate::DeviceSet):
//!   - it makes the plan's dataset, with the plan, the instrument and the calibration in force in
//!     its manifest, and records each snap there, keyed by the indices and the channel;
//!   - it reports [`Progress`] and other [`RunEvent`]s as it goes;
//!   - between steps it waits while paused, and stops when cancelled. A move under way is
//!     abandoned, a capture finishes; then the stage and the focus are halted;
//!   - a failed step is tried again, skipped or the end of the run, by its [`ErrorPolicy`], or
//!     the run waits for [`RunControl::decide`] to say which.

mod run;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use siq_types::ChannelId;
use tokio::sync::watch;

use crate::autofocus::SurfaceKind;
use crate::scan::{Order, Region};

pub use run::{
    DatasetTarget, FocusMap, LAYOUT, Layout, Outcome, Placed, RunError, RunReport, ScanLayout,
    TimeLapseRecord, TimePoint, resolve, run,
};

/// What to acquire: a dataset's name, and the steps that fill it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Plan {
    /// The dataset's name.
    pub name: String,
    /// The steps, in order.
    pub steps: Vec<Step>,
    /// What a failed step does, unless a group of steps has its own: by default, it ends the run.
    #[serde(default)]
    pub on_error: ErrorPolicy,
}

/// What a step that fails does.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ErrorPolicy {
    /// How many times it is tried again.
    #[serde(default)]
    pub retries: u32,
    /// Whether it is then skipped, rather than ending the run.
    #[serde(default)]
    pub skip: bool,
    /// Whether the run then waits to be told: tried again, skipped or the run's end
    /// ([`RunControl::decide`]), whatever `skip` says.
    #[serde(default)]
    pub ask: bool,
}

/// A step of a plan.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "step", rename_all = "snake_case")]
pub enum Step {
    /// Moves the stage to `x`, `y` and, if given, `z`, in µm, from the same side each time.
    MoveTo {
        /// Where, in µm.
        x: f64,
        /// Where, in µm.
        y: f64,
        /// The coarse z, in µm; where it is if not given.
        #[serde(default)]
        z: Option<f64>,
    },
    /// Moves the focus drive to `z` µm.
    Focus {
        /// Where, in µm.
        z: f64,
    },
    /// Puts a configured channel in place.
    Channel {
        /// The channel.
        id: ChannelId,
    },
    /// Takes a frame, recorded in the dataset at the indices and channel in force.
    Snap,
    /// Waits `ms` milliseconds.
    Wait {
        /// How long, in ms.
        ms: f64,
    },
    /// Runs `steps` `count` times, numbering the frames' `index` with each time, from 0.
    Repeat {
        /// How many times.
        count: u32,
        /// The index the times number: `t`, `z` or `p`; none if not given.
        #[serde(default)]
        index: Option<Index>,
        /// The steps.
        steps: Vec<Step>,
    },
    /// Sets `index` to `value` for the steps after it, among the steps it is in: to number the
    /// places of a plan's moves, say. It isn't a step of the device's.
    Index {
        /// The index: `t`, `z` or `p`.
        index: Index,
        /// Its value.
        value: u32,
    },
    /// Covers `region` with tiles (ADR-0053), numbering them `p`: at each, the stage moved and
    /// settled, the tile focused, then a frame in each channel. One channel is put in place once,
    /// before the first tile; several, at each.
    TileScan {
        /// The region, in stage µm.
        region: Region,
        /// The overlap between neighbours, of the frame: 0.1 if not given.
        #[serde(default = "overlap")]
        overlap: f64,
        /// The order the tiles are visited in: a snake if not given.
        #[serde(default)]
        order: Order,
        /// The channels, in order: the one in use if none.
        #[serde(default)]
        channels: Vec<ChannelId>,
        /// How long the stage settles after each move, in ms.
        #[serde(default)]
        settle_ms: f64,
        /// How each tile is focused: as the focus is, if not given.
        #[serde(default)]
        focus: TileFocus,
        /// A z-stack at each tile, about its focus, in place of one frame.
        #[serde(default)]
        z_stack: Option<ZPlanes>,
    },
    /// Runs `steps` at focus planes about where the focus is, numbering them `z` from the bottom;
    /// then the focus goes back to the middle.
    ZStack {
        /// The planes.
        planes: ZPlanes,
        /// What to take at each: a frame if none.
        #[serde(default)]
        steps: Vec<Step>,
    },
    /// Runs `steps` at each of `positions`, numbering them `p`: the stage moved there, the focus to
    /// the position's own if it has one, then an autofocus if asked. The places of its steps,
    /// moves, regions and focus maps, are about each position.
    MultiPosition {
        /// The positions, in order.
        positions: Vec<Position>,
        /// An autofocus at each, over this range in µm, if given.
        #[serde(default)]
        autofocus: Option<f64>,
        /// What to take at each: a frame if none.
        #[serde(default)]
        steps: Vec<Step>,
    },
    /// Runs `steps` at time points `interval_s` apart from the first, numbering them `t`: `count`
    /// of them, or as many as begin within `duration_s`, whichever is fewer. A point due while the
    /// last still runs is late, and `overrun` decides what it does.
    TimeLapse {
        /// The time between points' starts, in s.
        interval_s: f64,
        /// How many points.
        #[serde(default)]
        count: Option<u32>,
        /// How long, in s.
        #[serde(default)]
        duration_s: Option<f64>,
        /// What a late point does: skipped once its interval has passed, if not given.
        #[serde(default)]
        overrun: Overrun,
        /// What to do at each point: a frame if none.
        #[serde(default)]
        steps: Vec<Step>,
    },
    /// Autofocuses where the stage is (ADR-0054), over `range_um` about where the focus is. With
    /// nothing to focus on, the step fails.
    Autofocus {
        /// The range searched, in µm: 50 if not given.
        #[serde(default = "range")]
        range_um: f64,
    },
    /// Autofocuses at each of `places`, and fits a surface to the focus found there: the surface
    /// in force for the steps after it. A place with nothing to focus on is left out; three must
    /// remain.
    FocusMap {
        /// The places, `[x, y]` in stage µm.
        places: Vec<[f64; 2]>,
        /// The surface: a thin-plate spline if not given.
        #[serde(default)]
        surface: SurfaceKind,
        /// Each autofocus's range, in µm: 50 if not given.
        #[serde(default = "range")]
        range_um: f64,
    },
    /// Moves the focus to the surface in force at `x`, `y`.
    FocusOnSurface {
        /// Where, in µm.
        x: f64,
        /// Where, in µm.
        y: f64,
    },
    /// Runs `steps` with an error policy of their own.
    Policy {
        /// What a failed step of theirs does.
        on_error: ErrorPolicy,
        /// The steps.
        steps: Vec<Step>,
    },
}

/// An index of a dataset's frames, beside the channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Index {
    /// The time point.
    T,
    /// The focus plane.
    Z,
    /// The stage position.
    P,
}

/// A tile scan's overlap if not given.
fn overlap() -> f64 {
    0.1
}

/// The most planes a z-stack takes.
pub const MAX_PLANES: u32 = 1000;

/// A z-stack's planes: `range_um` across, a whole number of `step_um` steps, centred on the
/// focus, taken going `direction`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ZPlanes {
    /// From the lowest plane to the highest, in µm, at most: a whole number of steps.
    pub range_um: f64,
    /// Between planes, in µm.
    pub step_um: f64,
    /// The way the planes are taken: up if not given.
    #[serde(default)]
    pub direction: ZDirection,
}

impl ZPlanes {
    /// Each plane's `z` index and offset from the middle, in µm, in the order taken.
    pub fn planes(&self) -> Vec<(u32, f64)> {
        let steps = self.range_um / self.step_um + 1e-9;
        let n = match self.step_um > 0.0 && steps.is_finite() && steps >= 0.0 {
            true => (steps.floor() + 1.0).min(f64::from(MAX_PLANES)) as u32,
            false => 0,
        };
        let offset = |k: u32| (f64::from(k) - f64::from(n - 1) / 2.0) * self.step_um;
        let up = (0..n).map(|k| (k, offset(k)));
        match self.direction {
            ZDirection::Up => up.collect(),
            ZDirection::Down => up.rev().collect(),
        }
    }
}

/// The way a z-stack's planes are taken.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ZDirection {
    /// From the lowest: each plane approached from below, as the focus drive arrives.
    #[default]
    Up,
    /// From the highest: each plane overshot and approached from below.
    Down,
}

/// A position of a multi-position.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Position {
    /// Its name.
    #[serde(default)]
    pub name: String,
    /// Where, in stage µm.
    pub x: f64,
    /// Where, in stage µm.
    pub y: f64,
    /// The coarse z, in µm: where it is if not given.
    #[serde(default)]
    pub z: Option<f64>,
    /// The focus, in µm: where it is if not given.
    #[serde(default)]
    pub focus: Option<f64>,
}

/// What a time-lapse's late point does.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Overrun {
    /// It runs late within its own interval, and is skipped once that has passed: the schedule
    /// stays as it was.
    #[default]
    Skip,
    /// It runs at once, and the points after it keep the interval from it: the schedule slides.
    Shift,
}

impl Step {
    /// A time-lapse's points: `count`, or as many as begin within `duration_s`, whichever is fewer.
    fn points(count: Option<u32>, duration_s: Option<f64>, interval_s: f64) -> u32 {
        let within = duration_s.map(|d| (d / interval_s - 1e-9).ceil().clamp(1.0, 1e9) as u32);
        match (count, within) {
            (Some(n), Some(m)) => n.min(m),
            (Some(n), None) | (None, Some(n)) => n,
            (None, None) => 0,
        }
    }
}

/// An autofocus's range if not given.
fn range() -> f64 {
    50.0
}

/// How each tile of a scan is focused.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum TileFocus {
    /// As the focus is.
    #[default]
    None,
    /// At `z` µm, for every tile.
    Fixed {
        /// Where, in µm.
        z: f64,
    },
    /// On the surface in force: a focus map's, made before.
    Surface,
    /// Autofocused at each tile.
    Autofocus {
        /// The range searched, in µm.
        range_um: f64,
    },
}

/// The number of device steps in `steps`, each repeat counted out. A tile scan counts once
/// [`resolve`]d.
pub fn count(steps: &[Step]) -> u64 {
    steps
        .iter()
        .map(|step| match step {
            Step::Repeat { count, steps, .. } => {
                u64::from(*count).saturating_mul(self::count(steps))
            }
            Step::Policy { steps, .. } => self::count(steps),
            Step::FocusMap { places, .. } => 2 * places.len() as u64,
            // Each plane's focus and steps, a frame if none, then the focus back to the middle.
            Step::ZStack { planes, steps } => (planes.planes().len() as u64)
                .saturating_mul(1 + self::count(steps).max(u64::from(steps.is_empty())))
                .saturating_add(1),
            Step::TimeLapse {
                interval_s,
                count,
                duration_s,
                steps,
                ..
            } => u64::from(Step::points(*count, *duration_s, *interval_s))
                .saturating_mul(self::count(steps).max(u64::from(steps.is_empty()))),
            Step::Index { .. } | Step::TileScan { .. } | Step::MultiPosition { .. } => 0,
            _ => 1,
        })
        .fold(0, u64::saturating_add)
}

/// How far a run has gone.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Progress {
    /// The steps done.
    pub done: u64,
    /// The steps in the plan.
    pub total: u64,
    /// The time left, in s, at the pace so far.
    pub eta_s: Option<f64>,
    /// The step just done.
    pub step: String,
}

/// What a failed step does next.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Then {
    /// It is tried again.
    Retry,
    /// It is skipped.
    Skip,
    /// The run ends.
    Stop,
    /// The run waits to be told which.
    Ask,
}

/// What a run waiting on a failed step is told to do with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    /// Try it again.
    Retry,
    /// Skip it.
    Skip,
    /// End the run.
    Stop,
}

/// What happens in a run, as it happens.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum RunEvent {
    /// A step was done.
    Progress {
        /// How far the run has gone.
        progress: Progress,
    },
    /// A frame was taken, for the dataset at these indices.
    Frame {
        /// The time point.
        t: u32,
        /// The channel.
        c: u16,
        /// The focus plane.
        z: u32,
        /// The stage position.
        p: u32,
    },
    /// The run paused before its next step.
    Paused,
    /// The run went on.
    Resumed,
    /// A step failed.
    Problem {
        /// The step.
        step: String,
        /// Its attempt, from 1.
        attempt: u32,
        /// Why.
        error: String,
        /// What happens next.
        then: Then,
    },
}

/// What a run is asked to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Command {
    /// Go on.
    Run,
    /// Wait before the next step.
    Pause,
    /// Stop.
    Cancel,
}

/// Pauses, resumes and cancels a run, from anywhere: its clones share it.
#[derive(Debug, Clone)]
pub struct RunControl {
    commands: std::sync::Arc<watch::Sender<Command>>,
    decisions: std::sync::Arc<watch::Sender<Option<Decision>>>,
}

impl Default for RunControl {
    fn default() -> Self {
        Self {
            commands: std::sync::Arc::new(watch::Sender::new(Command::Run)),
            decisions: std::sync::Arc::new(watch::Sender::new(None)),
        }
    }
}

impl RunControl {
    /// A control that lets the run go on.
    pub fn new() -> Self {
        Self::default()
    }

    /// What the run is asked to do now.
    pub fn command(&self) -> Command {
        *self.commands.borrow()
    }

    /// Pauses the run before its next step, unless it is cancelled.
    pub fn pause(&self) {
        self.commands.send_if_modified(|now| {
            let pause = *now == Command::Run;
            if pause {
                *now = Command::Pause;
            }
            pause
        });
    }

    /// Lets a paused run go on.
    pub fn resume(&self) {
        self.commands.send_if_modified(|now| {
            let resume = *now == Command::Pause;
            if resume {
                *now = Command::Run;
            }
            resume
        });
    }

    /// Stops the run.
    pub fn cancel(&self) {
        self.commands.send_replace(Command::Cancel);
    }

    /// Tells a run waiting on a failed step what to do with it.
    pub fn decide(&self, decision: Decision) {
        self.decisions.send_replace(Some(decision));
    }

    /// Forgets what it was told before a step failed.
    fn forget(&self) {
        self.decisions.send_replace(None);
    }

    /// Waits to be told what to do with a failed step, and takes it: `None` if the run is
    /// cancelled first.
    async fn decision(&self) -> Option<Decision> {
        let mut decisions = self.decisions.subscribe();
        let told = tokio::select! {
            told = decisions.wait_for(Option::is_some) => told.ok().and_then(|told| *told),
            () = self.cancelled() => None,
        };
        self.forget();
        told
    }

    /// Resolves once the run is cancelled.
    pub async fn cancelled(&self) {
        let mut commands = self.commands.subscribe();
        let _ = commands
            .wait_for(|command| *command == Command::Cancel)
            .await;
    }

    /// Resolves once the run isn't paused: whether it may go on.
    async fn unpaused(&self) -> bool {
        let mut commands = self.commands.subscribe();
        let command = commands
            .wait_for(|command| *command != Command::Pause)
            .await;
        command.is_ok_and(|command| *command == Command::Run)
    }
}
