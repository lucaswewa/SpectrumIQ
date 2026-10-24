//! Focus measures, for autofocus and the live statistics: larger is sharper.

use std::ops::Range;

use siq_types::ImageView;

use crate::Sample;
use crate::bands::{band_rows, reduce};

/// Which focus measure [`focus`] takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusMeasure {
    /// The variance of the 4-neighbour Laplacian, over the interior pixels.
    LaplacianVariance,
    /// The mean squared Sobel gradient, over the interior pixels.
    Tenengrad,
    /// The variance of the intensities over their mean: independent of the illumination's level.
    NormalizedVariance,
}

/// `measure` of `input`, usually a ROI from [`sub_view`](ImageView::sub_view). An image with no
/// interior, under 3 × 3 pixels, measures 0 by the Laplacian and Tenengrad, and a black one 0 by
/// the normalized variance.
pub fn focus<T: Sample>(input: &ImageView<'_, T>, measure: FocusMeasure) -> f64 {
    let (width, height) = (input.width(), input.height());
    let interior = width >= 3 && height >= 3;
    // Each row gives the count, the sum and the sum of squares of its values, in `f64`.
    let sums = |rows: Range<usize>, per_row: &(dyn Fn(usize) -> Sums + Sync)| {
        let band = |band: Range<usize>| band.map(per_row).fold(Sums::default(), Sums::add);
        reduce(rows, band_rows(width), &band, &Sums::add)
    };
    let neighbours = |y: usize| [input.row(y - 1), input.row(y), input.row(y + 1)];
    let value = |v: &T| f64::from(v.to_f32());
    match measure {
        FocusMeasure::LaplacianVariance if interior => {
            let Sums(n, sum, squares) = sums(1..height - 1, &|y| {
                let [above, here, below] = neighbours(y);
                let pixels = here.windows(3).zip(&above[1..]).zip(&below[1..]);
                Sums::of(pixels.map(|((h, a), b)| {
                    value(a) + value(b) + value(&h[0]) + value(&h[2]) - 4.0 * value(&h[1])
                }))
            });
            squares / n - (sum / n).powi(2)
        }
        FocusMeasure::Tenengrad if interior => {
            let Sums(n, sum, _) = sums(1..height - 1, &|y| {
                let [above, here, below] = neighbours(y);
                let windows = above.windows(3).zip(here.windows(3)).zip(below.windows(3));
                Sums::of(windows.map(|((a, h), b)| {
                    let column = |x: usize| value(&a[x]) + 2.0 * value(&h[x]) + value(&b[x]);
                    let row = |r: &[T]| value(&r[0]) + 2.0 * value(&r[1]) + value(&r[2]);
                    let (gx, gy) = (column(2) - column(0), row(b) - row(a));
                    gx * gx + gy * gy
                }))
            });
            sum / n
        }
        FocusMeasure::NormalizedVariance if width > 0 && height > 0 => {
            // Deviations from a pixel keep the variance exact when it is small beside the mean.
            let reference = value(&input.row(0)[0]);
            let Sums(n, sum, squares) = sums(0..height, &|y| {
                Sums::of(input.row(y).iter().map(|v| value(v) - reference))
            });
            let (shift, mean) = (sum / n, sum / n + reference);
            match mean > 0.0 {
                true => (squares / n - shift * shift) / mean,
                false => 0.0,
            }
        }
        _ => 0.0,
    }
}

/// A count, a sum and a sum of squares.
#[derive(Debug, Clone, Copy, Default)]
struct Sums(f64, f64, f64);

impl Sums {
    fn of(values: impl Iterator<Item = f64>) -> Self {
        values.fold(Self::default(), |Self(n, s, q), v| {
            Self(n + 1.0, s + v, q + v * v)
        })
    }

    fn add(self, other: Self) -> Self {
        Self(self.0 + other.0, self.1 + other.1, self.2 + other.2)
    }
}
