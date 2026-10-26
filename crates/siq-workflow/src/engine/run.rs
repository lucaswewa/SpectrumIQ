//! Running a plan: its dataset, its steps, and how it ends.

use std::collections::{HashSet, VecDeque};
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use siq_buffer::Policy;
use siq_devices::{Approach, DeviceError, MoveOpts, Xyz};
use siq_storage::{
    Catalog, Dataset, DatasetKind, DatasetWriter, FrameKey, NewDataset, Snapshot, StorageError,
    WriterOptions,
};
use siq_types::{DatasetId, Micrometers, Timestamp};

use super::{ErrorPolicy, Plan, Progress, RunControl, RunEvent, Step, Then, count};
use crate::DeviceSet;
use crate::routines::channel;

/// Where a run's dataset goes, and what it records it was made with.
#[derive(Debug, Clone)]
pub struct DatasetTarget {
    /// The data root: the dataset goes in its `datasets/`.
    pub data_root: PathBuf,
    /// The bytes to leave free on the disk.
    pub min_free: u64,
    /// The catalog to list the dataset in, when its frames are written.
    pub catalog: Option<Arc<Catalog>>,
    /// The instrument's configuration.
    pub instrument: Option<Snapshot>,
}

/// How a run ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// Every step was done, or skipped.
    Finished,
    /// It was cancelled.
    Cancelled,
    /// A step failed, and its policy ended the run.
    Failed,
}

/// What a run did.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RunReport {
    /// Its dataset.
    pub dataset: DatasetId,
    /// The dataset's folder.
    pub path: PathBuf,
    /// How it ended.
    pub outcome: Outcome,
    /// Why it failed, if it did.
    pub error: Option<String>,
    /// The steps done, and in the plan.
    pub steps: [u64; 2],
    /// The frames recorded.
    pub frames: u64,
}

/// Why a run didn't start.
#[derive(Debug, thiserror::Error)]
pub enum RunError {
    /// The plan can't be run.
    #[error("the plan can't be run: {0}")]
    Invalid(String),
    /// Its dataset couldn't be made, or recorded to.
    #[error(transparent)]
    Storage(#[from] StorageError),
}

/// Why the steps stopped.
enum Stop {
    Cancelled,
    Failed(String),
}

type Steps<'a> = Pin<Box<dyn Future<Output = Result<(), Stop>> + Send + 'a>>;

/// A run under way: the indices in force, and how far it has gone.
struct Runner<'a> {
    devices: &'a DeviceSet,
    control: &'a RunControl,
    observe: &'a (dyn Fn(RunEvent) + Send + Sync),
    /// The keys of the frames snapped, in order, for the writer.
    keys: Arc<Mutex<VecDeque<FrameKey>>>,
    /// Every key taken: a frame has one of its own.
    taken: HashSet<FrameKey>,
    /// `t`, `z` and `p`.
    indices: [u32; 3],
    channel: u16,
    done: u64,
    total: u64,
    began: Timestamp,
}

/// Runs `plan` on `devices`, its dataset in `target`, as `control` asks, telling `observe` what
/// happens. It ends when the plan does, when it's cancelled, or when a failed step's policy
/// says so: the report says which. Once the dataset is made, it is always reported.
pub async fn run(
    devices: &DeviceSet,
    target: &DatasetTarget,
    plan: &Plan,
    control: &RunControl,
    observe: &(dyn Fn(RunEvent) + Send + Sync),
) -> Result<RunReport, RunError> {
    validate(devices, plan)?;
    let mut new = NewDataset::new(DatasetKind::Sequence, &plan.name);
    new.instrument = target.instrument.clone();
    let calibration = devices.acquisition.context().calibration;
    let record = calibration.and_then(|id| devices.calibrations.get(id));
    let snapshot = record.map(|record| Snapshot::of(&record)).transpose();
    new.calibration = snapshot.map_err(|error| RunError::Invalid(error.to_string()))?;
    new.plan = Some(serde_json::to_value(plan).map_err(|e| RunError::Invalid(e.to_string()))?);
    let dataset = Dataset::create(&target.data_root, new)?;

    let keys = Arc::new(Mutex::new(VecDeque::new()));
    let frames = devices
        .acquisition
        .subscribe("workflow", Policy::Lossless, |meta| meta.flags.save);
    let mut options = WriterOptions::new(target.min_free);
    options.catalog = target.catalog.clone();
    let queued = Arc::clone(&keys);
    options.key = Box::new(move |meta| {
        let next = queued
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front();
        next.unwrap_or(FrameKey {
            t: 0,
            c: meta.channel.map_or(0, |channel| channel.0),
            z: 0,
            p: 0,
        })
    });
    let writer = DatasetWriter::start(&dataset, frames, options)?;

    let mut runner = Runner {
        devices,
        control,
        observe,
        keys,
        taken: HashSet::new(),
        indices: [0; 3],
        channel: 0,
        done: 0,
        total: count(&plan.steps),
        began: devices.clock.now(),
    };
    let ended = runner.steps(&plan.steps, plan.on_error).await;
    if ended.is_err() {
        // However it stopped, the stage and focus are still.
        let (stage, focus) = tokio::join!(devices.stage.stop(), devices.focus.stop());
        for (what, result) in [("stage", stage), ("focus", focus)] {
            if let Err(error) = result {
                tracing::warn!(%error, "the {what} didn't stop");
            }
        }
    }
    let written = tokio::task::spawn_blocking(move || writer.finish()).await;
    let written = written.map_err(|error| RunError::Invalid(error.to_string()))?;
    let (outcome, error) = match (ended, written.error) {
        (Ok(()), None) => (Outcome::Finished, None),
        (Err(Stop::Cancelled), None) => (Outcome::Cancelled, None),
        (Err(Stop::Failed(why)), _) => (Outcome::Failed, Some(why)),
        (_, Some(error)) => (Outcome::Failed, Some(error.to_string())),
    };
    Ok(RunReport {
        dataset: dataset.manifest().id,
        path: dataset.path().to_owned(),
        outcome,
        error,
        steps: [runner.done, runner.total],
        frames: written.stats.frames,
    })
}

/// Refuses a plan that can't run: an unknown channel, no steps, or a wait of no length.
fn validate(devices: &DeviceSet, plan: &Plan) -> Result<(), RunError> {
    fn check(devices: &DeviceSet, steps: &[Step]) -> Result<(), String> {
        for step in steps {
            match step {
                Step::Channel { id } if !devices.channels.iter().any(|c| c.id == *id) => {
                    return Err(format!("there is no channel {id}"));
                }
                Step::Wait { ms } if !ms.is_finite() || *ms < 0.0 => {
                    return Err(format!("a wait of {ms} ms"));
                }
                Step::Repeat { steps, .. } | Step::Policy { steps, .. } => check(devices, steps)?,
                _ => {}
            }
        }
        Ok(())
    }
    if plan.name.trim().is_empty() || count(&plan.steps) == 0 {
        return Err(RunError::Invalid("it needs a name, and a step".to_owned()));
    }
    check(devices, &plan.steps).map_err(RunError::Invalid)
}

impl Runner<'_> {
    /// Runs `steps`, a failed one doing as `policy` says.
    fn steps<'s>(&'s mut self, steps: &'s [Step], policy: ErrorPolicy) -> Steps<'s> {
        Box::pin(async move {
            for step in steps {
                match step {
                    Step::Repeat {
                        count,
                        index,
                        steps,
                    } => {
                        // Its index counts within it, and is as it was after it.
                        let before = self.indices;
                        for time in 0..*count {
                            if let Some(index) = index {
                                self.indices[*index as usize] = time;
                            }
                            self.steps(steps, policy).await?;
                        }
                        self.indices = before;
                    }
                    Step::Policy { on_error, steps } => self.steps(steps, *on_error).await?,
                    Step::Index { index, value } => self.indices[*index as usize] = *value,
                    step => self.step(step, policy).await?,
                }
            }
            Ok(())
        })
    }

    /// Runs one device step once the run may go on, trying it again or skipping it as `policy`
    /// says if it fails.
    async fn step(&mut self, step: &Step, policy: ErrorPolicy) -> Result<(), Stop> {
        if self.control.command() == super::Command::Pause {
            (self.observe)(RunEvent::Paused);
            let go_on = self.control.unpaused().await;
            if go_on {
                (self.observe)(RunEvent::Resumed);
            }
        }
        if self.control.command() == super::Command::Cancel {
            return Err(Stop::Cancelled);
        }
        let what = self.describe(step);
        for attempt in 1.. {
            let error = match self.act(step).await {
                Ok(()) => break,
                Err(Stop::Cancelled) => return Err(Stop::Cancelled),
                Err(Stop::Failed(error)) => error,
            };
            let then = match (attempt <= policy.retries, policy.skip) {
                (true, _) => Then::Retry,
                (false, true) => Then::Skip,
                (false, false) => Then::Stop,
            };
            let step = what.clone();
            let problem = RunEvent::Problem {
                step,
                attempt,
                error: error.clone(),
                then,
            };
            (self.observe)(problem);
            match then {
                Then::Retry => continue,
                Then::Skip => break,
                Then::Stop => return Err(Stop::Failed(format!("{what}: {error}"))),
            }
        }
        self.done += 1;
        let elapsed = self.devices.clock.now().saturating_since(self.began);
        let left = (self.total - self.done) as f64;
        let progress = Progress {
            done: self.done,
            total: self.total,
            eta_s: Some(elapsed.as_secs_f64() / self.done as f64 * left),
            step: what,
        };
        (self.observe)(RunEvent::Progress { progress });
        Ok(())
    }

    /// What `step` does, for the progress and the problems.
    fn describe(&self, step: &Step) -> String {
        let [t, z, p] = self.indices;
        match step {
            Step::MoveTo { x, y, z: Some(h) } => format!("move to ({x}, {y}, {h}) µm"),
            Step::MoveTo { x, y, z: None } => format!("move to ({x}, {y}) µm"),
            Step::Focus { z } => format!("focus to {z} µm"),
            Step::Channel { id } => format!("channel {id}"),
            Step::Snap => format!("snap t{t} c{} z{z} p{p}", self.channel),
            Step::Wait { ms } => format!("wait {ms} ms"),
            Step::Repeat { .. } | Step::Policy { .. } | Step::Index { .. } => String::new(),
        }
    }

    /// Does `step` once. A move is abandoned if the run is cancelled meanwhile; a channel, a
    /// capture and a wait finish first.
    async fn act(&mut self, step: &Step) -> Result<(), Stop> {
        let devices = self.devices;
        let failed = |error: DeviceError| Stop::Failed(error.to_string());
        let opts = MoveOpts {
            speed: None,
            approach: Approach::Unidirectional,
        };
        match step {
            Step::MoveTo { x, y, z } => {
                let at = devices.stage.status().borrow().position;
                let target = Xyz {
                    x: Micrometers(*x),
                    y: Micrometers(*y),
                    z: z.map_or(at.z, Micrometers),
                };
                let moved = self
                    .unless_cancelled(devices.stage.move_to(target, opts))
                    .await?;
                moved.map(drop).map_err(failed)
            }
            Step::Focus { z } => {
                let focus = devices.focus.move_to(Micrometers(*z), opts);
                self.unless_cancelled(focus)
                    .await?
                    .map(drop)
                    .map_err(failed)
            }
            Step::Channel { id } => {
                let found = devices.channels.iter().find(|channel| channel.id == *id);
                let config = found.ok_or_else(|| Stop::Failed(format!("no channel {id}")))?;
                let applied = channel::apply(devices, config).await;
                applied.map_err(|error| Stop::Failed(error.to_string()))?;
                self.channel = id.0;
                Ok(())
            }
            Step::Snap => {
                let [t, z, p] = self.indices;
                let c = self.channel;
                let key = FrameKey { t, c, z, p };
                if !self.taken.insert(key) {
                    let why = format!("a frame was already taken at {key}: number the snaps");
                    return Err(Stop::Failed(why));
                }
                let keys = &self.keys;
                let queue = || keys.lock().unwrap_or_else(PoisonError::into_inner);
                queue().push_back(key);
                match devices.acquisition.snap(true).await {
                    Ok(_) => {
                        (self.observe)(RunEvent::Frame { t, c, z, p });
                        Ok(())
                    }
                    Err(error) => {
                        queue().pop_back();
                        self.taken.remove(&key);
                        Err(failed(error))
                    }
                }
            }
            Step::Wait { ms } => {
                tokio::time::sleep(Duration::from_secs_f64(ms / 1000.0)).await;
                Ok(())
            }
            Step::Repeat { .. } | Step::Policy { .. } | Step::Index { .. } => Ok(()),
        }
    }

    /// `operation`'s result, unless the run is cancelled first.
    async fn unless_cancelled<T>(&self, operation: impl Future<Output = T>) -> Result<T, Stop> {
        tokio::select! {
            done = operation => Ok(done),
            () = self.control.cancelled() => Err(Stop::Cancelled),
        }
    }
}
