//! Resampling: area averaging to any smaller size, binning, copying, and bilinear enlarging.

use siq_types::{Binning, ImageView, ImageViewMut};

use crate::bands::for_bands;
use crate::{KernelError, Sample, check, size, size_mut};

/// Output columns are planned, and summed, in chunks of this many on the stack.
const CHUNK: usize = 1024;

/// Shrinks `input` to `out`'s size, each output pixel the mean of the input area it covers, with
/// the pixels on its edges weighted by the share they cover. Neither dimension may grow.
pub fn downsample_area<T: Sample>(
    input: &ImageView<'_, T>,
    out: &mut ImageViewMut<'_, T>,
) -> Result<(), KernelError> {
    let ((width, height), (columns, rows)) = (size(input), size_mut(out));
    if columns > width || rows > height {
        return Err(KernelError::Parameter(
            "an area average can't enlarge an image",
        ));
    }
    if columns == 0 || rows == 0 {
        return Ok(());
    }
    let (sx, sy) = (width as f64 / columns as f64, height as f64 / rows as f64);
    let scale = (1.0 / (sx * sy)) as f32;
    for_bands(out.reborrow(), &|first, mut band| {
        let (mut plan, mut sums) = ([Span::default(); CHUNK], [0.0f32; CHUNK]);
        for start in (0..columns).step_by(CHUNK) {
            let n = CHUNK.min(columns - start);
            let (plan, sums) = (&mut plan[..n], &mut sums[..n]);
            for (column, span) in (start..).zip(plan.iter_mut()) {
                *span = Span::new(column, sx, width);
            }
            for (oy, row) in (first..).zip(band.rows_mut()) {
                let lines = Span::new(oy, sy, height);
                sums.fill(0.0);
                for y in lines.start..lines.end {
                    let (weight, line) = (lines.weight(y), input.row(y));
                    for (sum, span) in sums.iter_mut().zip(plan.iter()) {
                        *sum += weight * span.sum(line);
                    }
                }
                for (value, &sum) in row[start..start + n].iter_mut().zip(sums.iter()) {
                    *value = T::from_f32(sum * scale);
                }
            }
        }
    });
    Ok(())
}

/// The input pixels an output pixel covers along one axis, and the shares of the first and last.
#[derive(Debug, Clone, Copy, Default)]
struct Span {
    start: usize,
    end: usize,
    first: f32,
    last: f32,
}

impl Span {
    /// The span of output pixel `o`, each `scale` input pixels wide, of `len`.
    fn new(o: usize, scale: f64, len: usize) -> Self {
        let (from, to) = (o as f64 * scale, ((o + 1) as f64 * scale).min(len as f64));
        let start = from as usize;
        let end = (to.ceil() as usize).clamp(start + 1, len);
        let share = |i: usize| (to.min(i as f64 + 1.0) - from.max(i as f64)) as f32;
        Self {
            start,
            end,
            first: share(start),
            last: share(end - 1),
        }
    }

    /// The share of pixel `i` of the span.
    fn weight(&self, i: usize) -> f32 {
        match i {
            _ if i == self.start => self.first,
            _ if i + 1 == self.end => self.last,
            _ => 1.0,
        }
    }

    /// The sum of `line` over the span, the edge pixels weighted by their share.
    fn sum<T: Sample>(&self, line: &[T]) -> f32 {
        let values = &line[self.start..self.end];
        let (first, last) = (values[0].to_f32(), values[values.len() - 1].to_f32());
        if values.len() == 1 {
            return self.first * first;
        }
        let whole: f32 = values.iter().map(|v| v.to_f32()).sum();
        whole + (self.first - 1.0) * first + (self.last - 1.0) * last
    }
}

/// How [`bin`] combines a block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinMode {
    /// The sum, saturating at `u16::MAX`.
    Sum,
    /// The mean, rounded.
    Mean,
}

/// Combines each `binning` block of `input` into one pixel of `out`, which must be
/// `binning.image_size` of the input: blocks cut by the right or bottom edge are dropped.
pub fn bin(
    input: &ImageView<'_, u16>,
    binning: Binning,
    mode: BinMode,
    out: &mut ImageViewMut<'_, u16>,
) -> Result<(), KernelError> {
    let (width, height) = size(input);
    let expected = binning.image_size(width as u32, height as u32);
    check(
        "the output",
        (expected.0 as usize, expected.1 as usize),
        size_mut(out),
    )?;
    let (bx, by) = binning.factors();
    let (bx, by) = (usize::from(bx), usize::from(by));
    let (area, columns) = ((bx * by) as u32, size_mut(out).0);
    for_bands(out.reborrow(), &|first, mut band| {
        let mut sums = [0u32; CHUNK];
        for (oy, row) in (first..).zip(band.rows_mut()) {
            for start in (0..columns).step_by(CHUNK) {
                let n = CHUNK.min(columns - start);
                let sums = &mut sums[..n];
                sums.fill(0);
                for y in oy * by..(oy + 1) * by {
                    let line = &input.row(y)[start * bx..(start + n) * bx];
                    for (sum, block) in sums.iter_mut().zip(line.chunks_exact(bx)) {
                        *sum += block.iter().map(|&v| u32::from(v)).sum::<u32>();
                    }
                }
                for (value, &sum) in row[start..start + n].iter_mut().zip(sums.iter()) {
                    *value = match mode {
                        BinMode::Sum => sum.min(u32::from(u16::MAX)) as u16,
                        BinMode::Mean => ((sum + area / 2) / area) as u16,
                    };
                }
            }
        }
    });
    Ok(())
}

/// Copies `input` into `out`, which must be the same size. A crop is a copy of a
/// [`sub_view`](ImageView::sub_view).
pub fn copy<T: Copy + Send + Sync>(
    input: &ImageView<'_, T>,
    out: &mut ImageViewMut<'_, T>,
) -> Result<(), KernelError> {
    check("the output", size(input), size_mut(out))?;
    for_bands(out.reborrow(), &|first, mut band| {
        for (y, row) in (first..).zip(band.rows_mut()) {
            row.copy_from_slice(input.row(y));
        }
    });
    Ok(())
}

/// Resizes `input` to `out`'s size by bilinear interpolation between pixel centres, holding the
/// edge pixels beyond them. Meant for enlarging: to shrink, [`downsample_area`] doesn't alias.
pub fn upsample_bilinear<T: Sample>(
    input: &ImageView<'_, T>,
    out: &mut ImageViewMut<'_, T>,
) -> Result<(), KernelError> {
    let ((width, height), (columns, rows)) = (size(input), size_mut(out));
    if width == 0 || height == 0 {
        return match columns == 0 || rows == 0 {
            true => Ok(()),
            false => Err(KernelError::Parameter("an empty image can't be enlarged")),
        };
    }
    let (sx, sy) = (width as f32 / columns as f32, height as f32 / rows as f32);
    // The input position of an output pixel's centre, and its two neighbours with the weight
    // of the second.
    let source = |o: usize, scale: f32, len: usize| {
        let at = ((o as f32 + 0.5) * scale - 0.5).clamp(0.0, (len - 1) as f32);
        let low = at as usize;
        (low, (low + 1).min(len - 1), at - low as f32)
    };
    for_bands(out.reborrow(), &|first, mut band| {
        for (oy, row) in (first..).zip(band.rows_mut()) {
            let (y0, y1, fy) = source(oy, sy, height);
            let (top, bottom) = (input.row(y0), input.row(y1));
            for (ox, value) in row.iter_mut().enumerate() {
                let (x0, x1, fx) = source(ox, sx, width);
                let lerp = |a: T, b: T| a.to_f32() + (b.to_f32() - a.to_f32()) * fx;
                let (upper, lower) = (lerp(top[x0], top[x1]), lerp(bottom[x0], bottom[x1]));
                *value = T::from_f32(upper + (lower - upper) * fy);
            }
        }
    });
    Ok(())
}
