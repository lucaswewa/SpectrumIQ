//! Multiscale pyramids (ADR-0044): an OME-Zarr image's levels 1 and up, built from level 0 by
//! 2 × 2 mean, out of core.
//!
//! - **The filter** is the mean of each 2 × 2 block, of what is inside the image at the edges:
//!   rounded to the nearest for `u16`, exact for `f32`. A level halves the one before it in `y` and
//!   `x`, rounding up.
//! - **The levels** go on until the image fits [`TOP`] pixels both ways.
//! - **Chunk alignment:** a chunk of level `l + 1` is the mean of exactly four chunks of level `l`,
//!   the 2 × 2 block below it. Each is built alone, in parallel, read once and written once, with no
//!   read-modify-write. A chunk row is done at a time, so the memory held is a few chunks per
//!   thread, whatever the image's size.
//!
//! The work runs on the calling rayon pool: a job's, through `pool.install`.
//!
//! [`tiles`] renders deep-zoom tiles from the levels, on demand (ADR-0045).

pub mod tiles;

use std::sync::atomic::{AtomicU64, Ordering};

use rayon::prelude::*;
use siq_storage::StorageError;
use siq_storage::zarr::{ImageInfo, SampleType, ZarrImage, ZarrSample};
use siq_types::{ImageView, RectPx};

/// The largest size, in pixels, of the top level both ways: about a deep-zoom tile.
pub const TOP: u64 = 256;

/// Where a build reports its progress, and learns whether to stop.
pub trait Control: Sync {
    /// The share done, from 0 to 1.
    fn progress(&self, done: f64);
    /// Whether to stop.
    fn cancelled(&self) -> bool;
}

/// Nobody to report to, and no stopping.
impl Control for () {
    fn progress(&self, _: f64) {}

    fn cancelled(&self) -> bool {
        false
    }
}

/// Why a build stopped.
#[derive(Debug, thiserror::Error)]
pub enum PyramidError {
    /// It was asked to stop. The levels may be part-built.
    #[error("cancelled")]
    Cancelled,
    /// The image couldn't be read or written.
    #[error(transparent)]
    Storage(#[from] StorageError),
}

/// The levels a pyramid of `width` × `height` has: level 0, and each halving until both fit
/// `top`.
pub fn levels_for(width: u64, height: u64, top: u64) -> usize {
    let (mut side, mut levels) = (width.max(height), 1);
    while side > top.max(1) {
        side = side.div_ceil(2);
        levels += 1;
    }
    levels
}

/// The RAM a build of `info` takes at its peak with `threads` working: each holds the 2 × 2
/// chunks it reads and the one it writes, and as much again for the codec's buffers.
pub fn estimate(info: &ImageInfo, threads: usize) -> u64 {
    let sample = match info.sample {
        SampleType::U16 => 2,
        SampleType::F32 => 4,
    };
    let chunk = info.chunk * info.chunk * sample;
    threads.max(1) as u64 * (4 + 1) * chunk * 2
}

/// Builds `image`'s levels 1 and up from its level 0, adding the levels that `top` needs and
/// listing them in its metadata. Level 0 must be written.
pub fn build(image: &mut ZarrImage, top: u64, control: &dyn Control) -> Result<(), PyramidError> {
    let size = image.info().size;
    image.set_levels(levels_for(size.x, size.y, top))?;
    let image = &*image;
    let info = image.info();
    let planes: Vec<(u64, u64, u64)> = (0..size.t)
        .flat_map(|t| (0..size.c).flat_map(move |c| (0..size.z).map(move |z| (t, c, z))))
        .collect();
    let chunks = |level| {
        let (rows, cols) = info.chunks(level);
        rows * cols
    };
    let total: u64 = (1..info.levels).map(chunks).sum::<u64>() * planes.len() as u64;
    let done = AtomicU64::new(0);
    for level in 1..info.levels {
        let (rows, cols) = info.chunks(level);
        for row in 0..rows {
            let tasks = planes
                .par_iter()
                .flat_map(|&plane| (0..cols).into_par_iter().map(move |col| (plane, col)));
            tasks.try_for_each(|(plane, col)| {
                if control.cancelled() {
                    return Err(PyramidError::Cancelled);
                }
                match info.sample {
                    SampleType::U16 => halve::<u16>(image, level, plane, (row, col))?,
                    SampleType::F32 => halve::<f32>(image, level, plane, (row, col))?,
                }
                let finished = done.fetch_add(1, Ordering::Relaxed) + 1;
                control.progress(finished as f64 / total as f64);
                Ok(())
            })?;
        }
    }
    control.progress(1.0);
    Ok(())
}

/// Builds chunk (`row`, `col`) of `plane` at `level` from the 2 × 2 chunks of the level below.
fn halve<T: Mean>(
    image: &ZarrImage,
    level: usize,
    plane: (u64, u64, u64),
    (row, col): (u64, u64),
) -> Result<(), StorageError> {
    let info = image.info();
    let (chunk, below) = (info.chunk, info.size(level - 1));
    let (x0, y0) = (2 * col * chunk, 2 * row * chunk);
    let (width, height) = ((2 * chunk).min(below.x - x0), (2 * chunk).min(below.y - y0));
    let rect = RectPx::new(x0 as u32, y0 as u32, width as u32, height as u32);
    let source = image.read_region::<T>(level - 1, plane, rect)?;
    let (pixels, width, height) = mean_2x2(source.view());
    let view = ImageView::new(&pixels, width, height, width).expect("a chunk of its own size");
    image.write_chunk(level, plane, (row, col), view)
}

/// `source` halved: each pixel the mean of the 2 × 2 block of `source` it covers, or of what is
/// left of the block at the right and bottom edges. Its pixels, width and height.
pub fn mean_2x2<T: Mean>(source: ImageView<'_, T>) -> (Vec<T>, usize, usize) {
    let (width, height) = (source.width().div_ceil(2), source.height().div_ceil(2));
    let mut pixels = Vec::with_capacity(width * height);
    let mut block = Vec::with_capacity(4);
    for y in 0..height {
        let rows = [
            Some(source.row(2 * y)),
            (2 * y + 1 < source.height()).then(|| source.row(2 * y + 1)),
        ];
        for x in 0..width {
            block.clear();
            for row in rows.into_iter().flatten() {
                block.extend(
                    row.get(2 * x..(2 * x + 2).min(row.len()))
                        .unwrap_or_default(),
                );
            }
            pixels.push(T::mean(&block));
        }
    }
    (pixels, width, height)
}

/// A pixel type that has a mean.
pub trait Mean: ZarrSample {
    /// The mean of 1 to 4 pixels.
    fn mean(pixels: &[Self]) -> Self;
}

impl Mean for u16 {
    /// Rounded to the nearest, halves up.
    fn mean(pixels: &[Self]) -> Self {
        let n = pixels.len() as u32;
        let sum: u32 = pixels.iter().map(|&p| u32::from(p)).sum();
        ((sum + n / 2) / n.max(1)) as u16
    }
}

impl Mean for f32 {
    fn mean(pixels: &[Self]) -> Self {
        pixels.iter().sum::<f32>() / pixels.len().max(1) as f32
    }
}
