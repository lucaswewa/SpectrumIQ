//! The jobs the service runs, each around a processing crate's work: here because processing
//! (L3) can't depend on the job system (L4).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use siq_calib::shading::{Estimator, FlatField, ShadingRecord, ShadingRegistry};
use siq_imgproc::{Histogram, histogram, histogram_cfa, luminance};
use siq_pyramid::tiles::{DziGeometry, TileAt, TileFormat, TileLut, render};
use siq_pyramid::{Control, PyramidError, TOP};
use siq_stitch::composite::{self, Bounds, CompositeOptions, Placed, Plane};
use siq_stitch::{MappedTiles, Mosaic, Report, StitchError, StitchOptions, pairs, solve};
use siq_storage::ometiff::{self, TiffCompression};
use siq_storage::zarr::{CHUNK, ChannelInfo, ImageInfo, SampleType, Size5, ZarrImage};
use siq_storage::{Dataset, DatasetReader, FrameKey, StorageError};
use siq_types::{Affine2, ChannelId, ObjectiveId, OwnedImage, Roi};

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
        ..JobOutput::default()
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
            // A colour frame's luminance: one flat, for every colour (ADR-0071).
            match frame.sidecar.meta.cfa {
                None => estimator.add(&pixels),
                Some(_) => {
                    let mut seen = OwnedImage::new(pixels.width(), pixels.height());
                    luminance(&pixels, &mut seen.view_mut()).map_err(JobError::failed)?;
                    estimator.add(&seen.view())
                }
            }
            .map_err(JobError::failed)?;
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
            ..JobOutput::default()
        })
    }
}

/// A dataset's frames composited into one OME-Zarr mosaic at the corners given (ADR-0063), then
/// its pyramid built: the stitch's last stages, which the stitch job runs after its solve. Each
/// `(t, c, z)` of the frames given is a plane of the image, in their order.
pub struct CompositeJob {
    /// The dataset's folder.
    pub dataset: PathBuf,
    /// The image to make, such as `<dataset>/zarr/mosaic.ome.zarr`.
    pub image: PathBuf,
    /// Each frame composited, by its key, and its corner on the mosaic.
    pub corners: Vec<(FrameKey, [f64; 2])>,
    /// The calibration, sensor pixels a µm: the frames' size on the stage, and the image's.
    pub l: Affine2,
    /// The channels' labels, colours and windows, in the image's order.
    pub channels: Vec<ChannelInfo>,
    /// Each channel's dark level and flat, by its id: a channel without one is composited as it is.
    pub shading: Vec<(u16, f32, FlatField)>,
    /// How the frames are composited.
    pub options: CompositeOptions,
}

/// A part of a job's progress, as a builder's control: from `from`, `share` of the whole.
struct Part<'a> {
    ctx: &'a JobCtx,
    from: f64,
    share: f64,
}

impl Control for Part<'_> {
    fn progress(&self, done: f64) {
        self.ctx.progress(self.from + self.share * done);
    }

    fn cancelled(&self) -> bool {
        self.ctx.cancelled()
    }
}

/// The tiles kept mapped while compositing: a few for each thread.
fn mapped(threads: usize) -> usize {
    4 * threads.max(1)
}

/// The threads a job's estimate assumes: half the processors, as the pool has.
fn threads() -> usize {
    std::thread::available_parallelism().map_or(1, |n| n.get() / 2)
}

/// The bytes of the first frame of the dataset at `folder`.
fn frame_bytes(folder: &Path) -> u64 {
    Dataset::open(folder)
        .and_then(|dataset| DatasetReader::open(&dataset))
        .ok()
        .and_then(|reader| reader.frames().first().map(|frame| frame.sidecar.bytes))
        .unwrap_or(0)
}

fn stopped(error: StitchError) -> JobError {
    match error {
        StitchError::Cancelled => JobError::Cancelled,
        other => JobError::failed(other),
    }
}

impl CompositeJob {
    /// The image's planes, from the frames' `(t, c, z)`s in order, their frames' tiles, and each
    /// plane's channel id. Colour frames make three channels each: red, green and blue.
    fn planes(&self, mosaic: &Mosaic) -> (Size5, Vec<(Plane, u16)>) {
        let sorted = |pick: fn(&FrameKey) -> u32| {
            let mut values: Vec<u32> = self.corners.iter().map(|(key, _)| pick(key)).collect();
            values.sort_unstable();
            values.dedup();
            values
        };
        let (ts, cs, zs) = (
            sorted(|key| key.t),
            sorted(|key| u32::from(key.c)),
            sorted(|key| key.z),
        );
        let rank = |values: &[u32], value: u32| values.partition_point(|&v| v < value) as u64;
        let colours = mosaic
            .tiles
            .first()
            .and_then(|tile| tile.cfa)
            .map_or(1, |_| 3);
        let mut planes: Vec<(Plane, u16)> = Vec::new();
        for (source, ((key, corner), tile)) in self.corners.iter().zip(&mosaic.tiles).enumerate() {
            for colour in 0..colours {
                let c = rank(&cs, u32::from(key.c)) * colours + colour;
                let at = (rank(&ts, key.t), c, rank(&zs, key.z));
                let placed = Placed {
                    source,
                    tile: *tile,
                    corner: *corner,
                };
                match planes.iter_mut().find(|(plane, _)| plane.at == at) {
                    Some((plane, _)) => plane.tiles.push(placed),
                    None => planes.push((
                        Plane {
                            at,
                            tiles: vec![placed],
                            colour: (colours == 3).then_some(colour as usize),
                        },
                        key.c,
                    )),
                }
            }
        }
        let size = |values: &[u32]| values.len() as u64;
        let counts = Size5 {
            t: size(&ts),
            c: size(&cs) * colours,
            z: size(&zs),
            y: 0,
            x: 0,
        };
        (counts, planes)
    }

    /// The memory it holds at most, with `threads`.
    fn bytes(&self, threads: usize) -> u64 {
        // The composite, then the pyramid, whose builder holds five u16 chunks a thread and as
        // much again for the codec: the larger of the two.
        let frame = frame_bytes(&self.dataset);
        let composing =
            composite::bytes(CHUNK, &self.options, threads) + mapped(threads) as u64 * frame;
        composing.max(threads as u64 * 5 * CHUNK * CHUNK * 2 * 2)
    }

    /// Composites the frames and builds the pyramid, as the part of `ctx`'s progress from `from`,
    /// `share` of the whole: the image's bounds.
    fn compose(&self, ctx: &JobCtx, (from, share): (f64, f64)) -> Result<Bounds, JobError> {
        let dataset = Dataset::open(&self.dataset).map_err(JobError::failed)?;
        let reader = DatasetReader::open(&dataset).map_err(JobError::failed)?;
        // The frames in the order given, each by its key.
        let order: HashMap<FrameKey, usize> = self
            .corners
            .iter()
            .enumerate()
            .map(|(k, (key, _))| (*key, k))
            .collect();
        let (source, mut frames) =
            MappedTiles::open(&reader, |key| order.contains_key(key)).map_err(JobError::failed)?;
        let source = source.with_capacity(mapped(ctx.pool().current_num_threads()));
        if frames.len() != self.corners.len() {
            return Err(JobError::failed(
                "a frame to composite isn't in the dataset",
            ));
        }
        frames.sort_by_key(|(key, _)| order[key]);
        let mosaic = Mosaic::place(&frames, &self.l).map_err(JobError::failed)?;
        let (counts, planes) = self.planes(&mosaic);
        let corners: Vec<[f64; 2]> = self.corners.iter().map(|(_, corner)| *corner).collect();
        let sizes: Vec<[u32; 2]> = mosaic.tiles.iter().map(|tile| tile.size).collect();
        let bounds = Bounds::of(&corners, &sizes);
        // The image's pixel on the stage: a mosaic pixel `m` shows the stage at `L⁻¹·m·binning`.
        let binning = f64::from(mosaic.tiles.first().map_or(1, |tile| tile.binning));
        let inverse = self
            .l
            .invert()
            .ok_or_else(|| JobError::failed("the calibration is singular"))?;
        let first = bounds.origin.map(|at| (at as f64 + 0.5) * binning);
        let [x, y] = inverse.apply_vector(first);
        let pixel = 1.0 / mosaic.px_per_um;
        let info = ImageInfo {
            name: self
                .image
                .file_name()
                .and_then(|name| name.to_str())
                .and_then(|name| name.split('.').next())
                .unwrap_or("mosaic")
                .to_owned(),
            sample: SampleType::U16,
            size: Size5 {
                y: bounds.size[1],
                x: bounds.size[0],
                ..counts
            },
            levels: 1,
            chunk: CHUNK,
            pixel_size: [1.0, pixel, pixel],
            origin: [0.0, y, x],
            channels: self.channels.clone(),
            cfa: None,
        };
        let mut image = ZarrImage::create(&self.image, info).map_err(JobError::failed)?;
        ctx.log(format!(
            "compositing {} frames into {} × {} pixels, {} plane(s)",
            self.corners.len(),
            bounds.size[0],
            bounds.size[1],
            planes.len()
        ));
        // A channel at a time, through its own flat.
        let mut channels: Vec<u16> = planes.iter().map(|(_, c)| *c).collect();
        channels.sort_unstable();
        channels.dedup();
        let each = 0.8 * share / channels.len().max(1) as f64;
        for (k, c) in channels.iter().enumerate() {
            let its: Vec<Plane> = planes
                .iter()
                .filter(|(_, of)| of == c)
                .map(|(plane, _)| plane.clone())
                .collect();
            let flat = self.shading.iter().find(|(of, ..)| of == c);
            let shading = flat.map(|(_, dark, flat)| (*dark, flat));
            let part = Part {
                ctx,
                from: from + each * k as f64,
                share: each,
            };
            ctx.pool()
                .install(|| {
                    composite::composite(
                        &source,
                        &its,
                        &bounds,
                        &image,
                        shading,
                        &self.options,
                        &part,
                    )
                })
                .map_err(stopped)?;
        }
        ctx.log("building the pyramid");
        let part = Part {
            ctx,
            from: from + 0.8 * share,
            share: 0.2 * share,
        };
        ctx.pool()
            .install(|| siq_pyramid::build(&mut image, TOP, &part))
            .map_err(|error| match error {
                PyramidError::Cancelled => JobError::Cancelled,
                PyramidError::Storage(error) => JobError::failed(error),
            })?;
        ctx.log(format!("{} levels built", image.info().levels));
        Ok(bounds)
    }
}

impl Job for CompositeJob {
    fn kind(&self) -> &str {
        "composite"
    }

    fn params(&self) -> serde_json::Value {
        serde_json::json!({ "image": self.image, "frames": self.corners.len(), "options": self.options })
    }

    fn estimate(&self) -> Resources {
        let threads = threads();
        Resources {
            ram: self.bytes(threads),
            vram: 0,
            gpu: false,
            threads,
        }
    }

    fn run(&mut self, ctx: &JobCtx) -> Result<JobOutput, JobError> {
        self.compose(ctx, (0.0, 1.0))?;
        Ok(output(ctx, &self.image))
    }
}

/// A tile scan stitched (ADR-0061 to ADR-0064): its reference plane's pairs registered and solved,
/// every frame composited at its tile's corner, the pyramid built, and the QA report written: the
/// image `zarr/<image>.ome.zarr` and `qa/<image>.json`, and the report's summary in the dataset's
/// history. The reference plane is the first frame's `(t, c, z)`.
pub struct StitchJob {
    /// The dataset's folder.
    pub dataset: PathBuf,
    /// The image's name, such as `stitched`.
    pub image: String,
    /// The calibration the frames were taken with, sensor pixels a µm.
    pub l: Affine2,
    /// The channels' labels and colours, by id; another's is `channel <id>`, in white.
    pub channels: Vec<(u16, String, [u8; 3])>,
    /// Each channel's dark level and flat, by its id.
    pub shading: Vec<(u16, f32, FlatField)>,
    /// How it is stitched.
    pub options: StitchOptions,
}

impl StitchJob {
    /// The channels' labels, colours and windows, in id order: the window from the 0.1 and 99.9
    /// percentiles of each channel's first frame. A colour frame's channel is three, its red,
    /// green and blue, each through its own window: its white balance (ADR-0071).
    fn infos(&self, reader: &DatasetReader) -> Result<Vec<ChannelInfo>, JobError> {
        let mut ids: Vec<u16> = reader
            .frames()
            .iter()
            .map(|frame| frame.sidecar.key.c)
            .collect();
        ids.sort_unstable();
        ids.dedup();
        ids.iter()
            .map(|&c| {
                let frame = reader
                    .frames()
                    .iter()
                    .find(|frame| frame.sidecar.key.c == c);
                let mapped = frame.map(|frame| frame.map()).transpose();
                let mapped = mapped.map_err(JobError::failed)?;
                let cfa = frame.and_then(|frame| frame.sidecar.meta.cfa);
                let mut counts: [Histogram; 3] = std::array::from_fn(|_| Histogram::new(16));
                let view = mapped.as_ref().and_then(|mapped| mapped.image_u16());
                if let Some(view) = view {
                    let _ = match cfa {
                        Some(cfa) => histogram_cfa(&view, cfa, 2, &mut counts),
                        None => histogram(&view, 4, &mut counts[0]),
                    };
                }
                let window = |counts: &Histogram| match counts.count() {
                    0 => (0.0, 65_535.0),
                    _ => {
                        let (low, high) = counts.auto_window(0.001, 0.999);
                        (f64::from(low), f64::from(high))
                    }
                };
                let named = self.channels.iter().find(|(id, ..)| *id == c);
                let label =
                    named.map_or_else(|| format!("channel {c}"), |(_, name, _)| name.clone());
                let colour = named.map_or([255; 3], |(.., colour)| *colour);
                Ok(match cfa {
                    None => vec![ChannelInfo {
                        label,
                        colour,
                        window: window(&counts[0]),
                    }],
                    Some(_) => [
                        ("red", [255, 0, 0]),
                        ("green", [0, 255, 0]),
                        ("blue", [0, 0, 255]),
                    ]
                    .into_iter()
                    .zip(&counts)
                    .map(|((name, colour), counts)| ChannelInfo {
                        label: format!("{label} {name}"),
                        colour,
                        window: window(counts),
                    })
                    .collect(),
                })
            })
            .collect::<Result<Vec<Vec<_>>, _>>()
            .map(|channels| channels.into_iter().flatten().collect())
    }
}

impl Job for StitchJob {
    fn kind(&self) -> &str {
        "stitch"
    }

    fn params(&self) -> serde_json::Value {
        serde_json::json!({ "image": self.image, "options": self.options })
    }

    fn estimate(&self) -> Resources {
        let threads = threads();
        // The pairs first, each holding 16 bytes a pixel of an overlap of up to half a frame and
        // its FFTs; then the composite and the pyramid.
        let pixels = frame_bytes(&self.dataset) / 2;
        let search = pairs::search_bytes(&self.options.pairs);
        let pairs = threads as u64 * (16 * pixels / 2 + search);
        let composing = CompositeJob {
            dataset: self.dataset.clone(),
            image: PathBuf::new(),
            corners: Vec::new(),
            l: self.l,
            channels: Vec::new(),
            shading: Vec::new(),
            options: self.options.composite,
        };
        Resources {
            ram: pairs.max(composing.bytes(threads)),
            vram: 0,
            gpu: false,
            threads,
        }
    }

    fn run(&mut self, ctx: &JobCtx) -> Result<JobOutput, JobError> {
        let dataset = Dataset::open(&self.dataset).map_err(JobError::failed)?;
        let reader = DatasetReader::open(&dataset).map_err(JobError::failed)?;
        let first = reader.frames().first();
        let first = first.ok_or_else(|| JobError::failed("the dataset has no frames"))?;
        let reference = first.sidecar.key;
        let plane = |key: &FrameKey| (key.t, key.c, key.z);
        let same = |key: &FrameKey| plane(key) == plane(&reference);
        let (source, frames) = MappedTiles::open(&reader, same).map_err(JobError::failed)?;
        let threads = ctx.pool().current_num_threads();
        let source = source.with_capacity(mapped(threads));
        let mosaic = Mosaic::place(&frames, &self.l).map_err(JobError::failed)?;
        let options = &self.options;
        let (least, share) = (
            options.pairs.min_overlap_px,
            options.pairs.min_overlap_share,
        );
        let found = mosaic.overlaps(least, share);
        ctx.log(format!(
            "registering {} pairs of {} tiles",
            found.len(),
            mosaic.tiles.len()
        ));
        let flat = self.shading.iter().find(|(c, ..)| *c == reference.c);
        let shading = flat.map(|(_, dark, flat)| (*dark, flat));
        let pairing = &options.pairs;
        let edges = ctx
            .pool()
            .install(|| pairs::register(&source, &mosaic, &found, shading, pairing, threads))
            .map_err(stopped)?;
        ctx.check()?;
        ctx.progress(0.25);
        let solution = solve::solve(&mosaic, &edges, &options.solve);
        ctx.log(format!(
            "solved by {:?}: the edges used at {:.3} px RMS, {:.3} px at worst; {} component(s)",
            solution.method, solution.rms_px, solution.max_px, solution.components
        ));
        ctx.progress(0.3);

        // Every frame at its tile's corner, by its position in the scan.
        let by_p: HashMap<u32, [f64; 2]> = mosaic
            .tiles
            .iter()
            .zip(&solution.positions)
            .map(|(tile, at)| (tile.key.p, *at))
            .collect();
        let corners: Vec<(FrameKey, [f64; 2])> = reader
            .frames()
            .iter()
            .filter_map(|frame| {
                let key = frame.sidecar.key;
                by_p.get(&key.p).map(|corner| (key, *corner))
            })
            .collect();
        let image = dataset.zarr_dir().join(format!("{}.ome.zarr", self.image));
        let composing = CompositeJob {
            dataset: self.dataset.clone(),
            image: image.clone(),
            corners,
            l: self.l,
            channels: self.infos(&reader)?,
            shading: self.shading.clone(),
            options: options.composite,
        };
        let bounds = composing.compose(ctx, (0.3, 0.7))?;

        let report = Report::new(&mosaic, &edges, &solution, &bounds);
        let qa = dataset.qa_dir();
        let path = qa.join(format!("{}.json", self.image));
        std::fs::create_dir_all(&qa).map_err(JobError::failed)?;
        let text = serde_json::to_vec_pretty(&report).map_err(JobError::failed)?;
        std::fs::write(&path, text).map_err(JobError::failed)?;
        let mut done = output(ctx, &image);
        done.outputs.extend(output(ctx, &path).outputs);
        done.results = serde_json::to_value(report.summary).map_err(JobError::failed)?;
        Ok(done)
    }
}
