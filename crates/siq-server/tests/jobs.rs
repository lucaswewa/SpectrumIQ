//! The `jobs` Thing (ADR-0043), with the simulated instrument of `config/sim.toml`: the jobs running
//! and waiting, observed, and `cancel` over HTTP. REQ-3D-005.

mod common;

use std::time::Duration;

use common::{config, invoke, serve};
use serde_json::{Value, json};
use siq_jobs::{Job, JobCtx, JobError, JobOutput, JobState, Priority, Resources};

/// A job that runs until cancelled.
struct Waits(u64);

impl Job for Waits {
    fn kind(&self) -> &str {
        "waits"
    }

    fn estimate(&self) -> Resources {
        Resources {
            ram: self.0,
            ..Resources::default()
        }
    }

    fn run(&mut self, ctx: &JobCtx) -> Result<JobOutput, JobError> {
        loop {
            ctx.check()?;
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

#[tokio::test]
async fn the_jobs_thing_shows_the_jobs_and_cancels_them() {
    let (client, instrument) = serve(config()).await;
    let mut running = client.events("/api/v1/jobs/running").await;
    let small = instrument
        .jobs
        .submit(Box::new(Waits(1_000_000_000)), Priority::Normal, None);
    // Bigger than the 14 GB budget: it waits until nothing else runs.
    let huge = instrument
        .jobs
        .submit(Box::new(Waits(20_000_000_000)), Priority::Background, None);
    loop {
        let jobs = running.next().await.expect("the running jobs are observed");
        if jobs.as_array().is_some_and(|jobs| !jobs.is_empty()) {
            assert_eq!(
                (&jobs[0]["kind"], &jobs[0]["state"]),
                (&json!("waits"), &json!("running"))
            );
            break;
        }
    }
    let queue = client.get("/api/v1/jobs/queue").await.json();
    assert_eq!(queue[0]["id"], huge.0);
    assert_eq!(queue[0]["priority"], "background");
    assert_eq!(queue[0]["estimate"]["ram"], 20_000_000_000u64);

    let cancelled = invoke(&client, "/api/v1/jobs/cancel", json!({ "id": small.0 })).await;
    assert_eq!(cancelled, true);
    let jobs = instrument.jobs.clone();
    let ended = tokio::task::spawn_blocking(move || jobs.wait(small, Duration::from_secs(10)));
    assert_eq!(ended.await.unwrap().unwrap().state, JobState::Cancelled);
    // With the small one gone, the big one runs, alone, until cancelled too.
    loop {
        let jobs: Value = running.next().await.expect("the running jobs are observed");
        if jobs[0]["id"] == huge.0 {
            assert_eq!(jobs.as_array().map(Vec::len), Some(1), "{jobs}");
            break;
        }
    }
    assert_eq!(
        invoke(&client, "/api/v1/jobs/cancel", json!({ "id": huge.0 })).await,
        true
    );
    assert_eq!(
        invoke(&client, "/api/v1/jobs/cancel", json!({ "id": 99 })).await,
        false
    );
    client.stop().await;
}
