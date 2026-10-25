//! The jobs the service runs, each around a processing crate's work: here because processing
//! (L3) can't depend on the job system (L4).

use std::path::PathBuf;

use siq_pyramid::tiles::{DziGeometry, TileAt, TileFormat, TileLut, render};
use siq_pyramid::{Control, PyramidError, TOP};
use siq_storage::zarr::ZarrImage;

use crate::{Job, JobCtx, JobError, JobOutput, Resources};

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
        // The image, as a path in the dataset if it is in one.
        let image = ctx
            .dataset()
            .and_then(|dataset| self.image.strip_prefix(dataset).ok())
            .unwrap_or(&self.image);
        Ok(JobOutput {
            outputs: vec![image.to_string_lossy().replace('\\', "/")],
        })
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
        let output = ctx
            .dataset()
            .and_then(|dataset| descriptor.strip_prefix(dataset).ok())
            .unwrap_or(&descriptor);
        Ok(JobOutput {
            outputs: vec![output.to_string_lossy().replace('\\', "/")],
        })
    }
}
