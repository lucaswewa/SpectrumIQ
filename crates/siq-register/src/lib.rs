//! Image registration (ADR-0048): how far one image's content is from another's, to a fraction of
//! a pixel, and how sure that is, by phase correlation. The workhorse of calibration (M5) and of
//! stitching (M8).
//!
//! - **Phase correlation:** each image, less its mean and tapered, is transformed. The cross-power
//!   spectrum, normalised to unit magnitude, keeps only the phase difference; low-passed, so that
//!   the peak is a Gaussian of a pixel and the noisiest frequencies weigh little, and transformed
//!   back, it peaks at the shift.
//! - **The search** tapers the whole images with [`Options::window`], a Tukey taper by default,
//!   which keeps their edges, where low overlaps lie. An image with a side over
//!   [`Options::max_size`] is searched shrunk.
//! - **Candidates:** the highest peaks. A peak's shift is known only modulo the image's size, so
//!   each has four hypotheses, `dx` or `dx − width` and `dy` or `dy − height`, and the images'
//!   normalised cross-correlation over each hypothesis's overlap chooses. Among the peaks, the
//!   highest peak times its correlation wins: a tiny overlap correlating by chance doesn't.
//! - **The refinement** correlates windows of the overlap that show the same content, at full
//!   resolution, tapered by Hann: the tapers no longer pull the peak toward zero. It repeats until
//!   the residual is under half a pixel.
//! - **Sub-pixel:** a Gaussian through the peak and its neighbours on each axis, exact for the
//!   low-passed peak; or a parabola, or Foroosh's estimate ([`Subpixel`]).
//! - **Confidence** ([`Registration`]): the peak-to-correlation energy of the search, and the
//!   refinement's margin of its peak over the next and the overlap's correlation.
//! - **FFT plans** are kept per size, and the refinement's windows have sides with no prime factor
//!   but 2, 3 and 5, so a [`Registrar`] used for one geometry plans a few times, then reuses.

mod fft;

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use realfft::num_complex::Complex;
use siq_types::ImageView;

use fft::Plans;

/// The smallest side registered, in pixels.
pub const MIN_SIZE: usize = 16;
/// The peaks closer than this to a higher one, in pixels, are part of it.
const PEAK_RADIUS: i64 = 2;
/// The most refinements at full resolution.
const REFINEMENTS: usize = 3;

/// How the images are tapered at their edges, so that the jump there doesn't dominate the
/// spectrum.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Window {
    /// Hann: a raised cosine over the whole image.
    Hann,
    /// Tukey: flat, with cosine edges over this share of the image, from 0 (flat) to 1 (Hann).
    Tukey(f64),
}

/// How the peak is placed between pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Subpixel {
    /// A Gaussian through the peak and its two neighbours, on each axis: exact for the peak that
    /// [`Options::smoothing`] makes.
    Gaussian,
    /// A parabola through the peak and its two neighbours, on each axis.
    Parabolic,
    /// Foroosh, Zerubia and Berthod's estimate, from the peak and its higher neighbour, which
    /// assumes the unwindowed peak's shape.
    Foroosh,
}

/// How to register.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Options {
    /// The search's taper. The refinement's is Hann.
    pub window: Window,
    /// The sub-pixel estimate.
    pub subpixel: Subpixel,
    /// The correlation peak's width, as a Gaussian's standard deviation in pixels, to which the
    /// cross-power spectrum is low-passed; 0 leaves the sharp peak, for [`Subpixel::Foroosh`].
    pub smoothing: f64,
    /// The peaks whose hypotheses are weighed.
    pub candidates: usize,
    /// The longest side correlated at once: larger images are registered shrunk, then refined.
    pub max_size: usize,
    /// The smallest overlap a hypothesis may have, as a share of the image's area.
    pub min_overlap: f64,
}

impl Default for Options {
    /// A Tukey taper over a fifth of the image, which keeps the edges where low overlaps are, and a
    /// Gaussian peak of a pixel; four candidates, 512 pixels, and an overlap of at least 2 %.
    fn default() -> Self {
        Self {
            window: Window::Tukey(0.2),
            subpixel: Subpixel::Gaussian,
            smoothing: 1.0,
            candidates: 8,
            max_size: 512,
            min_overlap: 0.02,
        }
    }
}

/// What a registration found.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Registration {
    /// How far `b`'s content is from `a`'s, in pixels, `x` then `y`: `b` shows at `(x, y)` what
    /// `a` shows at `(x − dx, y − dy)`.
    pub shift: [f64; 2],
    /// The search's peak-to-correlation energy: the peak's square over the mean square of the rest
    /// of the surface. Thousands or more for a clear match; tens for unrelated images.
    pub pce: f64,
    /// How far the refinement's peak stands above its next one, from 0 (as high: a periodic
    /// pattern) to 1.
    pub margin: f64,
    /// The images' normalised cross-correlation over their overlap at the whole-pixel shift, from
    /// −1 to 1. Blobby content can correlate by chance: weigh it with the energy.
    pub ncc: f64,
}

/// Why two images couldn't be registered.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum RegisterError {
    /// They aren't the same size.
    #[error("the images are {a:?} and {b:?}: they must be the same size")]
    Sizes {
        /// The first's width and height.
        a: (usize, usize),
        /// The second's.
        b: (usize, usize),
    },
    /// A side is under [`MIN_SIZE`].
    #[error("the images are {0:?}: each side needs {MIN_SIZE} pixels")]
    TooSmall((usize, usize)),
    /// An image, or every overlap, is flat: there is nothing to register.
    #[error("an image is flat: there is nothing to register")]
    Flat,
}

/// Registers pairs of images, keeping the FFT plans of each size it has seen.
pub struct Registrar {
    options: Options,
    plans: Mutex<HashMap<(usize, usize), Arc<Plans>>>,
}

impl Registrar {
    /// A registrar with `options`.
    pub fn new(options: Options) -> Self {
        Self {
            options,
            plans: Mutex::new(HashMap::new()),
        }
    }

    /// Its options.
    pub fn options(&self) -> &Options {
        &self.options
    }

    /// The sizes it has plans for.
    pub fn planned(&self) -> usize {
        self.plans
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }

    /// How far `b`'s content is from `a`'s, which must be the same size.
    pub fn register<T: Copy + Into<f32>>(
        &self,
        a: ImageView<'_, T>,
        b: ImageView<'_, T>,
    ) -> Result<Registration, RegisterError> {
        let size = (a.width(), a.height());
        if (b.width(), b.height()) != size {
            return Err(RegisterError::Sizes {
                a: size,
                b: (b.width(), b.height()),
            });
        }
        if size.0 < MIN_SIZE || size.1 < MIN_SIZE {
            return Err(RegisterError::TooSmall(size));
        }
        let (a, b) = (Plane::of(a), Plane::of(b));
        let factor = size
            .0
            .max(size.1)
            .div_ceil(self.options.max_size.max(MIN_SIZE));
        // Every hypothesis, shrunk if the images are large.
        let first = if factor <= 1 {
            self.correlate(&a, &b, false)?
        } else {
            let coarse = self.correlate(&a.shrunk(factor), &b.shrunk(factor), false)?;
            Registration {
                shift: coarse.shift.map(|d| d * factor as f64),
                ..coarse
            }
        };
        // Then the residual, at full resolution, in windows of the overlap that show the same
        // content: the tapers no longer pull the peak toward zero. Again if it moves the whole
        // pixel.
        let mut found = first;
        for _ in 0..REFINEMENTS {
            let [dx, dy] = found.shift.map(|d| d.round() as i64);
            let Some((window_a, window_b)) = windows(&a, &b, (dx, dy), self.options.max_size)
            else {
                break;
            };
            let fine = self.correlate(&window_a, &window_b, true)?;
            found = Registration {
                shift: [dx as f64 + fine.shift[0], dy as f64 + fine.shift[1]],
                ..fine
            };
            if fine.shift.iter().all(|d| d.abs() <= 0.5) {
                break;
            }
        }
        // How much the peak stands out is the search's, over the whole images.
        Ok(Registration {
            pce: first.pce,
            ..found
        })
    }

    fn plans(&self, width: usize, height: usize) -> Arc<Plans> {
        let mut plans = self.plans.lock().unwrap_or_else(PoisonError::into_inner);
        let plan = plans
            .entry((width, height))
            .or_insert_with(|| Arc::new(Plans::new(width, height)));
        Arc::clone(plan)
    }

    /// Phase correlation of two planes of one size. `near` keeps only the hypothesis nearest zero
    /// of the highest peak, as the fine stage, whose shift is small, needs.
    fn correlate(&self, a: &Plane, b: &Plane, near: bool) -> Result<Registration, RegisterError> {
        let (width, height) = (a.width, a.height);
        let plans = self.plans(width, height);
        // The search keeps the edges, where low overlaps lie; a refinement's windows show the same
        // content, and Hann's taper keeps their edges from pulling the peak.
        let window = if near {
            Window::Hann
        } else {
            self.options.window
        };
        let (mut ta, mut tb) = (tapered(a, window)?, tapered(b, window)?);
        let (fa, fb) = (plans.forward(&mut ta), plans.forward(&mut tb));
        // The phase difference, low-passed so the peak is a Gaussian of `smoothing` pixels: its
        // shape is then known, and the noisiest frequencies weigh little.
        let half = width / 2 + 1;
        let sigma = self.options.smoothing.max(0.0);
        let gauss = |k: usize, n: usize| {
            let f = k.min(n - k) as f64 / n as f64;
            (-2.0 * (std::f64::consts::PI * sigma * f).powi(2)).exp()
        };
        let (gx, gy): (Vec<f64>, Vec<f64>) = (
            (0..half).map(|k| gauss(k, width)).collect(),
            (0..height).map(|k| gauss(k, height)).collect(),
        );
        let mut cross: Vec<Complex<f32>> = fa
            .iter()
            .zip(&fb)
            .enumerate()
            .map(|(i, (x, y))| {
                let c = x.conj() * y;
                let norm = c.norm();
                if norm > f32::MIN_POSITIVE {
                    c * (gx[i % half] * gy[i / half]) as f32 / norm
                } else {
                    Complex::default()
                }
            })
            .collect();
        let surface = plans.inverse(&mut cross);
        let peaks = peaks(&surface, width, height, self.options.candidates.max(2));
        let (first, second) = (peaks[0].0, peaks.get(1).map_or(0.0, |peak| peak.0.max(0.0)));
        let margin = if first > 0.0 {
            f64::from(1.0 - second / first).clamp(0.0, 1.0)
        } else {
            0.0
        };

        // Each peak's hypothesis is the one whose overlap correlates best; among the peaks, the
        // highest peak times its correlation wins, so a tiny overlap correlating by chance
        // doesn't.
        let mut best: Option<(f64, f64, usize, [i64; 2])> = None;
        let considered = if near { 1 } else { peaks.len() };
        for (k, &(height_k, x, y)) in peaks.iter().enumerate().take(considered) {
            let mut chosen: Option<(f64, [i64; 2])> = None;
            for dx in hypotheses(x, width, near) {
                for dy in hypotheses(y, height, near) {
                    let Some(ncc) = ncc(a, b, [dx, dy], self.options.min_overlap) else {
                        continue;
                    };
                    if chosen.is_none_or(|(highest, _)| ncc > highest) {
                        chosen = Some((ncc, [dx, dy]));
                    }
                }
            }
            if let Some((ncc, shift)) = chosen {
                let score = f64::from(height_k) * ncc.max(0.0);
                if best.is_none_or(|(top, ..)| score > top) {
                    best = Some((score, ncc, k, shift));
                }
            }
        }
        let (_, ncc, k, [dx, dy]) = best.ok_or(RegisterError::Flat)?;
        let (peak, x, y) = peaks[k];
        let at = |i: i64, j: i64| {
            let (i, j) = (i.rem_euclid(width as i64), j.rem_euclid(height as i64));
            f64::from(surface[j as usize * width + i as usize])
        };
        let (x, y) = (x as i64, y as i64);
        let fraction = [
            self.subpixel(at(x - 1, y), at(x, y), at(x + 1, y)),
            self.subpixel(at(x, y - 1), at(x, y), at(x, y + 1)),
        ];
        // The peak's square over the mean square of the surface outside it.
        let (mut energy, mut count) = (0.0, 0usize);
        for (j, row) in surface.chunks_exact(width).enumerate() {
            for (i, &value) in row.iter().enumerate() {
                if wrapped(i as i64 - x, width) > PEAK_RADIUS
                    || wrapped(j as i64 - y, height) > PEAK_RADIUS
                {
                    energy += f64::from(value).powi(2);
                    count += 1;
                }
            }
        }
        let pce = f64::from(peak).powi(2) / (energy / count.max(1) as f64).max(f64::MIN_POSITIVE);
        Ok(Registration {
            shift: [dx as f64 + fraction[0], dy as f64 + fraction[1]],
            pce,
            margin,
            ncc,
        })
    }

    /// The peak's offset from its pixel, from its value `c` and its neighbours `l` and `r`.
    fn subpixel(&self, l: f64, c: f64, r: f64) -> f64 {
        let offset = match self.options.subpixel {
            Subpixel::Parabolic => {
                let curvature = l - 2.0 * c + r;
                if curvature.abs() > f64::EPSILON * c.abs() {
                    0.5 * (l - r) / curvature
                } else {
                    0.0
                }
            }
            Subpixel::Gaussian if l > 0.0 && c > 0.0 && r > 0.0 => {
                let (l, c, r) = (l.ln(), c.ln(), r.ln());
                let curvature = l - 2.0 * c + r;
                if curvature < 0.0 {
                    0.5 * (l - r) / curvature
                } else {
                    0.0
                }
            }
            Subpixel::Gaussian => 0.0,
            Subpixel::Foroosh if r >= l && r > 0.0 => r / (r + c),
            Subpixel::Foroosh if l > 0.0 => -l / (l + c),
            Subpixel::Foroosh => 0.0,
        };
        offset.clamp(-1.0, 1.0)
    }
}

/// `plane` less its mean, tapered by `window`: what is transformed.
fn tapered(plane: &Plane, window: Window) -> Result<Vec<f32>, RegisterError> {
    let mean = plane.data.iter().map(|&v| f64::from(v)).sum::<f64>() / plane.data.len() as f64;
    let spread = plane
        .data
        .iter()
        .map(|&v| (f64::from(v) - mean).powi(2))
        .sum::<f64>();
    if spread <= f64::EPSILON {
        return Err(RegisterError::Flat);
    }
    let (wx, wy) = (taper(window, plane.width), taper(window, plane.height));
    let mut out = Vec::with_capacity(plane.data.len());
    for (row, &weight_y) in plane.data.chunks_exact(plane.width).zip(&wy) {
        out.extend(
            row.iter()
                .zip(&wx)
                .map(|(&v, &weight_x)| ((f64::from(v) - mean) * weight_x * weight_y) as f32),
        );
    }
    Ok(out)
}

impl Default for Registrar {
    fn default() -> Self {
        Self::new(Options::default())
    }
}

/// An image as `f32`, row after row.
#[derive(Debug, Clone)]
struct Plane {
    data: Vec<f32>,
    width: usize,
    height: usize,
}

impl Plane {
    fn of<T: Copy + Into<f32>>(view: ImageView<'_, T>) -> Self {
        let data = view.rows().flatten().map(|&v| v.into()).collect();
        Self {
            data,
            width: view.width(),
            height: view.height(),
        }
    }

    /// The mean of each `factor` × `factor` block; a partial block at the edges is dropped.
    fn shrunk(&self, factor: usize) -> Self {
        let (width, height) = ((self.width / factor).max(1), (self.height / factor).max(1));
        let mut data = vec![0.0f32; width * height];
        for y in 0..height * factor {
            let row = &self.data[y * self.width..][..width * factor];
            for (x, chunk) in row.chunks_exact(factor).enumerate() {
                data[(y / factor) * width + x] += chunk.iter().sum::<f32>();
            }
        }
        let area = (factor * factor) as f32;
        data.iter_mut().for_each(|v| *v /= area);
        Self {
            data,
            width,
            height,
        }
    }

    /// The `width` × `height` pixels from (`x`, `y`).
    fn crop(&self, (x, y): (usize, usize), (width, height): (usize, usize)) -> Self {
        let data = (y..y + height)
            .flat_map(|row| &self.data[row * self.width + x..][..width])
            .copied()
            .collect();
        Self {
            data,
            width,
            height,
        }
    }
}

/// The weights of `window` across `n` pixels.
fn taper(window: Window, n: usize) -> Vec<f64> {
    let last = (n.max(2) - 1) as f64;
    let cosine = |t: f64| 0.5 - 0.5 * (std::f64::consts::PI * t).cos();
    (0..n)
        .map(|i| {
            let i = i as f64;
            match window {
                Window::Hann => cosine(2.0 * i / last),
                Window::Tukey(share) => {
                    let edge = share.clamp(0.0, 1.0) * last / 2.0;
                    let from_edge = i.min(last - i);
                    if from_edge >= edge {
                        1.0
                    } else {
                        cosine(from_edge / edge)
                    }
                }
            }
        })
        .collect()
}

/// How far `d` is from 0 on a ring of `n`.
fn wrapped(d: i64, n: usize) -> i64 {
    let d = d.rem_euclid(n as i64);
    d.min(n as i64 - d)
}

/// The `k` highest local maxima of `surface`, highest first, each more than [`PEAK_RADIUS`] from
/// every higher one: value, column, row.
fn peaks(surface: &[f32], width: usize, height: usize, k: usize) -> Vec<(f32, usize, usize)> {
    let at = |i: i64, j: i64| {
        surface[j.rem_euclid(height as i64) as usize * width + i.rem_euclid(width as i64) as usize]
    };
    let mut maxima: Vec<(f32, usize, usize)> = Vec::new();
    for y in 0..height {
        for x in 0..width {
            let value = surface[y * width + x];
            let (i, j) = (x as i64, y as i64);
            let highest = (-1..=1)
                .flat_map(|dj| (-1..=1).map(move |di| (di, dj)))
                .all(|(di, dj)| (di, dj) == (0, 0) || at(i + di, j + dj) <= value);
            if highest {
                maxima.push((value, x, y));
            }
        }
    }
    maxima.sort_by(|p, q| q.0.total_cmp(&p.0));
    let mut kept: Vec<(f32, usize, usize)> = Vec::with_capacity(k);
    for peak in maxima {
        let apart = kept.iter().all(|other| {
            wrapped(peak.1 as i64 - other.1 as i64, width) > PEAK_RADIUS
                || wrapped(peak.2 as i64 - other.2 as i64, height) > PEAK_RADIUS
        });
        if apart {
            kept.push(peak);
        }
        if kept.len() == k {
            break;
        }
    }
    kept
}

/// The shifts a peak at `p` of `n` may stand for: itself, or one turn less; only the one nearest
/// zero if `near`.
fn hypotheses(p: usize, n: usize, near: bool) -> Vec<i64> {
    let (p, n) = (p as i64, n as i64);
    if near {
        vec![if p > n / 2 { p - n } else { p }]
    } else {
        vec![p, p - n]
    }
}

/// The normalised cross-correlation of `a` and `b` over their overlap at `shift`, if the overlap
/// is at least `min_overlap` of the area and neither side of it is flat.
fn ncc(a: &Plane, b: &Plane, [dx, dy]: [i64; 2], min_overlap: f64) -> Option<f64> {
    let (width, height) = (a.width as i64, a.height as i64);
    let (x0, x1) = (0.max(-dx), width.min(width - dx));
    let (y0, y1) = (0.max(-dy), height.min(height - dy));
    let area = (x1 - x0).max(0) * (y1 - y0).max(0);
    if (area as f64) < min_overlap * (width * height) as f64 || area == 0 {
        return None;
    }
    let (mut sa, mut sb, mut saa, mut sbb, mut sab) = (0.0, 0.0, 0.0, 0.0, 0.0);
    for y in y0..y1 {
        let row_a = &a.data[(y * width) as usize..][..a.width];
        let row_b = &b.data[((y + dy) * width) as usize..][..b.width];
        for x in x0..x1 {
            let (u, v) = (
                f64::from(row_a[x as usize]),
                f64::from(row_b[(x + dx) as usize]),
            );
            (sa, sb, saa, sbb, sab) = (sa + u, sb + v, saa + u * u, sbb + v * v, sab + u * v);
        }
    }
    let n = area as f64;
    let (va, vb) = (saa - sa * sa / n, sbb - sb * sb / n);
    (va > f64::EPSILON && vb > f64::EPSILON).then(|| (sab - sa * sb / n) / (va * vb).sqrt())
}

/// Windows of `a` and `b`, up to `size` a side, from the middle of their overlap at `shift`, the
/// second shifted with it: what the fine stage registers. `None` if the overlap is too small.
fn windows(a: &Plane, b: &Plane, (dx, dy): (i64, i64), size: usize) -> Option<(Plane, Plane)> {
    let (width, height) = (a.width as i64, a.height as i64);
    let (x0, x1) = (0.max(-dx), width.min(width - dx));
    let (y0, y1) = (0.max(-dy), height.min(height - dy));
    // Sides with no prime factor but 2, 3 and 5: quick to transform, and few, so plans are reused.
    let side = |extent: i64| {
        let mut n = extent.min(size as i64);
        while n > 1
            && ![2, 3, 5]
                .iter()
                .fold(n, |m, &p| {
                    let mut m = m;
                    while m % p == 0 {
                        m /= p;
                    }
                    m
                })
                .eq(&1)
        {
            n -= 1;
        }
        n
    };
    let (w, h) = (side(x1 - x0), side(y1 - y0));
    if w < MIN_SIZE as i64 || h < MIN_SIZE as i64 {
        return None;
    }
    let (x, y) = (x0 + (x1 - x0 - w) / 2, y0 + (y1 - y0 - h) / 2);
    let window = (w as usize, h as usize);
    let first = a.crop((x as usize, y as usize), window);
    let second = b.crop(((x + dx) as usize, (y + dy) as usize), window);
    Some((first, second))
}
