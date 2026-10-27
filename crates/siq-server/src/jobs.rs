//! The `jobs` Thing (ADR-0043): the jobs waiting and running, observed, and `cancel`; a dataset's
//! `stitch`, and whether a finished tile scan is stitched by itself (ADR-0064).

use std::sync::atomic::Ordering;
use std::sync::{Mutex, PoisonError};

use anyhow::Context;
use siq_jobs::{JobId, JobInfo, Priority};
use siq_stitch::StitchOptions;
use siq_types::DatasetId;
use teta_wot::prelude::Thing;
use teta_wot::prelude::{ActionError, Prop, PropertyError, ThingCtx, thing_impl};
use tokio::task::JoinHandle;

use crate::stitch::{self, IMAGE};
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

    /// Stitches dataset `dataset` into its image `image` (`stitched` by default), as a background
    /// job: its pairs registered, its tiles solved, every frame composited, the pyramid built, and
    /// its QA report written (`qa/<image>.json`). Its tiles are placed by the calibration the
    /// dataset keeps; each channel through the dark and flat in force. It answers with the job's
    /// id, and doesn't wait for the global lock.
    #[action(global_lock = false)]
    async fn stitch(
        &self,
        dataset: DatasetId,
        #[param(default)] image: Option<String>,
        #[param(default)] options: Option<StitchOptions>,
    ) -> Result<u64, ActionError> {
        let instrument = bound(&self.instrument);
        let summary = instrument.catalog.get(dataset);
        let folder = summary.context(format!("no dataset {dataset}"))?.path;
        let image = image.unwrap_or_else(|| IMAGE.to_owned());
        let options = options.unwrap_or_default();
        let id = stitch::submit(instrument, &folder, &image, options, Priority::Interactive)?;
        Ok(id.0)
    }

    /// Whether a tile scan that finishes is stitched by itself, into `stitched`: on when the
    /// service starts.
    #[property(global_lock = false)]
    async fn auto_stitch(&self) -> bool {
        bound(&self.instrument).auto_stitch.load(Ordering::Relaxed)
    }

    #[setter(auto_stitch)]
    async fn set_auto_stitch(&self, value: bool) -> Result<(), PropertyError> {
        bound(&self.instrument)
            .auto_stitch
            .store(value, Ordering::Relaxed);
        Ok(())
    }
}
