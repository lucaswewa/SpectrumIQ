//! The jobs the service runs, each around a processing crate's work: here because processing
//! (L3) can't depend on the job system (L4).

use std::path::PathBuf;

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
