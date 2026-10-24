//! Synthetic specimens: deterministic worlds with exact ground truth, for the camera simulator and
//! for generated datasets (REQ-SIM-004), and real images as worlds (REQ-SIM-005). ADR-0024.
//!
//! - A [`World`] is a pure function of sample coordinates, which are stage coordinates
//!   (ADR-0018). Sampling two regions apart gives the same value wherever they share a point.
//! - [`sample`] renders a world on any pixel grid: axis-aligned from [`grid`], or a frame's own
//!   [`PixelToStage`] mapping.
//! - The worlds: [`Cells`], [`Grains`], [`HeightField`], [`Beads`] and [`ImageWorld`].
//! - [`WorldInfo`] names a world and its parameters, so a world can be rebuilt from it; ground
//!   truth is written by [`write_ground_truth`].

use std::path::PathBuf;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use siq_types::{Affine2, ImageViewMut, Micrometers, PixelToStage, PointPx, RectUm, Vec2Um};

mod beads;
mod cells;
mod grains;
mod height;
mod image_world;
mod noise;
mod truth;

pub use beads::{Beads, BeadsParams};
pub use cells::{Cells, CellsParams};
pub use grains::{Grains, GrainsParams};
pub use height::{HeightField, HeightFieldParams};
pub use image_world::{ImageParams, ImageWorld};
pub use truth::{GroundTruth, HeightMap, WorldInfo, write_ground_truth};

/// A virtual specimen.
pub trait World: Send + Sync {
    /// The in-focus intensity at `at`, from 0 to 1, as a pixel `footprint` wide sees it: detail
    /// finer than the footprint is averaged away.
    fn intensity(&self, at: Vec2Um, footprint: Micrometers) -> f32;

    /// The height of the surface under `at`, for 2D and 2.5D worlds; `None` for volumes.
    fn surface_z(&self, _at: Vec2Um) -> Option<Micrometers> {
        None
    }

    /// The density at a point of a volume, from 0; `None` for worlds that aren't volumes.
    fn density(&self, _at: Vec2Um, _z: Micrometers) -> Option<f32> {
        None
    }

    /// The beads whose centres lie in `region`, for worlds made of beads.
    fn beads(&self, _region: RectUm) -> Vec<Bead> {
        Vec::new()
    }

    /// What the world is, and its parameters.
    fn info(&self) -> WorldInfo;
}

/// A fluorescent bead: a 3D Gaussian.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Bead {
    /// The centre's x.
    pub x: Micrometers,
    /// The centre's y.
    pub y: Micrometers,
    /// The centre's z.
    pub z: Micrometers,
    /// The Gaussian's standard deviation, the same on every axis.
    pub sigma: Micrometers,
    /// The density at the centre, from 0 to 1.
    pub brightness: f32,
}

/// Samples `world` into `out`. Pixel `(column, row)` takes the intensity at `to_stage` of its
/// centre, `(column + 0.5, row + 0.5)`, with the pixel's footprint.
pub fn sample<W: World + ?Sized>(world: &W, to_stage: &PixelToStage, out: &mut ImageViewMut<f32>) {
    let footprint = Micrometers(to_stage.0.determinant().abs().sqrt());
    for (row, line) in (0u32..).zip(out.rows_mut()) {
        for (column, value) in (0u32..).zip(line.iter_mut()) {
            let at = to_stage.apply(PointPx::pixel_centre(column, row));
            *value = world.intensity(at, footprint);
        }
    }
}

/// The axis-aligned grid of `scale` µm pixels whose pixel `(0, 0)` has its corner at `origin`.
pub fn grid(origin: Vec2Um, scale: Micrometers) -> PixelToStage {
    let [x, y] = origin.to_um();
    PixelToStage(Affine2::new([[scale.0, 0.0, x], [0.0, scale.0, y]]))
}

/// Why a world couldn't be made, or its ground truth written.
#[derive(Debug, thiserror::Error)]
pub enum SynthError {
    /// A file couldn't be read or written.
    #[error("{}: {source}", path.display())]
    Io {
        /// The file.
        path: PathBuf,
        /// What went wrong.
        source: std::io::Error,
    },
    /// An image couldn't be decoded, or isn't an image a world can use.
    #[error("{}: {message}", path.display())]
    Image {
        /// The file.
        path: PathBuf,
        /// What is wrong with it.
        message: String,
    },
}
