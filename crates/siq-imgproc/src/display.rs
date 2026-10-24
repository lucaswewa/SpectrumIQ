//! Statistics and display mapping (REQ-PRV-003): a 4096-bin histogram on a strided subsample,
//! percentiles and auto-contrast, and a 64 Ki lookup table from `u16` to 8-bit grey or colour.

use std::sync::atomic::{AtomicU32, Ordering};

use siq_types::{ImageView, ImageViewMut};

use crate::bands::{band_rows, for_bands, reduce};
use crate::{KernelError, check, size, size_mut};

/// The number of bins of a [`Histogram`].
pub const BINS: usize = 4096;

/// A histogram of 4096 bins over a pixel format's range: a 14-bit value `v` counts in bin
/// `v >> 2`, and values past the range in the last bin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Histogram {
    bins: [u32; BINS],
    shift: u32,
    count: u64,
}

impl Histogram {
    /// An empty histogram for `bits`-bit values. Under 12 bits, a bin is one value.
    pub fn new(bits: u32) -> Self {
        Self {
            bins: [0; BINS],
            shift: bits.saturating_sub(12),
            count: 0,
        }
    }

    /// The counts, by bin.
    pub fn bins(&self) -> &[u32; BINS] {
        &self.bins
    }

    /// The number of pixels counted.
    pub fn count(&self) -> u64 {
        self.count
    }

    /// The bin a value counts in.
    pub fn bin_of(&self, value: u16) -> usize {
        (usize::from(value) >> self.shift).min(BINS - 1)
    }

    /// The lowest value of the bin that holds the `fraction` point of the pixels, from 0 to 1.
    pub fn percentile(&self, fraction: f64) -> u16 {
        let target = (fraction.clamp(0.0, 1.0) * self.count as f64)
            .ceil()
            .max(1.0) as u64;
        let mut seen = 0;
        let bin = self.bins.iter().position(|&n| {
            seen += u64::from(n);
            seen >= target
        });
        (bin.unwrap_or(BINS - 1) << self.shift).min(usize::from(u16::MAX)) as u16
    }

    /// The window that auto-contrast shows: from the `low` point's bin to the end of the `high`
    /// point's bin, both fractions from 0 to 1. The window is never empty.
    pub fn auto_window(&self, low: f64, high: f64) -> (u16, u16) {
        let min = self.percentile(low);
        let top = usize::from(self.percentile(high)) + (1 << self.shift) - 1;
        (
            min,
            (top.min(usize::from(u16::MAX)) as u16).max(min.saturating_add(1)),
        )
    }
}

/// Counts every `step`-th pixel of every `step`-th row of `input` into `histogram`, after its
/// counts so far.
pub fn histogram(
    input: &ImageView<'_, u16>,
    step: usize,
    histogram: &mut Histogram,
) -> Result<(), KernelError> {
    if step == 0 {
        return Err(KernelError::Parameter(
            "a histogram's step must be at least 1",
        ));
    }
    let shared = [const { AtomicU32::new(0) }; BINS];
    let rows = input.height().div_ceil(step);
    let per_band = band_rows(input.width().div_ceil(step));
    let shift = histogram.shift;
    reduce(
        0..rows,
        per_band,
        &|band| {
            let mut local = [0u32; BINS];
            for y in band {
                for &value in input.row(y * step).iter().step_by(step) {
                    local[(usize::from(value) >> shift).min(BINS - 1)] += 1;
                }
            }
            for (total, &n) in shared.iter().zip(&local).filter(|(_, n)| **n > 0) {
                total.fetch_add(n, Ordering::Relaxed);
            }
        },
        &|(), ()| (),
    );
    for (bin, total) in histogram.bins.iter_mut().zip(&shared) {
        let n = total.load(Ordering::Relaxed);
        *bin += n;
        histogram.count += u64::from(n);
    }
    Ok(())
}

/// How display levels become colours.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Colormap {
    /// Black to white.
    Grey,
    /// White to black.
    Inverted,
    /// Black to this colour, as for a fluorescence channel.
    Colour([u8; 3]),
}

/// A display mapping: the window from `min` to `max`, a gamma, and a colormap.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DisplayMapping {
    /// The value shown black, and below.
    pub min: u16,
    /// The value shown full, and above.
    pub max: u16,
    /// The gamma: a level `t` of the window shows as `t^(1/gamma)`, so above 1 brightens the
    /// shadows.
    pub gamma: f32,
    /// The colours.
    pub colormap: Colormap,
}

/// A [`DisplayMapping`] as tables: 64 Ki levels, one per `u16` value, and 256 colours.
#[derive(Debug, Clone)]
pub struct Lut {
    levels: Box<[u8; 1 << 16]>,
    colours: [[u8; 3]; 256],
}

impl Lut {
    /// The tables of `mapping`. They are allocated once, here.
    pub fn new(mapping: &DisplayMapping) -> Result<Self, KernelError> {
        if mapping.min >= mapping.max || !(mapping.gamma.is_finite() && mapping.gamma > 0.0) {
            return Err(KernelError::Parameter(
                "a display window must have min < max and a positive gamma",
            ));
        }
        let (min, span) = (f32::from(mapping.min), f32::from(mapping.max - mapping.min));
        let mut levels: Box<[u8; 1 << 16]> = vec![0; 1 << 16]
            .into_boxed_slice()
            .try_into()
            .expect("64 Ki levels");
        for (value, level) in (0..=u16::MAX).zip(levels.iter_mut()) {
            let t = ((f32::from(value) - min) / span).clamp(0.0, 1.0);
            let shown = (t.powf(1.0 / mapping.gamma) * 255.0).round() as u8;
            *level = match mapping.colormap {
                Colormap::Inverted => 255 - shown,
                _ => shown,
            };
        }
        let colours = std::array::from_fn(|level| match mapping.colormap {
            Colormap::Colour(colour) => {
                colour.map(|c| ((u32::from(c) * level as u32 + 127) / 255) as u8)
            }
            _ => [level as u8; 3],
        });
        Ok(Self { levels, colours })
    }

    /// The level `value` shows at: grey, or inverted grey.
    pub fn level(&self, value: u16) -> u8 {
        self.levels[usize::from(value)]
    }

    /// The colour `value` shows as.
    pub fn colour(&self, value: u16) -> [u8; 3] {
        self.colours[usize::from(self.level(value))]
    }

    /// Maps `input` to levels in `out`, the same size. A colour colormap's levels are its
    /// brightness; [`apply_rgb`](Self::apply_rgb) gives its colours.
    pub fn apply(
        &self,
        input: &ImageView<'_, u16>,
        out: &mut ImageViewMut<'_, u8>,
    ) -> Result<(), KernelError> {
        check("the output", size(input), size_mut(out))?;
        for_bands(out.reborrow(), &|first, mut band| {
            for (y, row) in (first..).zip(band.rows_mut()) {
                row.iter_mut()
                    .zip(input.row(y))
                    .for_each(|(o, &v)| *o = self.level(v));
            }
        });
        Ok(())
    }

    /// Maps `input` to colours in `out`, the same size.
    pub fn apply_rgb(
        &self,
        input: &ImageView<'_, u16>,
        out: &mut ImageViewMut<'_, [u8; 3]>,
    ) -> Result<(), KernelError> {
        check("the output", size(input), size_mut(out))?;
        for_bands(out.reborrow(), &|first, mut band| {
            for (y, row) in (first..).zip(band.rows_mut()) {
                row.iter_mut()
                    .zip(input.row(y))
                    .for_each(|(o, &v)| *o = self.colour(v));
            }
        });
        Ok(())
    }
}
