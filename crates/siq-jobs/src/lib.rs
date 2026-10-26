//! The job system (ADR-0043): heavy processing, such as pyramids, stitching or exports, run in the
//! background without disturbing acquisition, within the memory budget (architecture §5).
//!
//! - A [`Job`] says what it needs at its peak ([`Resources`]), then runs with a [`JobCtx`]: its
//!   progress, whether it was cancelled, a log, its dataset, and a thread pool.
//! - The [`Scheduler`] queues jobs by [`Priority`], first come first served within one, and
//!   admits each only while the running jobs' RAM and VRAM, with its own, stay within the
//!   budgets; one GPU job at a time. A job that has waited too long stops others overtaking it.
//! - Jobs run on threads of below-normal priority, and so does their thread pool.
//! - A job on a dataset is recorded in its manifest's processing history.
//! - [`kinds`] has the jobs the service runs: the snap's Zarr writer, pyramids, the DZI and
//!   OME-TIFF exports, and the shading estimate ([`ShadingJob`](kinds::ShadingJob)).
//!
//! The unsafe code is in `priority`, a small Windows-API module (ADR-0007).

pub mod kinds;
mod priority;
mod scheduler;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub use priority::this_thread_priority;
pub use scheduler::{JobsSnapshot, Scheduler, SchedulerConfig};

/// What a job needs at its peak.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Resources {
    /// RAM, in bytes.
    pub ram: u64,
    /// GPU memory, in bytes.
    pub vram: u64,
    /// Whether it uses the GPU: one such job runs at a time.
    pub gpu: bool,
    /// The threads it can keep busy, for information.
    pub threads: usize,
}

/// How soon a job should run. A higher one goes first; within one, the first submitted.
#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Priority {
    /// Exports and other work nobody waits for.
    Background,
    /// Processing after acquisition.
    #[default]
    Normal,
    /// Work someone is waiting to see, such as a snap's pyramid.
    Interactive,
}

/// A job's number, unique in the scheduler.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(transparent)]
pub struct JobId(pub u64);

impl std::fmt::Display for JobId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "job-{}", self.0)
    }
}

/// Where a job is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    /// Waiting to be admitted.
    Queued,
    /// Running.
    Running,
    /// Finished.
    Done,
    /// Ended with an error.
    Failed,
    /// Cancelled, before it ran or while it ran.
    Cancelled,
}

impl JobState {
    /// Whether the job is over.
    pub fn is_finished(self) -> bool {
        matches!(self, Self::Done | Self::Failed | Self::Cancelled)
    }
}

/// What is known of a job.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct JobInfo {
    /// Its number.
    pub id: JobId,
    /// What it is.
    pub kind: String,
    /// How soon it should run.
    pub priority: Priority,
    /// Where it is.
    pub state: JobState,
    /// How far it got, from 0 to 1.
    pub progress: f64,
    /// What it said it needs.
    pub estimate: Resources,
    /// Whether it used more than it said: it was let go on, and counted.
    pub exceeded: bool,
    /// The dataset it works on.
    pub dataset: Option<PathBuf>,
    /// When it was submitted.
    pub queued: DateTime<Utc>,
    /// When it started.
    pub started: Option<DateTime<Utc>>,
    /// When it finished, failed or was cancelled.
    pub finished: Option<DateTime<Utc>>,
    /// The last thing it logged.
    pub message: Option<String>,
    /// Why it failed.
    pub error: Option<String>,
}

/// What a job made.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct JobOutput {
    /// What it wrote, as paths in its dataset, for the dataset's record.
    pub outputs: Vec<String>,
}

/// Why a job didn't finish.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum JobError {
    /// It was cancelled, and stopped.
    #[error("cancelled")]
    Cancelled,
    /// It failed.
    #[error("{0}")]
    Failed(String),
}

impl JobError {
    /// A failure, from any error.
    pub fn failed(error: impl std::fmt::Display) -> Self {
        Self::Failed(error.to_string())
    }
}

/// Heavy processing, run by the [`Scheduler`].
pub trait Job: Send {
    /// What it is: `pyramid`, `stitch`, `export_dzi`.
    fn kind(&self) -> &str;

    /// The version of what runs, for the dataset's record.
    fn version(&self) -> &str {
        env!("CARGO_PKG_VERSION")
    }

    /// Its parameters, for the dataset's record.
    fn params(&self) -> serde_json::Value {
        serde_json::Value::Null
    }

    /// What it will need at its peak: it is admitted only when that fits the budgets.
    fn estimate(&self) -> Resources;

    /// Does the work, checking [`JobCtx::check`] often enough to stop soon when cancelled.
    fn run(&mut self, ctx: &JobCtx) -> Result<JobOutput, JobError>;
}

/// What a running job is given.
pub struct JobCtx {
    id: JobId,
    kind: String,
    dataset: Option<PathBuf>,
    pool: Arc<rayon::ThreadPool>,
    cancelled: Arc<AtomicBool>,
    report: Arc<dyn Fn(Report) + Send + Sync>,
    progress: AtomicU64,
}

/// What a job tells the scheduler as it runs.
enum Report {
    Progress(f64),
    Message(String),
    Used(Resources),
}

impl JobCtx {
    /// Its number.
    pub fn id(&self) -> JobId {
        self.id
    }

    /// The dataset it works on, if any: its folder.
    pub fn dataset(&self) -> Option<&Path> {
        self.dataset.as_deref()
    }

    /// The jobs' thread pool, of below-normal priority, for parallel work: `ctx.pool().install(…)`.
    pub fn pool(&self) -> &rayon::ThreadPool {
        &self.pool
    }

    /// Whether it has been asked to stop.
    pub fn cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }

    /// `Err(Cancelled)` once it has been asked to stop: `ctx.check()?` between steps.
    pub fn check(&self) -> Result<(), JobError> {
        if self.cancelled() {
            Err(JobError::Cancelled)
        } else {
            Ok(())
        }
    }

    /// How far it has got, from 0 to 1. Watchers hear of each hundredth.
    pub fn progress(&self, fraction: f64) {
        let fraction = fraction.clamp(0.0, 1.0);
        let last = f64::from_bits(self.progress.load(Ordering::Relaxed));
        if fraction - last >= 0.01 || (fraction == 1.0 && last < 1.0) {
            self.progress.store(fraction.to_bits(), Ordering::Relaxed);
            (self.report)(Report::Progress(fraction));
        }
    }

    /// Says what it is doing: in the log, and as its last message.
    pub fn log(&self, message: impl Into<String>) {
        let message = message.into();
        tracing::info!(job = %self.id, kind = %self.kind, "{message}");
        (self.report)(Report::Message(message));
    }

    /// Says what it used at its peak, for the scheduler to compare with its estimate.
    pub fn used(&self, used: Resources) {
        (self.report)(Report::Used(used));
    }
}
