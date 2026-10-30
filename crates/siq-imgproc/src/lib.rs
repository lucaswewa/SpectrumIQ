//! CPU image kernels for the frame path, the simulator, calibration and storage (ADR-0025).
//!
//! - [`correct`]: dark and flat-field correction.
//! - [`downsample_area`], [`bin`], [`copy`], [`gaussian_blur`], [`upsample_bilinear`]: resampling.
//! - [`histogram`], [`Histogram`], [`Lut`]: statistics and display mapping.
//! - [`focus`]: focus measures.
//! - [`demosaic`], [`histogram_cfa`], [`white_gains`], [`balanced`], [`map_colours`]: a colour
//!   filter's mosaic, shown in colour; [`luminance`], for processing it.
//!
//! Every kernel writes into a buffer its caller owns, and splits its rows into bands on rayon's
//! pool. Called from a pool thread, a kernel allocates nothing. The loops are plain, for the
//! compiler to vectorize; explicit SIMD waits for a measured need.

use siq_types::{ImageView, ImageViewMut};

mod bands;
mod bayer;
mod blur;
mod correct;
mod display;
mod focus;
mod resample;

pub use bayer::{balanced, demosaic, histogram_cfa, luminance, map_colours, white_gains};
pub use blur::{MAX_SIGMA, gaussian_blur};
pub use correct::{Calibration, correct, correct_in_place};
pub use display::{BINS, Colormap, DisplayMapping, Histogram, Lut, histogram};
pub use focus::{FocusMeasure, focus};
pub use resample::{BinMode, bin, copy, downsample_area, upsample_bilinear};

/// Runs `work` on every row of `out`, with the row's index, in the kernels' row bands: for other
/// crates' per-pixel passes.
pub fn for_each_row<T: Send>(out: &mut ImageViewMut<'_, T>, work: impl Fn(usize, &mut [T]) + Sync) {
    bands::for_bands(out.reborrow(), &|first, mut band| {
        for (y, row) in (first..).zip(band.rows_mut()) {
            work(y, row);
        }
    });
}

/// A pixel type the kernels compute with in `f32`.
pub trait Sample: Copy + Send + Sync + 'static {
    /// The value as `f32`.
    fn to_f32(self) -> f32;

    /// The nearest value to `value`, clamped to the type's range.
    fn from_f32(value: f32) -> Self;
}

impl Sample for u8 {
    fn to_f32(self) -> f32 {
        f32::from(self)
    }

    fn from_f32(value: f32) -> Self {
        value.round().clamp(0.0, 255.0) as u8
    }
}

impl Sample for u16 {
    fn to_f32(self) -> f32 {
        f32::from(self)
    }

    fn from_f32(value: f32) -> Self {
        value.round().clamp(0.0, 65535.0) as u16
    }
}

impl Sample for f32 {
    fn to_f32(self) -> f32 {
        self
    }

    fn from_f32(value: f32) -> Self {
        value
    }
}

/// Why a kernel refused its arguments.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum KernelError {
    /// An image has the wrong size.
    #[error("{what} is {found:?} pixels, and must be {expected:?}")]
    Size {
        /// Which image.
        what: &'static str,
        /// The size it must have, width by height.
        expected: (usize, usize),
        /// The size it has.
        found: (usize, usize),
    },
    /// A parameter is out of range.
    #[error("{0}")]
    Parameter(&'static str),
}

fn size<T>(view: &ImageView<'_, T>) -> (usize, usize) {
    (view.width(), view.height())
}

fn size_mut<T>(view: &ImageViewMut<'_, T>) -> (usize, usize) {
    (view.width(), view.height())
}

/// Checks that `what` is `expected` pixels.
fn check(
    what: &'static str,
    expected: (usize, usize),
    found: (usize, usize),
) -> Result<(), KernelError> {
    match found == expected {
        true => Ok(()),
        false => Err(KernelError::Size {
            what,
            expected,
            found,
        }),
    }
}
