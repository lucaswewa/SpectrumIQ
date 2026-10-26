//! The jobs the service runs, each around a processing crate's work: here because processing
//! (L3) can't depend on the job system (L4).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use siq_calib::shading::{Estimator, ShadingRecord, ShadingRegistry};
use siq_pyramid::tiles::{DziGeometry, TileAt, TileFormat, TileLut, render};
use siq_pyramid::{Control, PyramidError, TOP};
use siq_storage::ometiff::{self, TiffCompression};
use siq_storage::zarr::{ImageInfo, ZarrImage};
use siq_storage::{Dataset, DatasetReader, FrameKey, StorageError};
use siq_types::{ChannelId, ObjectiveId, Roi};

use crate::{Job, JobCtx, JobError, JobOutput, Resources};

/// Writes a dataset's raw frames as level 0 of a new OME-Zarr image: the snap pipeline's first step
/// (ADR-0041, as amended in P4.7). Frame `c` of `frames` is the image's channel `c`. The job must
/// be given its dataset.
#[derive(Debug, Clone, PartialEq)]
pub struct ZarrFromRawJob {
    /// The image to make, such as `<dataset>/zarr/snap.ome.zarr`.
    pub image: PathBuf,
    /// Its metadata, of one level: its `c` is the frames' count, its `y` and `x` their size.
    pub info: ImageInfo,
    /// The frames, by key: one per channel, in order.
    pub frames: Vec<FrameKey>,
}

impl Job for ZarrFromRawJob {
    fn kind(&self) -> &str {
        "zarr_from_raw"
    }

    fn params(&self) -> serde_json::Value {
        let frames: Vec<String> = self.frames.iter().map(ToString::to_string).collect();
        serde_json::json!({ "image": self.image, "frames": frames })
    }

    fn estimate(&self) -> Resources {
        let threads = std::thread::available_parallelism().map_or(1, |n| n.get() / 2);
        // Each thread's chunk, padded, and its encoding: the frames are mapped, not read in.
        let chunk = self.info.chunk * self.info.chunk * 2;
        Resources {
            ram: 3 * chunk * threads as u64,
            vram: 0,
            gpu: false,
            threads,
        }
    }

    fn run(&mut self, ctx: &JobCtx) -> Result<JobOutput, JobError> {
        let dataset = ctx
            .dataset()
            .ok_or_else(|| JobError::failed("the job needs its dataset"))?;
        let dataset = Dataset::open(dataset).map_err(JobError::failed)?;
        let reader = DatasetReader::open(&dataset).map_err(JobError::failed)?;
        let image = ZarrImage::create(&self.image, self.info.clone()).map_err(JobError::failed)?;
        for (c, key) in self.frames.iter().enumerate() {
            ctx.check()?;
            let frame = reader
                .frames()
                .iter()
                .find(|frame| frame.sidecar.key == *key);
            let frame = frame.ok_or_else(|| JobError::failed(format!("no raw frame {key}")))?;
            let mapped = frame.map().map_err(JobError::failed)?;
            let pixels = mapped
                .image_u16()
                .ok_or_else(|| JobError::failed(format!("frame {key} isn't of 16-bit pixels")))?;
            ctx.pool()
                .install(|| image.write_plane(0, (0, c as u64, 0), pixels))
                .map_err(JobError::failed)?;
            ctx.progress((c + 1) as f64 / self.frames.len() as f64);
        }
        Ok(output(ctx, &self.image))
    }
}

/// Builds an OME-Zarr image's pyramid (ADR-0044): its levels 1 and up, from level 0.
#[derive(Debug, Clone, PartialEq)]
pub struct PyramidJob {
    /// The image's folder, such as `<dataset>/zarr/mosaic.ome.zarr`.
    pub image: PathBuf,
    /// The top level's largest size, in pixels: [`TOP`] by default.
    pub top: u64,
}

impl PyramidJob {
    /// The pyramid of the image at `image`, up to a top level of [`TOP`] pixels.
    pub fn new(image: impl Into<PathBuf>) -> Self {
        Self {
            image: image.into(),
            top: TOP,
        }
    }
}

/// A job's context, as the pyramid builder's control.
struct Steer<'a>(&'a JobCtx);

impl Control for Steer<'_> {
    fn progress(&self, done: f64) {
        self.0.progress(done);
    }

    fn cancelled(&self) -> bool {
        self.0.cancelled()
    }
}

impl Job for PyramidJob {
    fn kind(&self) -> &str {
        "pyramid"
    }

    fn params(&self) -> serde_json::Value {
        serde_json::json!({ "image": self.image, "top": self.top })
    }

    fn estimate(&self) -> Resources {
        let threads = std::thread::available_parallelism().map_or(1, |n| n.get() / 2);
        let ram = ZarrImage::open(&self.image)
            .map_or(0, |image| siq_pyramid::estimate(image.info(), threads));
        Resources {
            ram,
            vram: 0,
            gpu: false,
            threads,
        }
    }

    fn run(&mut self, ctx: &JobCtx) -> Result<JobOutput, JobError> {
        let mut image = ZarrImage::open(&self.image).map_err(JobError::failed)?;
        let levels = image.info().levels;
        ctx.log(format!("building the pyramid of {}", self.image.display()));
        ctx.pool()
            .install(|| siq_pyramid::build(&mut image, self.top, &Steer(ctx)))
            .map_err(|error| match error {
                PyramidError::Cancelled => JobError::Cancelled,
                PyramidError::Storage(error) => JobError::failed(error),
            })?;
        ctx.log(format!(
            "levels {levels} to {} built",
            image.info().levels - 1
        ));
        Ok(output(ctx, &self.image))
    }
}

/// A job's output at `path`: a path in its dataset, if it is in one.
fn output(ctx: &JobCtx, path: &Path) -> JobOutput {
    let path = ctx
        .dataset()
        .and_then(|dataset| path.strip_prefix(dataset).ok())
        .unwrap_or(path);
    JobOutput {
        outputs: vec![path.to_string_lossy().replace('\\', "/")],
    }
}

/// Writes a plane of an OME-Zarr image as a static DZI pyramid (ADR-0045): `<out>/<name>.dzi`, and
/// every level's tiles in `<out>/<name>_files/<level>/<col>_<row>.<format>`, as the tile service
/// renders them.
#[derive(Debug, Clone, PartialEq)]
pub struct DziExportJob {
    /// The image's folder.
    pub image: PathBuf,
    /// The plane: time point, channel and focus plane.
    pub plane: (u64, u64, u64),
    /// The mapping: the channel's own if `None`.
    pub lut: Option<TileLut>,
    /// The tiles' format.
    pub format: TileFormat,
    /// The folder to write in, such as `<dataset>/exports/dzi`.
    pub out: PathBuf,
    /// The descriptor's name, without `.dzi`.
    pub name: String,
}

impl Job for DziExportJob {
    fn kind(&self) -> &str {
        "export_dzi"
    }

    fn params(&self) -> serde_json::Value {
        serde_json::json!({
            "image": self.image, "plane": self.plane, "format": self.format.extension(),
            "out": self.out, "name": self.name,
        })
    }

    fn estimate(&self) -> Resources {
        let threads = std::thread::available_parallelism().map_or(1, |n| n.get() / 2);
        // A tile's region, its levels and its encoding, per thread.
        Resources {
            ram: threads as u64 * 4_000_000,
            vram: 0,
            gpu: false,
            threads,
        }
    }

    fn run(&mut self, ctx: &JobCtx) -> Result<JobOutput, JobError> {
        use rayon::prelude::*;
        let image = ZarrImage::open(&self.image).map_err(JobError::failed)?;
        let size = image.info().size;
        let geometry = DziGeometry {
            width: size.x,
            height: size.y,
        };
        let files = self.out.join(format!("{}_files", self.name));
        let tiles: Vec<TileAt> = (0..=geometry.max_level())
            .flat_map(|level| {
                let (cols, rows) = geometry.tiles(level);
                (0..rows).flat_map(move |row| (0..cols).map(move |col| (level, col, row)))
            })
            .map(|(level, col, row)| TileAt {
                plane: self.plane,
                level,
                col,
                row,
            })
            .collect();
        ctx.log(format!(
            "exporting {} tiles to {}",
            tiles.len(),
            files.display()
        ));
        let done = std::sync::atomic::AtomicUsize::new(0);
        ctx.pool().install(|| {
            tiles.par_iter().try_for_each(|&at| {
                ctx.check()?;
                let folder = files.join(at.level.to_string());
                let bytes = render(&image, at, self.lut, self.format).map_err(JobError::failed)?;
                std::fs::create_dir_all(&folder).map_err(JobError::failed)?;
                let name = format!("{}_{}.{}", at.col, at.row, self.format.extension());
                std::fs::write(folder.join(name), bytes).map_err(JobError::failed)?;
                let finished = done.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                ctx.progress(finished as f64 / tiles.len() as f64);
                Ok::<_, JobError>(())
            })
        })?;
        let descriptor = self.out.join(format!("{}.dzi", self.name));
        std::fs::write(&descriptor, geometry.descriptor(self.format)).map_err(JobError::failed)?;
        Ok(output(ctx, &descriptor))
    }
}

/// Writes an OME-Zarr image as one OME-TIFF file (ADR-0046): every plane and level, with the
/// OME-XML of the image and of its dataset's manifest, if the job has a dataset.
#[derive(Debug, Clone, PartialEq)]
pub struct OmeTiffExportJob {
    /// The image's folder.
    pub image: PathBuf,
    /// The file to write, such as `<dataset>/exports/mosaic.ome.tif`.
    pub out: PathBuf,
    /// How the tiles are compressed.
    pub compression: TiffCompression,
}

impl Job for OmeTiffExportJob {
    fn kind(&self) -> &str {
        "export_ome_tiff"
    }

    fn params(&self) -> serde_json::Value {
        let compression = match self.compression {
            TiffCompression::None => "none",
            TiffCompression::Deflate => "deflate",
        };
        serde_json::json!({ "image": self.image, "out": self.out, "compression": compression })
    }

    fn estimate(&self) -> Resources {
        let threads = std::thread::available_parallelism().map_or(1, |n| n.get() / 2);
        let ram = ZarrImage::open(&self.image)
            .map_or(0, |image| ometiff::estimate(image.info(), threads));
        Resources {
            ram,
            vram: 0,
            gpu: false,
            threads,
        }
    }

    fn run(&mut self, ctx: &JobCtx) -> Result<JobOutput, JobError> {
        let image = ZarrImage::open(&self.image).map_err(JobError::failed)?;
        let dataset = ctx.dataset().map(Dataset::open).transpose();
        let dataset = dataset.map_err(JobError::failed)?;
        if let Some(folder) = self.out.parent() {
            std::fs::create_dir_all(folder).map_err(JobError::failed)?;
        }
        ctx.log(format!(
            "exporting {} to {}",
            self.image.display(),
            self.out.display()
        ));
        let manifest = dataset.as_ref().map(Dataset::manifest);
        ctx.pool()
            .install(|| ometiff::export(&image, manifest, &self.out, self.compression, &Steer(ctx)))
            .map_err(|error| match error {
                StorageError::Cancelled => JobError::Cancelled,
                error => JobError::failed(error),
            })?;
        Ok(output(ctx, &self.out))
    }
}

/// Estimates the flat of a dataset's frames, its tiles of different places, and keeps it in the
/// registry for their objective and channel (ADR-0050, BaSiC simplified). The frames of another
/// channel, or not of the whole sensor, are left out. The job must be given its dataset.
#[derive(Debug, Clone)]
pub struct ShadingJob {
    /// Where the flat is kept.
    pub registry: Arc<ShadingRegistry>,
    /// The objective and channel the flat is for: the frames of that channel.
    pub key: (Option<ObjectiveId>, Option<ChannelId>),
    /// The dark level the frames are measured less, in ADU.
    pub dark: f64,
    /// The sensor's width and height, in pixels.
    pub sensor: [u32; 2],
}

impl Job for ShadingJob {
    fn kind(&self) -> &str {
        "estimate_shading"
    }

    fn params(&self) -> serde_json::Value {
        let (objective, channel) = self.key;
        serde_json::json!({ "objective": objective, "channel": channel, "dark": self.dark })
    }

    fn estimate(&self) -> Resources {
        // The frames are mapped one at a time; each is kept shrunk to the flat's grid.
        Resources {
            ram: 256 << 20,
            vram: 0,
            gpu: false,
            threads: 1,
        }
    }

    fn run(&mut self, ctx: &JobCtx) -> Result<JobOutput, JobError> {
        let dataset = ctx
            .dataset()
            .ok_or_else(|| JobError::failed("the job needs its dataset"))?;
        let dataset = Dataset::open(dataset).map_err(JobError::failed)?;
        let reader = DatasetReader::open(&dataset).map_err(JobError::failed)?;
        let whole = Roi::full(self.sensor[0], self.sensor[1]);
        let frames: Vec<_> = reader
            .frames()
            .iter()
            .filter(|frame| {
                let meta = &frame.sidecar.meta;
                meta.roi == whole && (self.key.1.is_none() || meta.channel == self.key.1)
            })
            .collect();
        let mut estimator = Estimator::new(self.dark as f32);
        let mut last = None;
        for (n, frame) in frames.iter().enumerate() {
            ctx.check()?;
            let mapped = frame.map().map_err(JobError::failed)?;
            let pixels = mapped
                .image_u16()
                .ok_or_else(|| JobError::failed("only 16-bit frames can be estimated from"))?;
            estimator.add(&pixels).map_err(JobError::failed)?;
            last = Some(frame.sidecar.meta);
            ctx.progress((n + 1) as f64 / frames.len() as f64);
        }
        let meta = last.ok_or_else(|| JobError::failed("no frames of the whole sensor"))?;
        let flat = estimator
            .finish(whole, self.sensor)
            .map_err(JobError::failed)?;
        let record = ShadingRecord::new(self.key, &meta, frames.len(), "estimate");
        let record = record.with_flat(flat, self.dark);
        let path = self.registry.save(&record, &[]).map_err(JobError::failed)?;
        ctx.log(format!(
            "the flat of {} frames, kept as {}",
            frames.len(),
            record.id
        ));
        Ok(JobOutput {
            outputs: vec![path.to_string_lossy().replace('\\', "/")],
        })
    }
}
