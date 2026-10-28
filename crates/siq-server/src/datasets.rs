//! The `datasets` Thing (ADR-0041, as amended in P4.7): the datasets of the data root, a snap made
//! into one, their exports, and deleting one.
//!
//! **A snap becomes a dataset** in three steps:
//! - its frames, one per channel, are recorded raw by the frame recorder;
//! - a job writes them as level 0 of an OME-Zarr image, `zarr/snap.ome.zarr`, a channel each;
//! - the image's pyramid is built as a job.
//!
//! `snap_to_dataset` returns once the pyramid job is submitted. The image's tiles are served from
//! then on, from the levels built so far.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, anyhow, bail};
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use siq_buffer::Policy;
use siq_config::model::ChannelConfig;
use siq_imgproc::{Histogram, histogram};
use siq_jobs::kinds::{DziExportJob, OmeTiffExportJob, PyramidJob, ZarrFromRawJob};
use siq_jobs::{JobState, Priority};
use siq_pyramid::tiles::TileFormat;
use siq_storage::ometiff::TiffCompression;
use siq_storage::zarr::{CHUNK, ChannelInfo, ImageInfo, SampleType, Size5};
use siq_storage::{
    Dataset, DatasetKind, DatasetSummary, DatasetWriter, FrameKey, NewDataset, Snapshot,
    WriterOptions,
};
use siq_types::{ChannelId, DatasetId, FrameMeta};
use teta_wot::prelude::{ActionError, Event, Thing, ThingCtx, thing_impl};

use crate::Instrument;
use crate::imaging::apply;
use crate::things::{Bound, bind, bound};

/// The image a snap is made into.
const SNAP_IMAGE: &str = "snap";
/// The most datasets a page holds.
const MAX_PAGE: usize = 500;
/// The longest a snap waits for its image to be written.
const WRITING: Duration = Duration::from_secs(3600);

/// The datasets: listed, made from a snap, exported and deleted.
#[derive(Thing)]
pub struct Datasets {
    /// A dataset was made.
    #[event]
    created: Event<DatasetEntry>,
    instrument: Bound,
}

/// A dataset, as the API lists it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct DatasetEntry {
    /// Its id.
    pub id: DatasetId,
    /// What it holds: `snap`, `tile_scan`, `z_stack`, ….
    pub kind: String,
    /// Its name.
    pub name: String,
    /// When it was made.
    pub created: DateTime<Utc>,
    /// Who made it.
    pub user: String,
    /// The raw frames recorded in it.
    pub frames: usize,
    /// Its OME-Zarr images, by name, as the data routes take them: `snap`, `mosaic`, ….
    pub images: Vec<String>,
}

impl DatasetEntry {
    pub(crate) fn of(summary: &DatasetSummary) -> Self {
        Self {
            id: summary.id,
            kind: summary.kind.name().to_owned(),
            name: summary.name.clone(),
            created: summary.created,
            user: summary.user.clone(),
            frames: summary.frames,
            images: images(&summary.path),
        }
    }
}

/// The names of the OME-Zarr images in a dataset's `zarr/` folder, sorted.
pub(crate) fn images(dataset: &Path) -> Vec<String> {
    let entries = std::fs::read_dir(dataset.join("zarr"))
        .into_iter()
        .flatten();
    let mut names: Vec<String> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            name.strip_suffix(".ome.zarr").map(str::to_owned)
        })
        .collect();
    names.sort();
    names
}

/// A page of the datasets, newest first.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct DatasetPage {
    /// The datasets in all.
    pub total: usize,
    /// The first one's place in the list.
    pub offset: usize,
    /// The page's datasets.
    pub datasets: Vec<DatasetEntry>,
}

/// A snap made into a dataset.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SnapDataset {
    /// The dataset.
    pub dataset: DatasetEntry,
    /// Its image's name.
    pub image: String,
    /// The job building the image's pyramid.
    pub pyramid: u64,
}

#[thing_impl]
impl Datasets {
    #[on_start]
    async fn start(&self, ctx: ThingCtx) -> anyhow::Result<()> {
        bind(&self.instrument, &ctx)
    }

    /// The datasets, newest first: up to `limit` (at most 500) from `offset`. It doesn't wait for
    /// the global lock.
    #[action(global_lock = false)]
    async fn summary(
        &self,
        #[param(default)] offset: usize,
        #[param(default = 50)] limit: usize,
    ) -> DatasetPage {
        let all = bound(&self.instrument).catalog.list();
        let datasets = all.iter().skip(offset).take(limit.min(MAX_PAGE));
        DatasetPage {
            total: all.len(),
            offset,
            datasets: datasets.map(DatasetEntry::of).collect(),
        }
    }

    /// Snaps a frame in each of `channels`, in order, or one in the channel in use, and makes them
    /// a dataset named `name` (`snap` by default): the frames recorded raw, then written as level
    /// 0 of the image `snap`, a channel each, then its pyramid submitted as a job. Frames must be
    /// of 16-bit pixels.
    #[action]
    async fn snap_to_dataset(
        &self,
        #[param(default)] channels: Option<Vec<ChannelId>>,
        #[param(default)] name: Option<String>,
    ) -> Result<SnapDataset, ActionError> {
        let instrument = Arc::clone(self.instrument.get().context("not started")?);
        let config = &instrument.config;
        let channels: Vec<Option<ChannelConfig>> = match channels {
            Some(ids) => ids
                .iter()
                .map(|id| {
                    let channel = config.channels.iter().find(|channel| channel.id == *id);
                    channel
                        .cloned()
                        .map(Some)
                        .context(format!("no channel {id}"))
                })
                .collect::<anyhow::Result<_>>()?,
            None => vec![None],
        };
        if channels.is_empty() {
            return Err(anyhow!("a snap needs a channel").into());
        }
        let mut new = NewDataset::new(DatasetKind::Snap, name.as_deref().unwrap_or("snap"));
        new.instrument = Some(Snapshot::of(config)?);
        let calibration = instrument.acquisition.context().calibration;
        let record = calibration.and_then(|id| instrument.calibrations.get(id));
        new.calibration = record.map(|record| Snapshot::of(&record)).transpose()?;
        let dataset = Dataset::create(instrument.catalog.data_root(), new)?;

        // The frames, recorded as they come: frame `c` is channel `c`.
        let frames =
            instrument
                .acquisition
                .subscribe("snap-to-dataset", Policy::Lossless, |meta| meta.flags.save);
        let mut options = WriterOptions::new((config.server.min_free_gb * 1e9) as u64);
        let mut next = 0;
        options.key = Box::new(move |_| {
            next += 1;
            key(next - 1)
        });
        let writer = DatasetWriter::start(&dataset, frames, options)?;
        let mut taken = Vec::new();
        for channel in &channels {
            if let Some(channel) = channel {
                apply(&instrument, channel).await?;
            }
            let frame = instrument.acquisition.snap(true).await?;
            let image = frame.image::<u16>().context("only 16-bit frames for now")?;
            let mut counts = Histogram::new(frame.meta().pixel_format.bits());
            histogram(&image, 4, &mut counts)?;
            taken.push((*frame.meta(), counts.auto_window(0.001, 0.999)));
        }
        let report = tokio::task::spawn_blocking(move || writer.finish()).await?;
        if let Some(error) = report.error {
            return Err(error.into());
        }

        let path = dataset.zarr_dir().join(format!("{SNAP_IMAGE}.ome.zarr"));
        let info = image_info(&instrument, &channels, &taken);
        let job = ZarrFromRawJob {
            image: path.clone(),
            info,
            frames: (0..taken.len()).map(key).collect(),
        };
        let folder = Some(dataset.path().to_owned());
        let written = instrument
            .jobs
            .submit(Box::new(job), Priority::Interactive, folder.clone());
        let jobs = Arc::clone(&instrument.jobs);
        let info = tokio::task::spawn_blocking(move || jobs.wait(written, WRITING)).await?;
        let info = info.context("the job writing the image was lost")?;
        if info.state != JobState::Done {
            let why = info.error.unwrap_or_default();
            return Err(anyhow!("the image wasn't written: {why}").into());
        }
        let pyramid = PyramidJob::new(&path);
        let pyramid = instrument
            .jobs
            .submit(Box::new(pyramid), Priority::Interactive, folder);

        instrument.catalog.record(dataset.path())?;
        let summary = instrument
            .catalog
            .get(dataset.manifest().id)
            .context("the dataset made")?;
        let entry = DatasetEntry::of(&summary);
        self.created.emit(entry.clone());
        Ok(SnapDataset {
            dataset: entry,
            image: SNAP_IMAGE.to_owned(),
            pyramid: pyramid.0,
        })
    }

    /// Exports image `image` of dataset `dataset` as OME-TIFF, to `exports/<image>.ome.tif`, as a
    /// background job: its id. It doesn't wait for the global lock.
    #[action(global_lock = false)]
    async fn export_ome_tiff(&self, dataset: DatasetId, image: String) -> Result<u64, ActionError> {
        let instrument = bound(&self.instrument);
        let (folder, path) = image_of(instrument, dataset, &image)?;
        let job = OmeTiffExportJob {
            image: path,
            out: folder.join("exports").join(format!("{image}.ome.tif")),
            compression: TiffCompression::Deflate,
        };
        let id = instrument
            .jobs
            .submit(Box::new(job), Priority::Background, Some(folder));
        Ok(id.0)
    }

    /// Exports plane (`c`, `z`, `t`) of image `image` of dataset `dataset` as a static DZI
    /// pyramid, in the channel's own window and colour, to `exports/dzi/<image>_c<c>_z<z>_t<t>.dzi`
    /// and its tiles, `jpeg` or `png`: a background job, whose id it gives. It doesn't wait for the
    /// global lock.
    #[action(global_lock = false)]
    async fn export_dzi(
        &self,
        dataset: DatasetId,
        image: String,
        #[param(default)] c: u64,
        #[param(default)] z: u64,
        #[param(default)] t: u64,
        #[param(default)] format: Option<String>,
    ) -> Result<u64, ActionError> {
        let instrument = bound(&self.instrument);
        let (folder, path) = image_of(instrument, dataset, &image)?;
        let extension = format.as_deref().unwrap_or("jpeg");
        let format = TileFormat::from_extension(extension)
            .with_context(|| format!("tiles are jpeg or png, not {extension}"))?;
        let job = DziExportJob {
            image: path,
            plane: (t, c, z),
            lut: None,
            format,
            out: folder.join("exports").join("dzi"),
            name: format!("{image}_c{c}_z{z}_t{t}"),
        };
        let id = instrument
            .jobs
            .submit(Box::new(job), Priority::Background, Some(folder));
        Ok(id.0)
    }

    /// Deletes dataset `dataset`: its folder, and everything in it. Refused while a job is waiting
    /// or running on it; false if there is no such dataset. Only an admin may, from M9. It doesn't
    /// wait for the global lock.
    #[action(global_lock = false)]
    async fn delete(&self, dataset: DatasetId) -> Result<bool, ActionError> {
        let instrument = bound(&self.instrument);
        let Some(summary) = instrument.catalog.get(dataset) else {
            return Ok(false);
        };
        let jobs = instrument.jobs.snapshot();
        let mut busy = jobs.queued.iter().chain(&jobs.running);
        if let Some(job) = busy.find(|job| job.dataset.as_deref() == Some(summary.path.as_path())) {
            let why = anyhow!("job {} ({}) is working on it", job.id.0, job.kind);
            return Err(why.into());
        }
        let deleted = instrument.catalog.delete(dataset)?;
        if deleted {
            let detail =
                serde_json::json!({ "id": dataset, "name": summary.name, "path": summary.path });
            let event = siq_auth::Event::new(siq_auth::Kind::DatasetDeleted, detail);
            instrument.auth.audit.record(event);
        }
        Ok(deleted)
    }
}

/// Frame `c`'s key in a snap's dataset.
fn key(c: usize) -> FrameKey {
    FrameKey {
        t: 0,
        c: c as u16,
        z: 0,
        p: 0,
    }
}

/// The folder of dataset `dataset` and of its image `image`, if both are there.
fn image_of(
    instrument: &Instrument,
    dataset: DatasetId,
    image: &str,
) -> anyhow::Result<(PathBuf, PathBuf)> {
    let summary = instrument
        .catalog
        .get(dataset)
        .with_context(|| format!("no dataset {dataset}"))?;
    if !images(&summary.path).iter().any(|name| name == image) {
        bail!("dataset {dataset} has no image {image}");
    }
    let path = summary.path.join("zarr").join(format!("{image}.ome.zarr"));
    Ok((summary.path, path))
}

/// The image a snap's frames make: a channel per frame, named and coloured as its channel, in its
/// frame's auto-contrast window. The pixel size is the calibration's, binned, or without one the
/// sensor's through the objective's magnification. The first pixel's centre is put where the
/// stage was, at the frame's centre: the image's axes taken as the stage's, as NGFF 0.5 has no
/// rotation.
fn image_info(
    instrument: &Instrument,
    channels: &[Option<ChannelConfig>],
    taken: &[(FrameMeta, (u16, u16))],
) -> ImageInfo {
    let (meta, _) = taken[0];
    let (width, height) = meta.image_size();
    let objective = instrument
        .config
        .objectives
        .iter()
        .find(|objective| Some(objective.id) == meta.objective);
    let magnification = objective.map_or(1.0, |objective| objective.magnification);
    let nominal = instrument.camera.capabilities().pixel_size.0 / magnification;
    let calibrated = meta
        .calibration
        .and_then(|id| instrument.calibrations.get(id))
        .map(|record| record.px_size_um);
    let pixel = calibrated.unwrap_or(nominal) * f64::from(meta.binning.horizontal.get());
    let origin = meta.pose.map_or([0.0; 3], |pose| {
        [
            pose.focus.0,
            pose.y.0 - (f64::from(height) - 1.0) / 2.0 * pixel,
            pose.x.0 - (f64::from(width) - 1.0) / 2.0 * pixel,
        ]
    });
    let channels = channels
        .iter()
        .zip(taken)
        .map(|(channel, (_, (min, max)))| ChannelInfo {
            label: channel
                .as_ref()
                .map_or_else(|| "frame".to_owned(), |channel| channel.name.clone()),
            colour: channel
                .as_ref()
                .map_or([255; 3], |channel| channel.colour.0),
            window: (f64::from(*min), f64::from(*max)),
        })
        .collect::<Vec<_>>();
    ImageInfo {
        name: SNAP_IMAGE.to_owned(),
        sample: SampleType::U16,
        size: Size5 {
            t: 1,
            c: channels.len() as u64,
            z: 1,
            y: u64::from(height),
            x: u64::from(width),
        },
        levels: 1,
        chunk: CHUNK,
        pixel_size: [1.0, pixel, pixel],
        origin,
        channels,
    }
}
