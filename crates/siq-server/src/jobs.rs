//! The `jobs` Thing (ADR-0043): the jobs waiting and running, observed, and `cancel`.

use std::sync::{Mutex, PoisonError};

use siq_jobs::{JobId, JobInfo};
use teta_wot::prelude::Thing;
use teta_wot::prelude::{Prop, ThingCtx, thing_impl};
use tokio::task::JoinHandle;

use crate::things::{Bound, bind, bound, follow, stop_task, this};

/// The background jobs: pyramids, stitching, exports.
#[derive(Thing)]
pub struct Jobs {
    /// The jobs waiting, in the order they will be considered.
    #[property(readonly, default = Vec::new())]
    queue: Prop<Vec<JobInfo>>,
    /// The jobs running, oldest first.
    #[property(readonly, default = Vec::new())]
    running: Prop<Vec<JobInfo>>,
    instrument: Bound,
    follower: Mutex<Option<JoinHandle<()>>>,
}

#[thing_impl]
impl Jobs {
    #[on_start]
    async fn start(&self, ctx: ThingCtx) -> anyhow::Result<()> {
        bind(&self.instrument, &ctx)?;
        let thing = this::<Jobs>(&ctx)?;
        let task = follow(bound(&self.instrument).jobs.watch(), move |jobs| {
            let _ = thing.queue.set(jobs.queued.clone());
            let _ = thing.running.set(jobs.running.clone());
        });
        *self.follower.lock().unwrap_or_else(PoisonError::into_inner) = Some(task);
        Ok(())
    }

    #[on_stop]
    async fn stop_following(&self) {
        stop_task(&self.follower);
    }

    /// Cancels job `id`: a waiting one at once, a running one as soon as it stops. False if it
    /// isn't waiting or running. It doesn't wait for the global lock.
    #[action(global_lock = false)]
    async fn cancel(&self, id: u64) -> bool {
        bound(&self.instrument).jobs.cancel(JobId(id))
    }
}
