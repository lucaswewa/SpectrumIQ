//! The scheduler (ADR-0043): a priority queue, first come first served within a priority, whose
//! jobs are admitted only within the RAM and VRAM budgets, one GPU job at a time.
//!
//! Admission is decided whenever a job is submitted, finishes or is cancelled. A job that doesn't
//! fit is skipped, so smaller ones behind it start (backfill), until it has waited
//! [`starvation`](SchedulerConfig::starvation): then nothing behind it starts until it does. A job
//! bigger than the budgets starts alone, once nothing else runs.

use std::collections::VecDeque;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use chrono::Utc;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use siq_storage::{Dataset, ProcessingRecord, ProcessingState};
use siq_telemetry::{Counter, Registry};
use tokio::sync::watch;

use crate::priority::lower_this_thread;
use crate::{Job, JobCtx, JobError, JobId, JobInfo, JobState, Priority, Report, Resources};

/// How a [`Scheduler`] admits and runs jobs.
#[derive(Debug, Clone, PartialEq)]
pub struct SchedulerConfig {
    /// The RAM that running jobs may use together, in bytes.
    pub ram_budget: u64,
    /// The GPU memory that running jobs may use together, in bytes.
    pub vram_budget: u64,
    /// The threads of the jobs' pool.
    pub threads: usize,
    /// How long a job may wait while jobs behind it, which fit, start before it.
    pub starvation: Duration,
    /// How many finished jobs are kept, to look up.
    pub history: usize,
}

impl SchedulerConfig {
    /// The budgets, in bytes, with half the processors for the pool (architecture §4.3), a
    /// minute's starvation guard, and the last 100 finished jobs kept.
    pub fn new(ram_budget: u64, vram_budget: u64) -> Self {
        let processors = thread::available_parallelism().map_or(2, |n| n.get());
        Self {
            ram_budget,
            vram_budget,
            threads: (processors / 2).max(1),
            starvation: Duration::from_secs(60),
            history: 100,
        }
    }
}

/// The jobs waiting and running: what the `jobs` Thing shows.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct JobsSnapshot {
    /// The jobs waiting, in the order they will be considered.
    pub queued: Vec<JobInfo>,
    /// The jobs running, oldest first.
    pub running: Vec<JobInfo>,
}

/// Runs jobs in the background, within the budgets.
pub struct Scheduler {
    inner: Arc<Inner>,
}

struct Inner {
    config: SchedulerConfig,
    state: Mutex<State>,
    finished: Condvar,
    pool: Arc<rayon::ThreadPool>,
    snapshot: watch::Sender<JobsSnapshot>,
    exceeded: Arc<Counter>,
    // One manifest update at a time, for jobs on the same dataset.
    manifests: Mutex<()>,
}

#[derive(Default)]
struct State {
    next: u64,
    queued: Vec<Entry>,
    running: Vec<Entry>,
    finished: VecDeque<JobInfo>,
    used: Resources,
    stopping: bool,
}

struct Entry {
    info: JobInfo,
    record: String,
    job: Option<Box<dyn Job>>,
    params: serde_json::Value,
    version: String,
    cancelled: Arc<AtomicBool>,
    since: Instant,
    thread: Option<JoinHandle<()>>,
}

impl Scheduler {
    /// A scheduler with `config`'s budgets. Its count of jobs that used more than their estimate
    /// is `jobs.estimates_exceeded` in `registry`.
    pub fn new(config: SchedulerConfig, registry: Option<&Registry>) -> Result<Self, String> {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(config.threads)
            .thread_name(|i| format!("jobs-{i}"))
            .start_handler(|_| lower_this_thread())
            .build()
            .map_err(|error| error.to_string())?;
        let exceeded = match registry {
            Some(registry) => registry
                .counter("jobs.estimates_exceeded")
                .map_err(|error| error.to_string())?,
            None => Arc::new(Counter::default()),
        };
        Ok(Self {
            inner: Arc::new(Inner {
                config,
                state: Mutex::new(State::default()),
                finished: Condvar::new(),
                pool: Arc::new(pool),
                snapshot: watch::Sender::new(JobsSnapshot::default()),
                exceeded,
                manifests: Mutex::new(()),
            }),
        })
    }

    /// Queues `job`, on the dataset in folder `dataset` if any, and starts it if it fits.
    pub fn submit(&self, job: Box<dyn Job>, priority: Priority, dataset: Option<PathBuf>) -> JobId {
        let inner = &self.inner;
        let mut state = inner.lock();
        state.next += 1;
        let id = JobId(state.next);
        let entry = Entry {
            info: JobInfo {
                id,
                kind: job.kind().to_owned(),
                priority,
                state: JobState::Queued,
                progress: 0.0,
                estimate: job.estimate(),
                exceeded: false,
                dataset,
                queued: Utc::now(),
                started: None,
                finished: None,
                message: None,
                error: None,
            },
            record: format!("{}-{}", Utc::now().format("%Y%m%dT%H%M%S%.6f"), id.0),
            params: job.params(),
            version: job.version().to_owned(),
            job: Some(job),
            cancelled: Arc::new(AtomicBool::new(false)),
            since: Instant::now(),
            thread: None,
        };
        if state.stopping {
            let mut info = entry.info;
            info.state = JobState::Cancelled;
            info.finished = Some(Utc::now());
            inner.retire(&mut state, info);
        } else {
            state.queued.push(entry);
            inner.admit(&mut state);
        }
        inner.publish(&state);
        id
    }

    /// Cancels job `id`: a queued one at once; a running one is asked to stop, and is cancelled
    /// when it does. False if it isn't queued or running.
    pub fn cancel(&self, id: JobId) -> bool {
        let inner = &self.inner;
        let mut state = inner.lock();
        if let Some(at) = state.queued.iter().position(|entry| entry.info.id == id) {
            let mut info = state.queued.remove(at).info;
            info.state = JobState::Cancelled;
            info.finished = Some(Utc::now());
            inner.retire(&mut state, info);
            // The queue changed: a job behind it may start.
            inner.admit(&mut state);
            inner.publish(&state);
            return true;
        }
        let running = state.running.iter().find(|entry| entry.info.id == id);
        running.is_some_and(|entry| {
            entry.cancelled.store(true, Ordering::Release);
            true
        })
    }

    /// Job `id` as it is now: queued, running, or among the last finished.
    pub fn info(&self, id: JobId) -> Option<JobInfo> {
        let state = self.inner.lock();
        state
            .queued
            .iter()
            .chain(&state.running)
            .map(|entry| &entry.info)
            .chain(&state.finished)
            .find(|info| info.id == id)
            .cloned()
    }

    /// Waits up to `timeout` for job `id` to finish: how it ended, or `None` if it hasn't.
    pub fn wait(&self, id: JobId, timeout: Duration) -> Option<JobInfo> {
        let deadline = Instant::now() + timeout;
        let mut state = self.inner.lock();
        loop {
            if let Some(info) = state.finished.iter().find(|info| info.id == id) {
                return Some(info.clone());
            }
            let left = deadline.checked_duration_since(Instant::now())?;
            state = self
                .inner
                .finished
                .wait_timeout(state, left)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }

    /// The jobs waiting and running.
    pub fn snapshot(&self) -> JobsSnapshot {
        self.inner.snapshot.borrow().clone()
    }

    /// The jobs waiting and running, as they change.
    pub fn watch(&self) -> watch::Receiver<JobsSnapshot> {
        self.inner.snapshot.subscribe()
    }

    /// The jobs that used more than their estimate.
    pub fn estimates_exceeded(&self) -> u64 {
        self.inner.exceeded.get()
    }

    /// The resources the running jobs' estimates add up to.
    pub fn in_use(&self) -> Resources {
        self.inner.lock().used
    }

    /// Stops: the queued jobs are cancelled, the running ones asked to stop and waited for.
    pub fn shutdown(&self) {
        let inner = &self.inner;
        let threads: Vec<JoinHandle<()>> = {
            let mut state = inner.lock();
            state.stopping = true;
            for entry in std::mem::take(&mut state.queued) {
                let mut info = entry.info;
                info.state = JobState::Cancelled;
                info.finished = Some(Utc::now());
                inner.retire(&mut state, info);
            }
            for entry in &mut state.running {
                entry.cancelled.store(true, Ordering::Release);
            }
            inner.publish(&state);
            let threads = state.running.iter_mut().filter_map(|e| e.thread.take());
            threads.collect()
        };
        for thread in threads {
            let _ = thread.join();
        }
    }
}

impl Drop for Scheduler {
    fn drop(&mut self) {
        self.shutdown();
    }
}

impl std::fmt::Debug for Scheduler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Scheduler")
            .field("config", &self.inner.config)
            .field("jobs", &self.snapshot())
            .finish()
    }
}

impl Inner {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Starts every queued job that fits, in priority order, as the module describes.
    fn admit(self: &Arc<Self>, state: &mut State) {
        if state.stopping {
            return;
        }
        let order = |a: &Entry, b: &Entry| {
            (b.info.priority.cmp(&a.info.priority)).then(a.info.id.cmp(&b.info.id))
        };
        state.queued.sort_by(order);
        let config = &self.config;
        let mut at = 0;
        while at < state.queued.len() {
            let entry = &state.queued[at];
            let (need, used) = (entry.info.estimate, state.used);
            let too_big = need.ram > config.ram_budget || need.vram > config.vram_budget;
            let fits = if too_big {
                state.running.is_empty()
            } else {
                used.ram + need.ram <= config.ram_budget
                    && used.vram + need.vram <= config.vram_budget
                    && !(need.gpu && used.gpu)
            };
            if fits {
                let entry = state.queued.remove(at);
                self.start(state, entry);
            } else if entry.since.elapsed() >= config.starvation {
                break;
            } else {
                at += 1;
            }
        }
    }

    fn start(self: &Arc<Self>, state: &mut State, mut entry: Entry) {
        let need = entry.info.estimate;
        state.used.ram += need.ram;
        state.used.vram += need.vram;
        state.used.gpu |= need.gpu;
        entry.info.state = JobState::Running;
        entry.info.started = Some(Utc::now());
        let id = entry.info.id;
        let ctx = JobCtx {
            id,
            kind: entry.info.kind.clone(),
            dataset: entry.info.dataset.clone(),
            pool: Arc::clone(&self.pool),
            cancelled: Arc::clone(&entry.cancelled),
            report: {
                let inner = Arc::clone(self);
                Arc::new(move |report| inner.report(id, report))
            },
            progress: AtomicU64::new(0f64.to_bits()),
        };
        let job = entry.job.take().expect("a queued job has its work");
        let record = (
            entry.record.clone(),
            entry.params.clone(),
            entry.version.clone(),
        );
        let inner = Arc::clone(self);
        let info = entry.info.clone();
        let spawned = thread::Builder::new()
            .name(id.to_string())
            .spawn(move || inner.run(job, &ctx, info, record));
        match spawned {
            Ok(thread) => entry.thread = Some(thread),
            Err(error) => tracing::error!(job = %id, %error, "a job's thread couldn't start"),
        }
        state.running.push(entry);
    }

    /// The job's thread: records it, runs it, and retires it.
    fn run(
        self: Arc<Self>,
        mut job: Box<dyn Job>,
        ctx: &JobCtx,
        started: JobInfo,
        (record, params, version): (String, serde_json::Value, String),
    ) {
        lower_this_thread();
        let write = |info: &JobInfo, outputs: Vec<String>, results: serde_json::Value| {
            let Some(path) = &info.dataset else { return };
            let _one = self
                .manifests
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            let entry = ProcessingRecord {
                id: record.clone(),
                kind: info.kind.clone(),
                version: version.clone(),
                params: params.clone(),
                inputs: Vec::new(),
                outputs,
                state: match info.state {
                    JobState::Queued => ProcessingState::Queued,
                    JobState::Running => ProcessingState::Running,
                    JobState::Done => ProcessingState::Done,
                    JobState::Failed => ProcessingState::Failed,
                    JobState::Cancelled => ProcessingState::Cancelled,
                },
                progress: info.progress,
                started: info.started,
                finished: info.finished,
                error: info.error.clone(),
                results,
            };
            let updated = Dataset::open(path).and_then(|mut dataset| {
                dataset.update(|manifest| {
                    match manifest.history.iter_mut().find(|r| r.id == entry.id) {
                        Some(existing) => *existing = entry,
                        None => manifest.history.push(entry),
                    }
                })
            });
            if let Err(error) = updated {
                tracing::warn!(job = %info.id, %error, "the job's record wasn't written");
            }
        };
        write(&started, Vec::new(), serde_json::Value::Null);

        let result = catch_unwind(AssertUnwindSafe(|| job.run(ctx)))
            .unwrap_or_else(|_| Err(JobError::Failed("the job panicked".to_owned())));
        let (mut outputs, mut results) = (Vec::new(), serde_json::Value::Null);
        let (state, error) = match result {
            Ok(output) => {
                (outputs, results) = (output.outputs, output.results);
                (JobState::Done, None)
            }
            Err(JobError::Cancelled) => (JobState::Cancelled, None),
            Err(JobError::Failed(why)) => (JobState::Failed, Some(why)),
        };
        // As it is now, with its progress and messages, then as it ended.
        let current = {
            let shared = self.lock();
            let entry = shared.running.iter().find(|e| e.info.id == started.id);
            entry.map_or(started, |entry| entry.info.clone())
        };
        let mut info = JobInfo {
            state,
            error,
            finished: Some(Utc::now()),
            ..current
        };
        if state == JobState::Done {
            info.progress = 1.0;
        }
        match info.state {
            JobState::Failed => tracing::warn!(job = %info.id, kind = %info.kind,
                error = info.error.as_deref().unwrap_or_default(), "a job failed"),
            state => tracing::info!(job = %info.id, kind = %info.kind, ?state, "a job ended"),
        }
        // Recorded first, so that a finished job is a recorded one.
        write(&info, outputs, results);
        let mut shared = self.lock();
        if let Some(at) = shared.running.iter().position(|e| e.info.id == info.id) {
            shared.running.remove(at);
        }
        let need = info.estimate;
        shared.used.ram -= need.ram;
        shared.used.vram -= need.vram;
        shared.used.gpu &= !need.gpu;
        self.retire(&mut shared, info);
        self.admit(&mut shared);
        self.publish(&shared);
    }

    fn report(&self, id: JobId, report: Report) {
        let mut state = self.lock();
        let Some(entry) = state.running.iter_mut().find(|e| e.info.id == id) else {
            return;
        };
        let info = &mut entry.info;
        match report {
            Report::Progress(fraction) => info.progress = fraction,
            Report::Message(message) => info.message = Some(message),
            Report::Used(used) => {
                let over = used.ram > info.estimate.ram || used.vram > info.estimate.vram;
                if over && !info.exceeded {
                    info.exceeded = true;
                    self.exceeded.increment();
                    tracing::warn!(job = %id, kind = %info.kind, estimate = ?info.estimate,
                        ?used, "a job used more than its estimate; it goes on");
                }
            }
        }
        self.publish(&state);
    }

    /// Keeps a finished job's info, and wakes those waiting for it.
    fn retire(&self, state: &mut State, info: JobInfo) {
        state.finished.push_back(info);
        while state.finished.len() > self.config.history {
            state.finished.pop_front();
        }
        self.finished.notify_all();
    }

    fn publish(&self, state: &State) {
        let infos = |entries: &[Entry]| entries.iter().map(|e| e.info.clone()).collect();
        self.snapshot.send_replace(JobsSnapshot {
            queued: infos(&state.queued),
            running: infos(&state.running),
        });
    }
}
