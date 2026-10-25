//! Synthetic jobs through the scheduler, within the workstation's budgets (14 GB of RAM, 4 GB of
//! VRAM): which run, which wait and why, as each one starts and ends.
//!
//! ```text
//! cargo run -p siq-jobs --example jobs_demo
//! ```
//!
//! It takes about 3 s; smoke mode (`SIQ_SMOKE=1`) runs ten times faster.

use std::time::{Duration, Instant};

use rayon::prelude::*;
use siq_jobs::{
    Job, JobCtx, JobError, JobOutput, JobsSnapshot, Priority, Resources, Scheduler, SchedulerConfig,
};

const GB: u64 = 1_000_000_000;

/// A job that keeps the pool busy for `seconds`, reporting its progress.
struct Synthetic {
    kind: &'static str,
    estimate: Resources,
    seconds: f64,
}

impl Job for Synthetic {
    fn kind(&self) -> &str {
        self.kind
    }

    fn estimate(&self) -> Resources {
        self.estimate
    }

    fn run(&mut self, ctx: &JobCtx) -> Result<JobOutput, JobError> {
        let started = Instant::now();
        let length = Duration::from_secs_f64(self.seconds);
        while started.elapsed() < length {
            ctx.check()?;
            // A little parallel work on the jobs' pool, at below-normal priority.
            let sum: u64 = ctx.pool().install(|| (0..200_000u64).into_par_iter().sum());
            std::hint::black_box(sum);
            ctx.progress(started.elapsed().as_secs_f64() / self.seconds);
        }
        Ok(JobOutput::default())
    }
}

fn job(kind: &'static str, ram: u64, vram: u64, seconds: f64) -> Box<dyn Job> {
    let estimate = Resources {
        ram: ram * GB,
        vram: vram * GB,
        gpu: vram > 0,
        threads: 4,
    };
    Box::new(Synthetic {
        kind,
        estimate,
        seconds,
    })
}

fn show(at: f64, jobs: &JobsSnapshot) {
    let list = |infos: &[siq_jobs::JobInfo]| {
        let names: Vec<String> = infos
            .iter()
            .map(|info| format!("{} ({} GB)", info.kind, info.estimate.ram / GB))
            .collect();
        if names.is_empty() {
            "-".to_owned()
        } else {
            names.join(", ")
        }
    };
    println!(
        "{at:5.2} s  running: {:<52} waiting: {}",
        list(&jobs.running),
        list(&jobs.queued)
    );
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let pace = if std::env::var_os("SIQ_SMOKE").is_some() {
        0.1
    } else {
        1.0
    };
    let mut config = SchedulerConfig::new(14 * GB, 4 * GB);
    config.starvation = Duration::from_secs_f64(0.8 * pace);
    let jobs = Scheduler::new(config, None)?;
    let mut changes = jobs.watch();
    let started = Instant::now();

    let ids = [
        jobs.submit(job("stitch", 8, 0, 1.2 * pace), Priority::Normal, None),
        jobs.submit(job("dff", 3, 3, 0.8 * pace), Priority::Normal, None),
        jobs.submit(job("deconvolve", 2, 2, 0.6 * pace), Priority::Normal, None),
        jobs.submit(job("export", 5, 0, 0.6 * pace), Priority::Background, None),
        jobs.submit(
            job("pyramid", 4, 0, 0.5 * pace),
            Priority::Interactive,
            None,
        ),
        jobs.submit(job("huge", 20, 0, 0.4 * pace), Priority::Normal, None),
    ];
    println!("budgets: 14 GB RAM, 4 GB VRAM, one GPU job at a time");
    let mut last = JobsSnapshot::default();
    loop {
        let now = changes.borrow_and_update().clone();
        // Progress alone isn't news here: only who runs and who waits.
        let who = |jobs: &JobsSnapshot| {
            let ids = |infos: &[siq_jobs::JobInfo]| infos.iter().map(|i| i.id).collect::<Vec<_>>();
            (ids(&jobs.running), ids(&jobs.queued))
        };
        if who(&now) != who(&last) {
            show(started.elapsed().as_secs_f64(), &now);
            last = now;
        }
        if ids
            .iter()
            .all(|&id| jobs.info(id).is_some_and(|info| info.state.is_finished()))
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    for id in ids {
        let info = jobs.info(id).expect("kept");
        let (queued, started, finished) = (info.queued, info.started, info.finished);
        let waited = started.map_or(0.0, |s| (s - queued).as_seconds_f64());
        let ran = started
            .zip(finished)
            .map_or(0.0, |(s, f)| (f - s).as_seconds_f64());
        println!(
            "{:<10} {:?}: waited {waited:.2} s, ran {ran:.2} s",
            info.kind, info.state
        );
    }
    Ok(())
}
