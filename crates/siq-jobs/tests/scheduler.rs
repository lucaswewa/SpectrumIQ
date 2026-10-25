//! The scheduler (ADR-0043): admission within the budgets, priority and the starvation guard,
//! cancellation, a job's record in its dataset, an estimate exceeded, and below-normal threads.
//! REQ-3D-005, REQ-NFR-002.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use siq_jobs::*;
use siq_storage::{Dataset, DatasetKind, NewDataset, ProcessingState};

const GB: u64 = 1_000_000_000;
const WAIT: Duration = Duration::from_secs(10);

/// A job that runs until let go, or cancelled.
struct Held {
    kind: &'static str,
    estimate: Resources,
    go: Arc<AtomicBool>,
    used: Option<Resources>,
    fail: bool,
    priorities: Arc<Mutex<Vec<i32>>>,
}

impl Job for Held {
    fn kind(&self) -> &str {
        self.kind
    }

    fn params(&self) -> serde_json::Value {
        serde_json::json!({ "ram": self.estimate.ram })
    }

    fn estimate(&self) -> Resources {
        self.estimate
    }

    fn run(&mut self, ctx: &JobCtx) -> Result<JobOutput, JobError> {
        let pool = ctx.pool().install(this_thread_priority);
        *self.priorities.lock().unwrap() = vec![this_thread_priority(), pool];
        if let Some(used) = self.used {
            ctx.used(used);
        }
        ctx.log("working");
        while !self.go.load(Ordering::Acquire) {
            ctx.check()?;
            thread::sleep(Duration::from_millis(2));
        }
        ctx.progress(0.5);
        if self.fail {
            return Err(JobError::failed("the disk is full"));
        }
        Ok(JobOutput {
            outputs: vec![format!("zarr/{}.ome.zarr", self.kind)],
        })
    }
}

/// A job of `ram` bytes, and the switch that lets it finish.
fn held(kind: &'static str, ram: u64) -> (Held, Arc<AtomicBool>) {
    let go = Arc::new(AtomicBool::new(false));
    let job = Held {
        kind,
        estimate: Resources {
            ram,
            vram: 0,
            gpu: false,
            threads: 1,
        },
        go: go.clone(),
        used: None,
        fail: false,
        priorities: Arc::default(),
    };
    (job, go)
}

fn scheduler(ram: u64, vram: u64, starvation: Duration) -> Scheduler {
    let mut config = SchedulerConfig::new(ram, vram);
    config.threads = 2;
    config.starvation = starvation;
    Scheduler::new(config, None).unwrap()
}

fn state(jobs: &Scheduler, id: JobId) -> JobState {
    jobs.info(id).unwrap().state
}

fn kinds(infos: &[JobInfo]) -> Vec<String> {
    infos.iter().map(|info| info.kind.clone()).collect()
}

#[test]
fn jobs_are_admitted_only_within_the_ram_and_vram_budgets() {
    let jobs = scheduler(10 * GB, 4 * GB, Duration::from_secs(60));
    let (a, go_a) = held("a", 6 * GB);
    let (b, go_b) = held("b", 6 * GB);
    let (c, go_c) = held("c", 3 * GB);
    let [a, b, c] = [a, b, c].map(|job| jobs.submit(Box::new(job), Priority::Normal, None));
    // a runs; b doesn't fit beside it; c, smaller, does.
    assert_eq!(
        [state(&jobs, a), state(&jobs, b), state(&jobs, c)],
        [JobState::Running, JobState::Queued, JobState::Running]
    );
    assert_eq!(jobs.in_use().ram, 9 * GB);
    go_a.store(true, Ordering::Release);
    jobs.wait(a, WAIT).unwrap();
    assert_eq!(
        state(&jobs, b),
        JobState::Running,
        "b starts once a has ended"
    );
    assert_eq!(jobs.in_use().ram, 9 * GB);

    // One GPU job at a time, and within the VRAM budget.
    let (mut g1, go_g1) = held("g1", 0);
    let (mut g2, go_g2) = held("g2", 0);
    (g1.estimate.gpu, g1.estimate.vram) = (true, GB);
    (g2.estimate.gpu, g2.estimate.vram) = (true, GB);
    let g1 = jobs.submit(Box::new(g1), Priority::Normal, None);
    let g2 = jobs.submit(Box::new(g2), Priority::Normal, None);
    assert_eq!(
        (state(&jobs, g1), state(&jobs, g2)),
        (JobState::Running, JobState::Queued)
    );
    for go in [go_b, go_c, go_g1, go_g2] {
        go.store(true, Ordering::Release);
    }
    for id in [b, c, g1, g2] {
        assert_eq!(jobs.wait(id, WAIT).unwrap().state, JobState::Done);
    }
    assert_eq!(jobs.in_use(), Resources::default());
}

#[test]
fn priority_goes_first_then_submission_order_and_a_starved_job_stops_overtaking() {
    let jobs = scheduler(10 * GB, 0, Duration::from_millis(200));
    let (holder, go_holder) = held("holder", 8 * GB);
    let holder = jobs.submit(Box::new(holder), Priority::Normal, None);

    let (big, go_big) = held("big", 5 * GB);
    let big = jobs.submit(Box::new(big), Priority::Normal, None);
    // Not starved yet: a small job behind it starts at once.
    let (small, go_small) = held("small", GB);
    let small = jobs.submit(Box::new(small), Priority::Normal, None);
    assert_eq!(state(&jobs, small), JobState::Running);
    go_small.store(true, Ordering::Release);
    jobs.wait(small, WAIT).unwrap();

    // Once big has waited its starvation time, nothing behind it starts.
    thread::sleep(Duration::from_millis(250));
    let (later, go_later) = held("later", GB);
    let later = jobs.submit(Box::new(later), Priority::Normal, None);
    assert_eq!(state(&jobs, later), JobState::Queued);
    // Queued in priority order, then submission order.
    let (urgent, go_urgent) = held("urgent", 4 * GB);
    let (background, go_background) = held("background", GB);
    let background = jobs.submit(Box::new(background), Priority::Background, None);
    let urgent = jobs.submit(Box::new(urgent), Priority::Interactive, None);
    assert_eq!(
        kinds(&jobs.snapshot().queued),
        ["urgent", "big", "later", "background"]
    );

    go_holder.store(true, Ordering::Release);
    jobs.wait(holder, WAIT).unwrap();
    // 10 GB free: urgent (4) first, then big (5); later and background (1 each) after big.
    assert_eq!(kinds(&jobs.snapshot().running), ["urgent", "big", "later"]);
    assert_eq!(kinds(&jobs.snapshot().queued), ["background"]);
    for go in [go_big, go_later, go_urgent, go_background] {
        go.store(true, Ordering::Release);
    }
    for id in [big, later, urgent, background] {
        assert_eq!(jobs.wait(id, WAIT).unwrap().state, JobState::Done);
    }
}

#[test]
fn a_job_bigger_than_the_budget_runs_alone() {
    let jobs = scheduler(10 * GB, 0, Duration::from_secs(60));
    let (other, go_other) = held("other", GB);
    let other = jobs.submit(Box::new(other), Priority::Normal, None);
    let (huge, go_huge) = held("huge", 20 * GB);
    let huge = jobs.submit(Box::new(huge), Priority::Normal, None);
    assert_eq!(state(&jobs, huge), JobState::Queued);
    go_other.store(true, Ordering::Release);
    jobs.wait(other, WAIT).unwrap();
    assert_eq!(state(&jobs, huge), JobState::Running);
    go_huge.store(true, Ordering::Release);
    assert_eq!(jobs.wait(huge, WAIT).unwrap().state, JobState::Done);
}

#[test]
fn cancelling_takes_a_queued_job_out_and_stops_a_running_one() {
    let jobs = scheduler(4 * GB, 0, Duration::from_secs(60));
    let (running, _) = held("running", 3 * GB);
    let (queued, _) = held("queued", 3 * GB);
    let running = jobs.submit(Box::new(running), Priority::Normal, None);
    let queued = jobs.submit(Box::new(queued), Priority::Normal, None);
    assert!(jobs.cancel(queued));
    let info = jobs.info(queued).unwrap();
    assert_eq!(
        (info.state, info.started),
        (JobState::Cancelled, None),
        "it never ran"
    );
    assert!(jobs.cancel(running));
    assert_eq!(jobs.wait(running, WAIT).unwrap().state, JobState::Cancelled);
    assert_eq!(jobs.in_use(), Resources::default());
    assert!(!jobs.cancel(running), "it has ended");
    assert!(!jobs.cancel(JobId(99)));
    assert_eq!(jobs.snapshot(), JobsSnapshot::default());
}

#[test]
fn a_job_on_a_dataset_is_recorded_in_its_processing_history() {
    let root = tempfile::tempdir().unwrap();
    let dataset = Dataset::create(root.path(), NewDataset::new(DatasetKind::Snap, "s")).unwrap();
    let jobs = scheduler(10 * GB, 0, Duration::from_secs(60));
    let (pyramid, go) = held("pyramid", GB);
    go.store(true, Ordering::Release);
    let (mut broken, go_broken) = held("export", GB);
    broken.fail = true;
    go_broken.store(true, Ordering::Release);
    let path = Some(dataset.path().to_owned());
    let done = jobs.submit(Box::new(pyramid), Priority::Normal, path.clone());
    let failed = jobs.submit(Box::new(broken), Priority::Normal, path);
    assert_eq!(jobs.wait(done, WAIT).unwrap().state, JobState::Done);
    let failure = jobs.wait(failed, WAIT).unwrap();
    assert_eq!(
        (failure.state, failure.error.as_deref()),
        (JobState::Failed, Some("the disk is full"))
    );

    let history = Dataset::open(dataset.path())
        .unwrap()
        .manifest()
        .history
        .clone();
    assert_eq!(history.len(), 2, "{history:?}");
    let pyramid = history.iter().find(|r| r.kind == "pyramid").unwrap();
    assert_eq!(pyramid.state, ProcessingState::Done);
    assert_eq!(
        (pyramid.progress, pyramid.params["ram"].as_u64()),
        (1.0, Some(GB))
    );
    assert_eq!(pyramid.outputs, ["zarr/pyramid.ome.zarr"]);
    assert!(pyramid.started.is_some() && pyramid.finished >= pyramid.started);
    let export = history.iter().find(|r| r.kind == "export").unwrap();
    assert_eq!(
        (export.state, export.error.as_deref()),
        (ProcessingState::Failed, Some("the disk is full"))
    );
}

#[test]
fn a_job_over_its_estimate_goes_on_and_is_counted() {
    let jobs = scheduler(10 * GB, 0, Duration::from_secs(60));
    let (mut job, go) = held("greedy", GB);
    job.used = Some(Resources {
        ram: 3 * GB,
        ..Resources::default()
    });
    let id = jobs.submit(Box::new(job), Priority::Normal, None);
    go.store(true, Ordering::Release);
    let info = jobs.wait(id, WAIT).unwrap();
    assert_eq!((info.state, info.exceeded), (JobState::Done, true));
    assert_eq!(info.message.as_deref(), Some("working"));
    assert_eq!(jobs.estimates_exceeded(), 1);
}

#[test]
fn jobs_and_their_pool_run_below_normal_priority() {
    let jobs = scheduler(10 * GB, 0, Duration::from_secs(60));
    let (job, go) = held("low", GB);
    let priorities = job.priorities.clone();
    go.store(true, Ordering::Release);
    let id = jobs.submit(Box::new(job), Priority::Normal, None);
    jobs.wait(id, WAIT).unwrap();
    // THREAD_PRIORITY_BELOW_NORMAL, on the job's thread and on the pool's.
    assert_eq!(*priorities.lock().unwrap(), [-1, -1]);
    assert_eq!(
        this_thread_priority(),
        0,
        "the caller's thread is left alone"
    );
}

#[test]
fn shutting_down_cancels_what_is_queued_and_stops_what_runs() {
    let jobs = scheduler(4 * GB, 0, Duration::from_secs(60));
    let (running, _) = held("running", 3 * GB);
    let (queued, _) = held("queued", 3 * GB);
    let running = jobs.submit(Box::new(running), Priority::Normal, None);
    let queued = jobs.submit(Box::new(queued), Priority::Normal, None);
    jobs.shutdown();
    assert_eq!(state(&jobs, running), JobState::Cancelled);
    assert_eq!(state(&jobs, queued), JobState::Cancelled);
    let (late, _) = held("late", GB);
    let late = jobs.submit(Box::new(late), Priority::Normal, None);
    assert_eq!(
        state(&jobs, late),
        JobState::Cancelled,
        "nothing starts once stopped"
    );
}
