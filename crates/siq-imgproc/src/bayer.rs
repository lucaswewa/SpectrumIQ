//! Colour filter arrays (REQ-PRV-001, ADR-0071): a Bayer mosaic's raw samples, shown in colour.
//!
//! - [`demosaic`]: at the mosaic's own size by bilinear interpolation, smaller from whole cells.
//! - [`histogram_cfa`], [`white_gains`], [`balanced`] and [`map_colours`]: each colour's
//!   statistics, the white balance, and the display mapping on each colour.
//! - [`luminance`]: what registration, focus and the flat take of a mosaic (P10.7).

use siq_types::{Cfa, ImageView, ImageViewMut, RectPx};

use crate::bands::for_bands;
use crate::display::{Colormap, DisplayMapping, Histogram, Lut, histogram};
use crate::{KernelError, check, size, size_mut};

/// The colour of each phase of `cfa`'s cell, by row and column.
fn phases(cfa: Cfa) -> [[usize; 2]; 2] {
    [0, 1].map(|y| [0, 1].map(|x| cfa.colour(x, y)))
}

/// `input`, a mosaic with `cfa` at its top left, in red, green and blue in `out`, which is no
/// larger:
///
/// - **at its size,** each pixel keeps its own colour, and takes for each other colour the mean
///   of its neighbours of that colour in the 3 × 3 about it, mirrored at the edges: bilinear
///   interpolation;
/// - **smaller,** each pixel is each colour's mean over the whole cells it covers.
pub fn demosaic(
    input: &ImageView<'_, u16>,
    cfa: Cfa,
    out: &mut ImageViewMut<'_, [u16; 3]>,
) -> Result<(), KernelError> {
    let ((width, height), (w, h)) = (size(input), size_mut(out));
    if (w, h) == (width, height) {
        interpolate(input, phases(cfa), out);
        Ok(())
    } else if w <= width && h <= height && width >= 2 && height >= 2 && w * h > 0 {
        cells(input, phases(cfa), out);
        Ok(())
    } else {
        Err(KernelError::Parameter(
            "a mosaic shows at most at its own size, and shrinks from 2 × 2 pixels",
        ))
    }
}

/// Pixel `i`'s neighbour `d` away, of `n`, mirrored at an edge: which keeps its colour.
fn mirror(i: usize, d: isize, n: usize) -> usize {
    let at = i as isize + d;
    let at = if at < 0 {
        -at
    } else if at >= n as isize {
        2 * n as isize - 2 - at
    } else {
        at
    };
    at.clamp(0, n as isize - 1) as usize
}

/// The luminance of `input`, a mosaic of any phase, in `out`, the same size: each pixel the 3 × 3
/// binomial mean about it, mirrored at the edges. Wherever it is centred, it weighs red, green and
/// blue 1 : 2 : 1, so the pattern is gone, and the detail is still a pixel's.
pub fn luminance(
    input: &ImageView<'_, u16>,
    out: &mut ImageViewMut<'_, u16>,
) -> Result<(), KernelError> {
    check("the output", size(input), size_mut(out))?;
    let (width, height) = size(input);
    for_bands(out.reborrow(), &|first, mut band| {
        for (y, row) in (first..).zip(band.rows_mut()) {
            let lines = [-1, 0, 1].map(|d| input.row(mirror(y, d, height)));
            for (x, value) in row.iter_mut().enumerate() {
                let [l, c, r] = [-1, 0, 1].map(|d| mirror(x, d, width));
                let across =
                    |line: &[u16]| u32::from(line[l]) + 2 * u32::from(line[c]) + u32::from(line[r]);
                let sum = across(lines[0]) + 2 * across(lines[1]) + across(lines[2]);
                *value = ((sum + 8) / 16) as u16;
            }
        }
    });
    Ok(())
}

fn interpolate(
    input: &ImageView<'_, u16>,
    phases: [[usize; 2]; 2],
    out: &mut ImageViewMut<'_, [u16; 3]>,
) {
    let (width, height) = size(input);
    for_bands(out.reborrow(), &|first, mut band| {
        for (y, row) in (first..).zip(band.rows_mut()) {
            let lines = [-1, 0, 1].map(|d| mirror(y, d, height));
            for (x, pixel) in row.iter_mut().enumerate() {
                let columns = [-1, 0, 1].map(|d| mirror(x, d, width));
                let (mut sums, mut counts) = ([0u32; 3], [0u32; 3]);
                for sy in lines {
                    let line = input.row(sy);
                    for sx in columns {
                        let colour = phases[sy & 1][sx & 1];
                        sums[colour] += u32::from(line[sx]);
                        counts[colour] += 1;
                    }
                }
                let (own, value) = (phases[y & 1][x & 1], input.row(y)[x]);
                *pixel = std::array::from_fn(|c| match (c == own, counts[c]) {
                    (false, n) if n > 0 => ((sums[c] + n / 2) / n) as u16,
                    _ => value,
                });
            }
        }
    });
}

fn cells(
    input: &ImageView<'_, u16>,
    phases: [[usize; 2]; 2],
    out: &mut ImageViewMut<'_, [u16; 3]>,
) {
    let (width, height) = size(input);
    let ((columns, rows), (w, h)) = ((width / 2, height / 2), size_mut(out));
    // The cells output pixel `i` of `n` covers, of `cells`: at least one.
    let span = |i: usize, n: usize, cells: usize| {
        let start = i * cells / n;
        start..((i + 1) * cells / n).max(start + 1)
    };
    for_bands(out.reborrow(), &|first, mut band| {
        for (oy, row) in (first..).zip(band.rows_mut()) {
            let ys = span(oy, h, rows);
            for (ox, pixel) in row.iter_mut().enumerate() {
                let xs = span(ox, w, columns);
                let mut sums = [0u64; 3];
                for y in 2 * ys.start..2 * ys.end {
                    let (mut even, mut odd) = (0u64, 0u64);
                    let pairs = input.row(y)[2 * xs.start..2 * xs.end].as_chunks::<2>().0;
                    for &[left, right] in pairs {
                        even += u64::from(left);
                        odd += u64::from(right);
                    }
                    sums[phases[y & 1][0]] += even;
                    sums[phases[y & 1][1]] += odd;
                }
                let n = (ys.len() * xs.len()) as u64;
                let counts = [n, 2 * n, n];
                *pixel = std::array::from_fn(|c| ((sums[c] + counts[c] / 2) / counts[c]) as u16);
            }
        }
    });
}

/// Counts every `step`-th cell of `input`, a mosaic with `cfa` at its top left, both ways, into
/// red's, green's and blue's histograms, after their counts so far.
pub fn histogram_cfa(
    input: &ImageView<'_, u16>,
    cfa: Cfa,
    step: usize,
    counts: &mut [Histogram; 3],
) -> Result<(), KernelError> {
    let (width, height) = size(input);
    for (x, y) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
        if width > x && height > y {
            let rect = RectPx::new(x as u32, y as u32, (width - x) as u32, (height - y) as u32);
            let phase = input.sub_view(rect).expect("a phase is inside its mosaic");
            histogram(&phase, 2 * step, &mut counts[cfa.colour(x, y)])?;
        }
    }
    Ok(())
}

/// The gains that show the brightest of a colour frame white, from red's, green's and blue's
/// histograms: each brings its colour's `fraction` point, from 0 to 1, to green's.
pub fn white_gains(counts: &[Histogram; 3], fraction: f64) -> [f32; 3] {
    let high = counts
        .each_ref()
        .map(|counts| f32::from(counts.percentile(fraction)).max(1.0));
    high.map(|colour| high[1] / colour)
}

/// `mapping` on each colour of a frame balanced by `gains`, red's, green's and blue's: a colour's
/// window is the window over its gain. A colour frame shows its own colours, so a colormap other
/// than inverted shows it as it is.
pub fn balanced(mapping: &DisplayMapping, gains: [f32; 3]) -> [DisplayMapping; 3] {
    let colormap = match mapping.colormap {
        Colormap::Inverted => Colormap::Inverted,
        _ => Colormap::Grey,
    };
    gains.map(|gain| {
        let gain = if gain.is_finite() && gain > 0.0 {
            gain
        } else {
            1.0
        };
        let at = |value: u16| (f32::from(value) / gain).round().clamp(0.0, 65535.0) as u16;
        let min = at(mapping.min).min(u16::MAX - 1);
        DisplayMapping {
            min,
            max: at(mapping.max).max(min + 1),
            gamma: mapping.gamma,
            colormap,
        }
    })
}

/// Maps `input`'s red, green and blue each through its own table of `luts` into `out`, the same
/// size.
pub fn map_colours(
    luts: &[Lut; 3],
    input: &ImageView<'_, [u16; 3]>,
    out: &mut ImageViewMut<'_, [u8; 3]>,
) -> Result<(), KernelError> {
    check("the output", size(input), size_mut(out))?;
    for_bands(out.reborrow(), &|first, mut band| {
        for (y, row) in (first..).zip(band.rows_mut()) {
            for (shown, value) in row.iter_mut().zip(input.row(y)) {
                *shown = std::array::from_fn(|c| luts[c].level(value[c]));
            }
        }
    });
    Ok(())
}
