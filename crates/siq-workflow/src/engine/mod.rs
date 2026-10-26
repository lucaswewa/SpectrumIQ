//! The workflow engine (ADR-0052): one executor for every acquisition plan.
//!
//! - **A [`Plan`]** is its dataset's name and steps: moves of the stage and the focus, channels,
//!   snaps and waits, and repeats of steps that number the frames' `t`, `z` or `p`. A group of
//!   steps may have an error policy of its own. A tile scan (ADR-0053) is made into its moves and
//!   snaps when the run starts, by the calibration in force. It is serde, with a JSON Schema.
//! - **[`run`]** runs it on a [`DeviceSet`](crate::DeviceSet):
//!   - it makes the plan's dataset, with the plan, the instrument and the calibration in force in
//!     its manifest, and records each snap there, keyed by the indices and the channel;
//!   - it reports [`Progress`] and other [`RunEvent`]s as it goes;
//!   - between steps it waits while paused, and stops when cancelled. A move under way is
//!     abandoned, a capture finishes; then the stage and the focus are halted;
//!   - a failed step is tried again, skipped or the end of the run, by its [`ErrorPolicy`].

mod run;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use siq_types::ChannelId;
use tokio::sync::watch;

use crate::scan::{Order, Region};

pub use run::{DatasetTarget, LAYOUT, Outcome, RunError, RunReport, resolve, run};

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
    /// settled, then a frame in each channel. One channel is put in place once, before the first
    /// tile; several, at each.
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
        /// The focus for every tile, in µm: where it is if not given.
        #[serde(default)]
        focus: Option<f64>,
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
            Step::Index { .. } | Step::TileScan { .. } => 0,
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
pub struct RunControl(std::sync::Arc<watch::Sender<Command>>);

impl Default for RunControl {
    fn default() -> Self {
        Self(std::sync::Arc::new(watch::Sender::new(Command::Run)))
    }
}

impl RunControl {
    /// A control that lets the run go on.
    pub fn new() -> Self {
        Self::default()
    }

    /// What the run is asked to do now.
    pub fn command(&self) -> Command {
        *self.0.borrow()
    }

    /// Pauses the run before its next step, unless it is cancelled.
    pub fn pause(&self) {
        self.0.send_if_modified(|now| {
            let pause = *now == Command::Run;
            if pause {
                *now = Command::Pause;
            }
            pause
        });
    }

    /// Lets a paused run go on.
    pub fn resume(&self) {
        self.0.send_if_modified(|now| {
            let resume = *now == Command::Pause;
            if resume {
                *now = Command::Run;
            }
            resume
        });
    }

    /// Stops the run.
    pub fn cancel(&self) {
        self.0.send_replace(Command::Cancel);
    }

    /// Resolves once the run is cancelled.
    pub async fn cancelled(&self) {
        let mut commands = self.0.subscribe();
        let _ = commands
            .wait_for(|command| *command == Command::Cancel)
            .await;
    }

    /// Resolves once the run isn't paused: whether it may go on.
    async fn unpaused(&self) -> bool {
        let mut commands = self.0.subscribe();
        let command = commands
            .wait_for(|command| *command != Command::Pause)
            .await;
        command.is_ok_and(|command| *command == Command::Run)
    }
}
