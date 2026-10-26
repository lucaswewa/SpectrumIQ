//! The `calibration` Thing (ADR-0049): the stage–camera calibration of the objective in use,
//! measured by `siq-workflow`'s routine, fitted and checked by `siq-calib`, kept in the registry,
//! and stamped on every frame taken under it; and each objective's calibration in force.

use std::sync::Arc;

use anyhow::Context;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use siq_acquire::AcquisitionContext;
use siq_calib::{CalibrationRecord, Thresholds, backlash, fit};
use siq_config::model::ObjectiveConfig;
use siq_register::Registrar;
use siq_types::ObjectiveId;
use siq_workflow::routines::stage_camera::{StageCameraPlan, measure};
use teta_wot::prelude::{ActionError, Prop, Thing, ThingCtx, thing_impl};

use crate::Instrument;
use crate::things::{Bound, bind, bound};

/// The instrument's calibrations.
#[derive(Thing)]
pub struct Calibration {
    /// Each configured objective, with the stage–camera calibration in force for it, if any.
    #[property(readonly, default = Vec::new())]
    status: Prop<Vec<ObjectiveCalibration>>,
    instrument: Bound,
}

/// An objective, and its calibration in force.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ObjectiveCalibration {
    /// The objective.
    pub objective: ObjectiveId,
    /// Its name.
    pub name: String,
    /// Its stage–camera calibration in force: `null` until it is calibrated.
    pub calibration: Option<CalibrationRecord>,
}

/// The objective in the light path: the frames' context's, the turret's, or the only one
/// configured.
pub(crate) fn in_use(instrument: &Instrument) -> Option<ObjectiveConfig> {
    let objectives = &instrument.config.objectives;
    let by_context = instrument.acquisition.context().objective;
    let at_turret = instrument
        .peripherals
        .turret
        .as_ref()
        .and_then(|turret| turret.status().borrow().position);
    let found = objectives.iter().find(|objective| {
        Some(objective.id) == by_context
            || (by_context.is_none()
                && at_turret.is_some()
                && objective.turret_position == at_turret)
    });
    found
        .or((objectives.len() == 1).then(|| &objectives[0]))
        .cloned()
}

/// Gives the frames the objective in use and its calibration in force.
pub(crate) fn stamp(instrument: &Instrument) {
    let acquisition = &instrument.acquisition;
    let objective = in_use(instrument).map(|objective| objective.id);
    let calibration = objective
        .and_then(|objective| instrument.calibrations.current(objective))
        .map(|record| record.id);
    acquisition.set_context(AcquisitionContext {
        objective,
        calibration,
        ..acquisition.context()
    });
}

#[thing_impl]
impl Calibration {
    #[on_start]
    async fn start(&self, ctx: ThingCtx) -> anyhow::Result<()> {
        bind(&self.instrument, &ctx)?;
        self.refresh()
    }

    fn refresh(&self) -> anyhow::Result<()> {
        let instrument = bound(&self.instrument);
        let status = instrument
            .config
            .objectives
            .iter()
            .map(|objective| ObjectiveCalibration {
                objective: objective.id,
                name: objective.name.clone(),
                calibration: instrument.calibrations.current(objective.id),
            })
            .collect();
        Ok(self.status.set(status)?)
    }

    /// Calibrates the objective in use against the stage: a star of 16 moves about where the stage
    /// is, out to a third of the field, a frame at each registered against the first; the affine
    /// fitted, checked, and the backlash measured. Kept as a new version, it is in force for the
    /// objective's frames from then on. Something must be in the field to register.
    #[action]
    async fn stage_camera(&self) -> Result<CalibrationRecord, ActionError> {
        let instrument = bound(&self.instrument);
        let objective = in_use(instrument).context("no objective is in the light path")?;
        let capabilities = instrument.camera.capabilities();
        let pixel = capabilities.pixel_size.0 / objective.magnification;
        let field = [capabilities.sensor_width, capabilities.sensor_height]
            .map(|pixels| f64::from(pixels) * pixel);
        let plan = StageCameraPlan::for_field(field);
        let registrar = Arc::new(Registrar::default());
        let stage = instrument.stage.as_ref();
        let measured = measure(&instrument.acquisition, stage, &registrar, &plan).await?;
        let fitted = fit(&measured.moves)?;
        fitted.check(&Thresholds::default())?;
        let backlash = measured
            .backlash_shift
            .map(|shift| backlash(&fitted, shift));
        let record = CalibrationRecord::new(objective.id, &fitted, backlash, "stage_camera");
        instrument.calibrations.save(&record)?;
        stamp(instrument);
        self.refresh()?;
        Ok(record)
    }
}
