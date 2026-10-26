//! The `calibration` Thing: the stage–camera calibration of the objective in use (ADR-0049),
//! measured by `siq-workflow`'s routine, fitted and checked by `siq-calib`, kept in the registry,
//! and stamped on every frame taken under it; the dark and the flat of the objective and channel in
//! use, captured or estimated from a dataset (ADR-0050), and the previews' correction; and what is
//! in force for each.

use std::sync::Arc;

use anyhow::Context;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use siq_acquire::AcquisitionContext;
use siq_calib::shading::{FlatField, ShadingKind, ShadingRecord};
use siq_calib::{CalibrationRecord, Thresholds, backlash, fit};
use siq_config::model::ObjectiveConfig;
use siq_jobs::Priority;
use siq_jobs::kinds::ShadingJob;
use siq_preview::PreviewFlat;
use siq_register::Registrar;
use siq_storage::{Dataset, DatasetReader};
use siq_types::{ChannelId, DatasetId, ObjectiveId, Roi};
use siq_workflow::routines::shading::{FlatPlan, capture_dark, capture_flat};
use siq_workflow::routines::stage_camera::{StageCameraPlan, measure};
use teta_wot::prelude::{ActionError, Thing, ThingCtx, thing_impl};

use crate::Instrument;
use crate::things::{Bound, bind, bound};

/// The instrument's calibrations.
#[derive(Thing)]
pub struct Calibration {
    instrument: Bound,
}

/// An objective, and the calibrations in force for it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ObjectiveCalibration {
    /// The objective.
    pub objective: ObjectiveId,
    /// Its name.
    pub name: String,
    /// Its stage–camera calibration in force: `null` until it is calibrated.
    pub calibration: Option<CalibrationRecord>,
    /// Its dark and flat with each channel, and with none.
    pub shading: Vec<ChannelShading>,
}

/// A channel's dark and flat under one objective.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ChannelShading {
    /// The channel: `null` for frames taken with none.
    pub channel: Option<ChannelId>,
    /// Its name.
    pub name: String,
    /// The dark in force.
    pub dark: Option<ShadingRecord>,
    /// The flat in force.
    pub flat: Option<ShadingRecord>,
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

/// The dark level and the flat in force for `objective` and `channel`: the dark's level, or the
/// one the flat was measured less.
fn shading_in_force(
    instrument: &Instrument,
    (objective, channel): (Option<ObjectiveId>, Option<ChannelId>),
) -> Option<(f32, FlatField)> {
    let shading = &instrument.shading;
    let flat = shading.current((objective, channel, ShadingKind::Flat))?;
    let dark = shading.current((objective, channel, ShadingKind::Dark));
    let level = dark.map_or(flat.level, |dark| dark.level);
    Some((level as f32, flat.flat?))
}

/// Gives the frames the objective in use and its calibration in force, and the previews the
/// dark and flat in force for it and the channel.
pub(crate) fn stamp(instrument: &Instrument) {
    let acquisition = &instrument.acquisition;
    let objective = in_use(instrument).map(|objective| objective.id);
    let calibration = objective
        .and_then(|objective| instrument.calibrations.current(objective))
        .map(|record| record.id);
    let context = AcquisitionContext {
        objective,
        calibration,
        ..acquisition.context()
    };
    acquisition.set_context(context);
    let shading = shading_in_force(instrument, (objective, context.channel)).map(Arc::new);
    let previews = &instrument.previews;
    previews.set_flat(PreviewFlat {
        shading,
        ..previews.flat()
    });
}

/// The objective in use, and the field it sees, in µm.
fn field(instrument: &Instrument) -> anyhow::Result<(ObjectiveConfig, [f64; 2])> {
    let objective = in_use(instrument).context("no objective is in the light path")?;
    let capabilities = instrument.camera.capabilities();
    let pixel = capabilities.pixel_size.0 / objective.magnification;
    let field = [capabilities.sensor_width, capabilities.sensor_height]
        .map(|pixels| f64::from(pixels) * pixel);
    Ok((objective, field))
}

#[thing_impl]
impl Calibration {
    #[on_start]
    async fn start(&self, ctx: ThingCtx) -> anyhow::Result<()> {
        bind(&self.instrument, &ctx)
    }

    /// Each configured objective, with its stage–camera calibration in force, and its dark and
    /// flat with each channel.
    #[property]
    async fn status(&self) -> Vec<ObjectiveCalibration> {
        let instrument = bound(&self.instrument);
        let config = &instrument.config;
        let channels: Vec<_> = config
            .channels
            .iter()
            .map(|channel| (Some(channel.id), channel.name.clone()))
            .chain([(None, "no channel".to_owned())])
            .collect();
        let shading = &instrument.shading;
        let objectives = config.objectives.iter().map(|objective| {
            let key = Some(objective.id);
            let shading = channels.iter().map(|(channel, name)| ChannelShading {
                channel: *channel,
                name: name.clone(),
                dark: shading.current((key, *channel, ShadingKind::Dark)),
                flat: shading.current((key, *channel, ShadingKind::Flat)),
            });
            ObjectiveCalibration {
                objective: objective.id,
                name: objective.name.clone(),
                calibration: instrument.calibrations.current(objective.id),
                shading: shading.collect(),
            }
        });
        objectives.collect()
    }

    /// Calibrates the objective in use against the stage: a star of 16 moves about where the stage
    /// is, out to 0.4 of the field, a frame at each registered against the first; the affine
    /// fitted, checked, and the backlash measured. Kept as a new version, it is in force for the
    /// objective's frames from then on. Something must be in the field to register.
    #[action]
    async fn stage_camera(&self) -> Result<CalibrationRecord, ActionError> {
        let instrument = bound(&self.instrument);
        let (objective, field) = field(instrument)?;
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
        instrument.calibrations.save(&record, &[])?;
        stamp(instrument);
        Ok(record)
    }

    /// Takes the dark of the objective and channel in use: `frames` frames (16 by default) with
    /// every light off, averaged, at the camera's settings, of the whole sensor. Kept as a new
    /// version, it is in force for them from then on.
    #[action]
    async fn capture_dark(
        &self,
        #[param(default)] frames: Option<u32>,
    ) -> Result<ShadingRecord, ActionError> {
        let instrument = bound(&self.instrument);
        let key = (
            in_use(instrument).map(|objective| objective.id),
            instrument.acquisition.context().channel,
        );
        let lights = instrument.peripherals.illumination.as_deref();
        let frames = frames.unwrap_or(16).max(1) as usize;
        let captured = capture_dark(&instrument.acquisition, lights, frames).await?;
        let capabilities = instrument.camera.capabilities();
        let whole = Roi::full(capabilities.sensor_width, capabilities.sensor_height);
        if captured.meta.roi != whole {
            let why = "a dark is of the whole sensor: set the full region first";
            return Err(anyhow::anyhow!(why).into());
        }
        let dark = captured.sum.dark(captured.meta.binning);
        let record = instrument
            .shading
            .save_dark(key, &dark, &captured.meta, frames)?;
        stamp(instrument);
        Ok(record)
    }

    /// Takes the flat of the objective and channel in use: `frames` frames (18 by default) at
    /// nine places a field apart about the stage, `defocus_um` out of focus (0 by default, for a
    /// blank slide), averaged, less the dark in force, and fitted. Kept as a new version, it is in
    /// force for them from then on.
    #[action]
    async fn capture_flat(
        &self,
        #[param(default)] defocus_um: Option<f64>,
        #[param(default)] frames: Option<u32>,
    ) -> Result<ShadingRecord, ActionError> {
        let instrument = bound(&self.instrument);
        let (objective, field) = field(instrument)?;
        let key = (Some(objective.id), instrument.acquisition.context().channel);
        let mut plan = FlatPlan::for_field(field, defocus_um.unwrap_or(0.0));
        if let Some(frames) = frames {
            plan.frames = (frames as usize).div_ceil(plan.places.len()).max(1);
        }
        let (stage, focus) = (instrument.stage.as_ref(), instrument.focus.as_ref());
        let captured = capture_flat(&instrument.acquisition, stage, focus, &plan).await?;
        let dark = instrument
            .shading
            .current((key.0, key.1, ShadingKind::Dark));
        let level = dark.map_or(0.0, |dark| dark.level);
        let capabilities = instrument.camera.capabilities();
        let sensor = [capabilities.sensor_width, capabilities.sensor_height];
        let mean = captured.sum.mean();
        let flat = FlatField::fit(&mean.view(), captured.meta.roi, sensor, level as f32)?;
        let frames = captured.sum.frames() as usize;
        let record = ShadingRecord::new(key, &captured.meta, frames, "capture");
        let record = record.with_flat(flat, level);
        instrument.shading.save(&record, &[])?;
        stamp(instrument);
        Ok(record)
    }

    /// Estimates the flat of dataset `dataset`'s frames of the whole sensor, its tiles of
    /// different places, as a background job, whose id it gives (BaSiC, simplified). The flat is
    /// kept, and in force, for the first frame's objective and channel when the job ends.
    /// `dark_level` is what the frames are measured less, in ADU: by default the level of the
    /// dark in force for them, or 0. It doesn't wait for the global lock.
    #[action(global_lock = false)]
    async fn estimate_shading(
        &self,
        dataset: DatasetId,
        #[param(default)] dark_level: Option<f64>,
    ) -> Result<u64, ActionError> {
        let instrument = bound(&self.instrument);
        let summary = instrument
            .catalog
            .get(dataset)
            .with_context(|| format!("no dataset {dataset}"))?;
        let reader = DatasetReader::open(&Dataset::open(&summary.path)?)?;
        let first = reader.frames().first();
        let meta = first.context("the dataset has no raw frames")?.sidecar.meta;
        let key = (meta.objective, meta.channel);
        let dark = instrument
            .shading
            .current((key.0, key.1, ShadingKind::Dark));
        let job = ShadingJob {
            registry: Arc::clone(&instrument.shading),
            key,
            dark: dark_level.or(dark.map(|dark| dark.level)).unwrap_or(0.0),
            sensor: [
                meta.roi.offset_x + meta.roi.width,
                meta.roi.offset_y + meta.roi.height,
            ],
        };
        let id = instrument
            .jobs
            .submit(Box::new(job), Priority::Background, Some(summary.path));
        Ok(id.0)
    }
}
