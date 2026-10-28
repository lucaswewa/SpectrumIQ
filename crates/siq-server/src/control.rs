//! The `control` Thing (ADR-0066): who holds the Hardware Control Token and since when, observed,
//! and `changed` as it changes hands. The token is requested, released and taken over through
//! `/api/siq/control/…`, which know who is asking; a Thing's action doesn't.

use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use chrono::{DateTime, Utc};
use siq_auth::{ControlState, Reason};
use teta_wot::prelude::Thing;
use teta_wot::prelude::{Event, Prop, ThingCtx, thing_impl};
use tokio::task::JoinHandle;

use crate::things::{Bound, bind, bound, follow, this};

/// How often a silent holder's timeout is looked at.
const TICK: Duration = Duration::from_secs(1);

/// The Hardware Control Token.
#[derive(Thing)]
pub struct Control {
    /// The user who holds the token: `null` while it is free.
    #[property(readonly, default = None)]
    holder: Prop<Option<String>>,
    /// Since when the holder has held it, in UTC.
    #[property(readonly, default = None)]
    since: Prop<Option<DateTime<Utc>>>,
    /// The token changed hands: its holder now, if any, and why.
    #[event]
    changed: Event<ControlState>,
    instrument: Bound,
    tasks: Mutex<Vec<JoinHandle<()>>>,
}

#[thing_impl]
impl Control {
    #[on_start]
    async fn start(&self, ctx: ThingCtx) -> anyhow::Result<()> {
        bind(&self.instrument, &ctx)?;
        let control = std::sync::Arc::clone(&bound(&self.instrument).auth.control);
        let thing = this::<Control>(&ctx)?;
        let follower = follow(control.watch(), move |state: &ControlState| {
            let holder = state.holder.as_ref();
            let _ = thing.holder.set(holder.map(|h| h.user.clone()));
            let _ = thing.since.set(holder.map(|h| h.since));
            if state.reason != Reason::Start {
                thing.changed.emit(state.clone());
            }
        });
        // A silent holder's token goes without anyone asking, and the watchers hear of it.
        let ticker = tokio::spawn(async move {
            let mut tick = tokio::time::interval(TICK);
            loop {
                tick.tick().await;
                control.expire();
            }
        });
        let mut tasks = self.tasks.lock().unwrap_or_else(PoisonError::into_inner);
        tasks.extend([follower, ticker]);
        Ok(())
    }

    #[on_stop]
    async fn stop_tasks(&self) {
        for task in self
            .tasks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .drain(..)
        {
            task.abort();
        }
    }
}
