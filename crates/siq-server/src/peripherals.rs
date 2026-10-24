//! The peripherals' Things (ADR-0033, ADR-0035), each served only if it is fitted:
//!
//! - **`illumination`:** the sources and their levels, and `set_level` by a source's name;
//! - **`filter_wheel`:** the filter in the light path, and `select`;
//! - **`objective`:** the objective in the light path, and `select`, which also sets the frames'
//!   objective;
//! - **`modality`:** the mode and its pattern, and `set_modality`.
//!
//! Each mirrors its device's status into observable properties, and announces each change.

use std::sync::{Arc, Mutex, PoisonError};

use anyhow::Context;
use siq_config::model::ObjectiveConfig;
use siq_devices::{ChangerStatus, Modality, Pattern, PatternSet};
use siq_types::ModalityMode;
use teta_wot::prelude::Thing;
use teta_wot::prelude::{ActionError, Event, Prop, ThingCtx, thing_impl};
use tokio::task::JoinHandle;

use crate::imaging::until_frames_begin_after;
use crate::things::{Bound, bind, bound, follow, stop_task, this};

fn keep(follower: &Mutex<Option<JoinHandle<()>>>, task: JoinHandle<()>) {
    *follower.lock().unwrap_or_else(PoisonError::into_inner) = Some(task);
}

/// A changer's status, in its properties: `changed` when a position arrives.
fn mirror(
    position: &Prop<Option<u8>>,
    moving: &Prop<bool>,
    changed: &Event<u8>,
    status: &ChangerStatus,
) {
    let arrived = status
        .position
        .filter(|_| position.get() != status.position);
    let _ = position.set(status.position);
    let _ = moving.set(status.moving);
    if let Some(at) = arrived {
        changed.emit(at);
    }
}

/// The light sources.
#[derive(Thing)]
pub struct Illumination {
    /// Each source's level, by index, from 0 (off) to 1.
    #[property(readonly, default = Vec::new())]
    levels: Prop<Vec<f64>>,
    /// A level changed: every source's level.
    #[event]
    changed: Event<Vec<f64>>,
    instrument: Bound,
    follower: Mutex<Option<JoinHandle<()>>>,
}

impl Illumination {
    fn device(&self) -> &Arc<dyn siq_devices::Illumination> {
        let fitted = bound(&self.instrument).peripherals.illumination.as_ref();
        fitted.expect("served only when fitted")
    }
}

#[thing_impl]
impl Illumination {
    #[on_start]
    async fn start(&self, ctx: ThingCtx) -> anyhow::Result<()> {
        bind(&self.instrument, &ctx)?;
        let thing = this::<Illumination>(&ctx)?;
        let task = follow(self.device().status(), move |status| {
            if thing.levels.get() != status.levels {
                let _ = thing.levels.set(status.levels.clone());
                thing.changed.emit(status.levels.clone());
            }
        });
        keep(&self.follower, task);
        Ok(())
    }

    #[on_stop]
    async fn stop_following(&self) {
        stop_task(&self.follower);
    }

    /// The sources' names, by index.
    #[property]
    async fn sources(&self) -> Vec<String> {
        self.device().sources().to_vec()
    }

    /// Sets the level of the source named `source`, from 0 (off) to 1, and waits for the light to
    /// settle.
    #[action]
    async fn set_level(&self, source: String, level: f64) -> Result<Vec<f64>, ActionError> {
        let device = self.device();
        let sources = device.sources();
        let index = sources.iter().position(|name| *name == source);
        let index = index.with_context(|| format!("there is no source `{source}`: {sources:?}"))?;
        device.set_level(index, level).await?;
        Ok(device.status().borrow().levels.clone())
    }
}

/// The filter wheel.
#[derive(Thing)]
pub struct FilterWheel {
    /// The filter in the light path, by position: `null` while the wheel turns.
    #[property(readonly, default = None)]
    position: Prop<Option<u8>>,
    /// Whether the wheel is turning.
    #[property(readonly, default = false)]
    moving: Prop<bool>,
    /// A filter arrived in the light path: its position.
    #[event]
    changed: Event<u8>,
    instrument: Bound,
    follower: Mutex<Option<JoinHandle<()>>>,
}

impl FilterWheel {
    fn device(&self) -> &Arc<dyn siq_devices::FilterWheel> {
        let fitted = bound(&self.instrument).peripherals.filter_wheel.as_ref();
        fitted.expect("served only when fitted")
    }
}

#[thing_impl]
impl FilterWheel {
    #[on_start]
    async fn start(&self, ctx: ThingCtx) -> anyhow::Result<()> {
        bind(&self.instrument, &ctx)?;
        let thing = this::<FilterWheel>(&ctx)?;
        let task = follow(self.device().status(), move |status| {
            mirror(&thing.position, &thing.moving, &thing.changed, status);
        });
        keep(&self.follower, task);
        Ok(())
    }

    #[on_stop]
    async fn stop_following(&self) {
        stop_task(&self.follower);
    }

    /// The filters' names, by position.
    #[property]
    async fn filters(&self) -> Vec<String> {
        self.device().positions().to_vec()
    }

    /// Turns the wheel to `position`, the shorter way round, and waits for it to arrive.
    #[action]
    async fn select(&self, position: u8) -> Result<u8, ActionError> {
        self.device().select(position).await?;
        Ok(position)
    }
}

/// The objective turret.
#[derive(Thing)]
pub struct Objective {
    /// The objective in the light path, by turret position: `null` while the turret turns.
    #[property(readonly, default = None)]
    position: Prop<Option<u8>>,
    /// Whether the turret is turning.
    #[property(readonly, default = false)]
    moving: Prop<bool>,
    /// An objective arrived in the light path: its turret position.
    #[event]
    changed: Event<u8>,
    instrument: Bound,
    follower: Mutex<Option<JoinHandle<()>>>,
}

impl Objective {
    fn device(&self) -> &Arc<dyn siq_devices::ObjectiveTurret> {
        let fitted = bound(&self.instrument).peripherals.turret.as_ref();
        fitted.expect("served only when fitted")
    }

    fn configured(&self, position: Option<u8>) -> Option<ObjectiveConfig> {
        let objectives = &bound(&self.instrument).config.objectives;
        let found = objectives.iter().find(|objective| {
            objective.turret_position.is_some() && objective.turret_position == position
        });
        found.cloned()
    }
}

#[thing_impl]
impl Objective {
    #[on_start]
    async fn start(&self, ctx: ThingCtx) -> anyhow::Result<()> {
        bind(&self.instrument, &ctx)?;
        let thing = this::<Objective>(&ctx)?;
        let task = follow(self.device().status(), move |status| {
            mirror(&thing.position, &thing.moving, &thing.changed, status);
        });
        keep(&self.follower, task);
        Ok(())
    }

    #[on_stop]
    async fn stop_following(&self) {
        stop_task(&self.follower);
    }

    /// The configured objectives.
    #[property]
    async fn objectives(&self) -> Vec<ObjectiveConfig> {
        bound(&self.instrument).config.objectives.clone()
    }

    /// The configured objective in the light path, if there is one.
    #[property]
    async fn current(&self) -> Option<ObjectiveConfig> {
        self.configured(self.position.get())
    }

    /// Turns the turret to `position`, and waits for it to arrive. Frames exposed meanwhile have
    /// no objective; then they have the one at `position`.
    #[action]
    async fn select(&self, position: u8) -> Result<Option<ObjectiveConfig>, ActionError> {
        let instrument = bound(&self.instrument);
        let acquisition = &instrument.acquisition;
        let context = acquisition.context();
        acquisition.set_context(siq_acquire::AcquisitionContext {
            objective: None,
            ..context
        });
        self.device().select(position).await?;
        until_frames_begin_after(instrument).await;
        let objective = self.configured(Some(position));
        acquisition.set_context(siq_acquire::AcquisitionContext {
            objective: objective.as_ref().map(|objective| objective.id),
            ..acquisition.context()
        });
        Ok(objective)
    }
}

/// The modality unit: widefield, light field, confocal and structured illumination.
#[derive(Thing)]
pub struct ModalityUnit {
    /// The mode in place, or coming into place, with its pattern for structured illumination.
    #[property(readonly, default = Modality::WIDEFIELD)]
    modality: Prop<Modality>,
    /// Whether it is changing.
    #[property(readonly, default = false)]
    moving: Prop<bool>,
    /// A mode came into place.
    #[event]
    changed: Event<Modality>,
    instrument: Bound,
    follower: Mutex<Option<JoinHandle<()>>>,
}

impl ModalityUnit {
    fn device(&self) -> &Arc<dyn siq_devices::ModalityUnit> {
        let fitted = bound(&self.instrument).peripherals.modality.as_ref();
        fitted.expect("served only when fitted")
    }
}

#[thing_impl]
impl ModalityUnit {
    #[on_start]
    async fn start(&self, ctx: ThingCtx) -> anyhow::Result<()> {
        bind(&self.instrument, &ctx)?;
        let thing = this::<ModalityUnit>(&ctx)?;
        let task = follow(self.device().status(), move |status| {
            let arrived = thing.moving.get() && !status.moving;
            let _ = thing.modality.set(status.modality);
            let _ = thing.moving.set(status.moving);
            if arrived {
                thing.changed.emit(status.modality);
            }
        });
        keep(&self.follower, task);
        Ok(())
    }

    #[on_stop]
    async fn stop_following(&self) {
        stop_task(&self.follower);
    }

    /// The modes it offers.
    #[property]
    async fn modes(&self) -> Vec<ModalityMode> {
        self.device().modes().to_vec()
    }

    /// The structured-illumination patterns it can show, if it offers that mode.
    #[property]
    async fn patterns(&self) -> Option<PatternSet> {
        self.device().patterns()
    }

    /// Puts `mode` in place, with `pattern` for structured illumination, and waits for it.
    #[action]
    async fn set_modality(
        &self,
        mode: ModalityMode,
        #[param(default)] pattern: Option<Pattern>,
    ) -> Result<Modality, ActionError> {
        let modality = Modality { mode, pattern };
        self.device().set_modality(modality).await?;
        Ok(modality)
    }
}
