//! Stitching from the service (ADR-0064): a dataset's stitch job made from what the instrument
//! knows (the calibration the scan kept, the channels' names and colours, and the dark and flat in
//! force for its objective and each channel), run on `jobs.stitch` or by itself once a tile scan
//! finishes; and its QA report drawn as the viewer's overlays.

use std::path::Path;
use std::sync::atomic::Ordering;

use anyhow::Context;
use serde_json::{Value, json};
use siq_calib::CalibrationRecord;
use siq_jobs::kinds::StitchJob;
use siq_jobs::{JobId, Priority};
use siq_stitch::StitchOptions;
use siq_stitch::report::{Report, TileQa};
use siq_storage::{Dataset, DatasetKind, DatasetReader};
use siq_workflow::engine::{Outcome, RunReport};
use tracing::{info, warn};

use crate::calibration::shading_in_force;
use crate::{DatasetHook, Instrument};

/// The image a stitch makes, unless asked otherwise.
pub const IMAGE: &str = "stitched";

/// The stitch job of the dataset at `folder`, into the image `image`.
pub(crate) fn job(
    instrument: &Instrument,
    folder: &Path,
    image: &str,
    options: StitchOptions,
) -> anyhow::Result<StitchJob> {
    let dataset = Dataset::open(folder)?;
    let snapshot = dataset.manifest().calibration.as_ref();
    let snapshot =
        snapshot.context("the dataset keeps no stage–camera calibration to place its tiles by")?;
    let record: CalibrationRecord = serde_json::from_value(snapshot.value.clone())?;
    let reader = DatasetReader::open(&dataset)?;
    let first = reader
        .frames()
        .first()
        .context("the dataset has no frames")?;
    let objective = first.sidecar.meta.objective;
    let mut ids: Vec<u16> = reader
        .frames()
        .iter()
        .map(|frame| frame.sidecar.key.c)
        .collect();
    ids.sort_unstable();
    ids.dedup();
    let configured = &instrument.config.channels;
    // A frame's channel is its key's, `0` for none.
    let shading = ids
        .iter()
        .filter_map(|&c| {
            let channel = configured.iter().find(|channel| channel.id.0 == c);
            let (dark, flat) = shading_in_force(instrument, (objective, channel.map(|ch| ch.id)))?;
            Some((c, dark, flat))
        })
        .collect();
    let channels = configured
        .iter()
        .map(|channel| (channel.id.0, channel.name.clone(), channel.colour.0))
        .collect();
    Ok(StitchJob {
        dataset: folder.to_owned(),
        image: image.to_owned(),
        l: record.affine(),
        channels,
        shading,
        options,
    })
}

/// Submits the stitch of the dataset at `folder` into `image`, at `priority`: its job.
pub(crate) fn submit(
    instrument: &Instrument,
    folder: &Path,
    image: &str,
    options: StitchOptions,
    priority: Priority,
) -> anyhow::Result<JobId> {
    let job = job(instrument, folder, image, options)?;
    Ok(instrument
        .jobs
        .submit(Box::new(job), priority, Some(folder.to_owned())))
}

/// Stitches each tile scan that finishes, into `stitched`, while `jobs.auto_stitch` says so.
pub(crate) struct AutoStitch;

impl DatasetHook for AutoStitch {
    fn on_dataset_complete(&self, instrument: &Instrument, report: &RunReport) {
        if !instrument.auto_stitch.load(Ordering::Relaxed) || report.outcome != Outcome::Finished {
            return;
        }
        let kind = Dataset::open(&report.path).map(|dataset| dataset.manifest().kind);
        if kind.ok() != Some(DatasetKind::TileScan) {
            return;
        }
        let options = StitchOptions::default();
        match submit(
            instrument,
            &report.path,
            IMAGE,
            options,
            Priority::Background,
        ) {
            Ok(id) => info!(dataset = %report.dataset, job = %id.0, "the tile scan is stitched"),
            Err(error) => warn!(dataset = %report.dataset, %error, "the tile scan wasn't stitched"),
        }
    }
}

/// The overlays of a stitch's report: each tile's outline, labelled with its position, and each
/// pair's line between the tiles' centres, its tone its residual's (ADR-0064).
pub(crate) fn overlays(report: &Report) -> Value {
    let size = report.bounds.size;
    let outlines: Vec<Value> = report
        .tiles
        .iter()
        .map(|tile| {
            json!({
                "kind": "rect",
                "x": tile.corner[0],
                "y": tile.corner[1],
                "width": tile.size[0],
                "height": tile.size[1],
                "dashed": true,
                "label": tile.p.to_string(),
                "title": format!(
                    "tile {}: moved {:.1} px from the stage's place, {:.2} px residual at worst",
                    tile.p, tile.moved_px, tile.worst_px
                ),
            })
        })
        .collect();
    let centre = |tile: &TileQa| {
        let [w, h] = tile.size.map(f64::from);
        [tile.corner[0] + w / 2.0, tile.corner[1] + h / 2.0]
    };
    let lines: Vec<Value> = report
        .edges
        .iter()
        .map(|edge| {
            let (a, b) = (&report.tiles[edge.i], &report.tiles[edge.j]);
            let (tone, what) = match (edge.confident, edge.used) {
                (false, _) => ("muted", "not trusted".to_owned()),
                (true, false) => (
                    "danger",
                    format!("rejected, {:.2} px off", edge.residual_px),
                ),
                _ if edge.residual_px <= 0.5 => ("success", format!("{:.2} px", edge.residual_px)),
                _ => ("warning", format!("{:.2} px", edge.residual_px)),
            };
            json!({
                "kind": "line",
                "points": [centre(a), centre(b)],
                "dashed": !edge.used,
                "tone": tone,
                "title": format!("tiles {} and {}: {what}, correlation {:.2}", a.p, b.p, edge.ncc),
            })
        })
        .collect();
    json!([
        { "id": "stitch-tiles", "label": "Tile outlines", "size": size, "shapes": outlines },
        { "id": "stitch-residuals", "label": "Stitch residuals", "size": size, "shapes": lines },
    ])
}
