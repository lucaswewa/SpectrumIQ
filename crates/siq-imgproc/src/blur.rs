//! A separable Gaussian blur, its kernel cut at 3σ, with the edge pixels held beyond the image.

use siq_types::{ImageView, ImageViewMut};

use crate::bands::for_bands;
use crate::{KernelError, check, size, size_mut};

/// The widest blur, in pixels: its kernel, 3σ each side, holds at most 512 taps a side. To blur
/// more, blur a downsampled image.
pub const MAX_SIGMA: f32 = 170.0;

/// Blurs `input` into `out` with a Gaussian of `sigma` pixels, through `scratch`. All three are
/// the same size; a σ of 0 copies.
pub fn gaussian_blur(
    input: &ImageView<'_, f32>,
    sigma: f32,
    scratch: &mut ImageViewMut<'_, f32>,
    out: &mut ImageViewMut<'_, f32>,
) -> Result<(), KernelError> {
    check("the scratch image", size(input), size_mut(scratch))?;
    check("the output", size(input), size_mut(out))?;
    if !(0.0..=MAX_SIGMA).contains(&sigma) {
        return Err(KernelError::Parameter(
            "a blur's sigma must be from 0 to MAX_SIGMA",
        ));
    }
    let mut taps = [0.0f32; 513];
    let radius = (3.0 * sigma).ceil() as usize;
    let weights = &mut taps[..=radius];
    for (k, weight) in weights.iter_mut().enumerate() {
        *weight = match sigma > 0.0 {
            true => (-((k * k) as f32) / (2.0 * sigma * sigma)).exp(),
            false => 1.0,
        };
    }
    let total = weights[0] + 2.0 * weights[1..].iter().sum::<f32>();
    weights.iter_mut().for_each(|weight| *weight /= total);
    let weights = &*weights;

    // Along the rows, into the scratch image.
    for_bands(scratch.reborrow(), &|first, mut band| {
        for (y, row) in (first..).zip(band.rows_mut()) {
            convolve_row(input.row(y), weights, row);
        }
    });
    // Down the columns, a whole row at a time.
    let (blurred, height) = (scratch.as_view(), input.height());
    for_bands(out.reborrow(), &|first, mut band| {
        for (y, row) in (first..).zip(band.rows_mut()) {
            axpy(row, blurred.row(y), weights[0], None);
            for (k, &weight) in (1..).zip(&weights[1..]) {
                let (above, below) = (
                    blurred.row(y.saturating_sub(k)),
                    blurred.row((y + k).min(height - 1)),
                );
                axpy(row, above, weight, Some(below));
            }
        }
    });
    Ok(())
}

/// `out = weight × a` when `b` is `None`, and `out += weight × (a + b)` otherwise.
fn axpy(out: &mut [f32], a: &[f32], weight: f32, b: Option<&[f32]>) {
    match b {
        None => out.iter_mut().zip(a).for_each(|(o, &a)| *o = weight * a),
        Some(b) => {
            for ((o, &a), &b) in out.iter_mut().zip(a).zip(b) {
                *o += weight * (a + b);
            }
        }
    }
}

/// One row through the kernel `weights` (from the centre out), the edge pixels held beyond it.
fn convolve_row(input: &[f32], weights: &[f32], out: &mut [f32]) {
    let width = input.len();
    axpy(out, input, weights[0], None);
    for (k, &weight) in (1..).zip(&weights[1..]) {
        // Within the row, both taps are in it, and the slices line up for the compiler.
        let inner = width.saturating_sub(2 * k);
        let (left, right) = (&input[..inner], &input[(2 * k).min(width)..]);
        axpy(
            &mut out[k.min(width)..k.min(width) + inner],
            left,
            weight,
            Some(right),
        );
        // Near the edges, a tap past the end takes the edge pixel.
        let edges = (0..k.min(width)).chain((k + inner).max(k.min(width))..width);
        for x in edges {
            out[x] += weight * (input[x.saturating_sub(k)] + input[(x + k).min(width - 1)]);
        }
    }
}
