//! The live mosaic's canvas (ADR-0059): while the stage moves, each frame painted where its
//! calibration and pose put it, on a sparse canvas of GPU pages.
//!
//! - **The canvas** is the stage's plane, x right and y down as the stage's, at `canvas_scale`
//!   canvas pixels a sensor pixel (½ by default): stage µm and canvas pixels differ by a scale
//!   and an origin alone ([`CanvasGeometry`]).
//! - **Pages** of 4096² pixels are made where a frame first falls. A pixel is a 32-bit word: its
//!   value in the low half, how much of it is covered in the high half.
//! - **Painting** maps each pixel of the frame's footprint back into the frame
//!   ([`frame_to_canvas`] inverted), samples it bilinearly, and lays it over the page with an
//!   alpha that ramps from 0 at the frame's edge to 1 a feather in: the newest frame wins inside
//!   its feather band, and its edge never darkens what it covers alone.
//! - **The VRAM budget:** pages past it are evicted least recently used first, read back and kept
//!   in host RAM shrunk; a page painted again is restored.
//! - **The policy:** only frames whose pose is known paint ([`paints`]): settled, or moving with an
//!   uncertainty under the threshold.
//! - **On the GPU's thread:** a [`Canvas`] is used inside `siq-gpu`'s jobs; [`paint::cpu`] is the
//!   reference its kernel is held to.

mod canvas;

pub mod paint;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use siq_gpu::Shader;
use siq_types::{Affine2, Micrometers, PoseQuality};

pub use canvas::{Canvas, CanvasOptions, PAGE, Painted};

/// Every shader here: the tests validate each.
pub const SHADERS: &[Shader] = &[paint::SHADER];

/// Where the canvas is on the stage: canvas pixel = (stage µm − `origin`) × `px_per_um`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CanvasGeometry {
    /// The stage point at the canvas's (0, 0), in µm.
    pub origin: [f64; 2],
    /// Canvas pixels a µm.
    pub px_per_um: f64,
}

impl CanvasGeometry {
    /// The canvas at `canvas_scale` canvas pixels a sensor pixel, for the calibration `l` (sensor
    /// pixels a µm), its origin at the stage's 0.
    pub fn for_calibration(l: &Affine2, canvas_scale: f64) -> Self {
        Self {
            origin: [0.0, 0.0],
            px_per_um: canvas_scale * l.determinant().abs().sqrt(),
        }
    }

    /// Where stage point `at` is on the canvas.
    pub fn to_canvas(&self, [x, y]: [f64; 2]) -> [f64; 2] {
        [
            (x - self.origin[0]) * self.px_per_um,
            (y - self.origin[1]) * self.px_per_um,
        ]
    }

    /// Where canvas point `at` is on the stage.
    pub fn to_stage(&self, [x, y]: [f64; 2]) -> [f64; 2] {
        [
            x / self.px_per_um + self.origin[0],
            y / self.px_per_um + self.origin[1],
        ]
    }
}

/// What a frame's pixel coordinates are on the canvas: the frame `size` (binned pixels) with the
/// stage at `pose` µm, `binning` sensor pixels to a frame pixel, and its centre where the stage
/// is (`sensor px = c + L·(S − P)`). `None` if `l` is singular.
pub fn frame_to_canvas(
    l: &Affine2,
    size: [u32; 2],
    binning: u32,
    pose: [f64; 2],
    geometry: &CanvasGeometry,
) -> Option<Affine2> {
    let centre = [-f64::from(size[0]) / 2.0, -f64::from(size[1]) / 2.0];
    let bin = f64::from(binning);
    // Frame pixels from the centre, as sensor pixels; then the stage; then the canvas.
    let to_sensor = Affine2::scale(bin, bin).compose(&Affine2::translation(centre));
    let to_stage = Affine2::translation(pose).compose(&l.invert()?);
    let k = geometry.px_per_um;
    let to_canvas = Affine2::scale(k, k).compose(&Affine2::translation([
        -geometry.origin[0],
        -geometry.origin[1],
    ]));
    Some(to_canvas.compose(&to_stage).compose(&to_sensor))
}

/// Whether a frame of `quality` paints: settled, or moving with its pose within `uncertainty`.
pub fn paints(quality: &PoseQuality, uncertainty: Micrometers) -> bool {
    match quality {
        PoseQuality::Settled => true,
        PoseQuality::Interpolated { uncertainty: known } => known.0 < uncertainty.0,
        PoseQuality::Unknown => false,
    }
}
