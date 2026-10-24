//! The noiseless signal: the world, blurred by its defocus, times the vignetting and each pixel's
//! own gain, times the binned area.

use siq_imgproc::{KernelError, MAX_SIGMA, for_each_row, gaussian_blur, upsample_bilinear};
use siq_synth::World;
use siq_types::{Affine2, ImageView, ImageViewMut, Micrometers, PixelToStage, PointPx};

use super::noise::{PRNU, gaussian, hash, key};
use super::{OpticsModel, SensorModel, Shot};

/// The buffers a render needs. They keep their memory between frames, and grow only when a
/// frame needs more, so drafts and full renders don't reallocate in turn.
#[derive(Debug, Default)]
pub struct Scratch {
    signal: Buffer,
    sharp: Buffer,
    blurred: Buffer,
    pass: Buffer,
    layer: Buffer,
    blur: Vec<f64>,
}

impl Scratch {
    /// The signal of the last render, at the frame's size.
    pub fn signal(&self) -> ImageView<'_, f32> {
        self.signal.view()
    }
}

/// An image whose rows are packed, in memory that only grows.
#[derive(Debug, Default)]
struct Buffer {
    data: Vec<f32>,
    size: (usize, usize),
}

impl Buffer {
    /// Makes the buffer `size`, growing its memory if it is too small.
    fn fit(&mut self, size: (usize, usize)) -> &mut Self {
        let needed = size.0 * size.1;
        if self.data.len() < needed {
            self.data.resize(needed, 0.0);
        }
        self.size = size;
        self
    }

    fn view(&self) -> ImageView<'_, f32> {
        let (width, height) = self.size;
        ImageView::new(&self.data, width, height, width).expect("a buffer holds its size")
    }

    fn view_mut(&mut self) -> ImageViewMut<'_, f32> {
        let (width, height) = self.size;
        ImageViewMut::new(&mut self.data, width, height, width).expect("a buffer holds its size")
    }
}

/// What a render did.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rendered {
    /// The world was sampled at 1/`detail` of the frame's resolution, then enlarged.
    pub detail: u32,
    /// The least and the greatest blur over the frame, as a standard deviation in its pixels.
    pub blur: [f64; 2],
    /// Whether the signal was the last one, so that only the noise is new.
    pub cached: bool,
}

/// The spacing of the grid the defocus is computed on, in frame pixels.
const GRID: usize = 32;

/// The most blur levels when the defocus varies across the frame.
const LEVELS: usize = 6;

/// Renders the noiseless signal of `shot` into `scratch`. [`digitize`](super::digitize) turns it
/// into a frame.
///
/// - The world is sampled at 1/`detail` of the frame's resolution, then enlarged. A strongly
///   blurred frame is sampled more coarsely still: at about half its blur.
/// - The defocus at a point is `z_coarse + focus` plus the world's surface height there; a
///   volume's surface is taken as 0.
/// - A defocus that varies across the frame blends up to six blur levels.
pub fn render_signal(
    world: &dyn World,
    optics: &OpticsModel,
    sensor: &SensorModel,
    shot: &Shot,
    detail: u32,
    scratch: &mut Scratch,
) -> Result<Rendered, KernelError> {
    let Scratch {
        signal,
        sharp,
        blurred,
        pass,
        layer,
        blur,
    } = scratch;
    let (width, height) = shot.image_size();
    let to_stage = shot.to_stage();
    let pixel = to_stage.0.determinant().abs().sqrt();
    let focal = shot.pose.z_coarse.0 + shot.pose.focus.0;

    // The blur, in frame pixels, on a grid over the frame.
    let (columns, rows) = (width.div_ceil(GRID) + 1, height.div_ceil(GRID) + 1);
    blur.clear();
    blur.extend((0..rows * columns).map(|n| {
        let corner = PointPx::new(
            ((n % columns) * GRID).min(width) as f64,
            ((n / columns) * GRID).min(height) as f64,
        );
        let at = to_stage.apply(corner);
        let surface = world.surface_z(at).map_or(0.0, |z| z.0);
        optics.blur(Micrometers(focal + surface)).0 / pixel
    }));
    let blur = &*blur;
    let (least, most) = blur
        .iter()
        .fold((f64::MAX, 0.0f64), |(l, m), &b| (l.min(b), m.max(b)));
    let blur_at = |x: f64, y: f64| {
        let (gx, gy) = (
            (x / GRID as f64).clamp(0.0, (columns - 1) as f64),
            (y / GRID as f64).clamp(0.0, (rows - 1) as f64),
        );
        let (i, j) = (
            (gx as usize).min(columns.saturating_sub(2)),
            (gy as usize).min(rows.saturating_sub(2)),
        );
        let (fx, fy) = (gx - i as f64, gy - j as f64);
        let node = |i: usize, j: usize| blur[(j.min(rows - 1)) * columns + i.min(columns - 1)];
        let top = node(i, j) + (node(i + 1, j) - node(i, j)) * fx;
        let bottom = node(i, j + 1) + (node(i + 1, j + 1) - node(i, j + 1)) * fx;
        top + (bottom - top) * fy
    };

    // Sample coarsely when even the sharpest part is blurred over several pixels.
    let coarse = match least >= 4.0 {
        true => 1u32 << (least / 2.0).log2().floor() as u32,
        false => 1,
    };
    let detail = detail.max(coarse).clamp(1, width.max(height).max(1) as u32);
    let d = f64::from(detail);
    let size = (
        width.div_ceil(detail as usize),
        height.div_ceil(detail as usize),
    );
    let sampled = PixelToStage(to_stage.0.compose(&Affine2::scale(d, d)));
    let footprint = Micrometers(pixel * d);
    for_each_row(&mut sharp.fit(size).view_mut(), |y, line| {
        for (x, value) in (0u32..).zip(line.iter_mut()) {
            *value = world.intensity(sampled.apply(PointPx::pixel_centre(x, y as u32)), footprint);
        }
    });

    // Blur at the sampled resolution, into the signal, or into `blurred` to enlarge.
    let enlarged = detail > 1;
    let target = match enlarged {
        true => blurred.fit(size),
        false => signal.fit(size),
    };
    let sigma = |frame_pixels: f64| ((frame_pixels / d) as f32).min(MAX_SIGMA);
    if (most - least) / d < 0.5 {
        gaussian_blur(
            &sharp.view(),
            sigma((least + most) / 2.0),
            &mut pass.fit(size).view_mut(),
            &mut target.view_mut(),
        )?;
    } else {
        // Blur levels evenly across the range, each pixel weighting the two around its blur.
        let levels = (((most - least) / d).ceil() as usize + 1).clamp(2, LEVELS);
        let step = (most - least) / (levels - 1) as f64;
        target.view_mut().fill(0.0);
        let layer = layer.fit(size);
        for level in 0..levels {
            let at = least + step * level as f64;
            gaussian_blur(
                &sharp.view(),
                sigma(at),
                &mut pass.fit(size).view_mut(),
                &mut layer.view_mut(),
            )?;
            let layer = layer.view();
            for_each_row(&mut target.view_mut(), |y, line| {
                for (x, (value, &blurred)) in line.iter_mut().zip(layer.row(y)).enumerate() {
                    let here = blur_at((x as f64 + 0.5) * d, (y as f64 + 0.5) * d);
                    *value += (1.0 - (here - at).abs() / step).max(0.0) as f32 * blurred;
                }
            });
        }
    }
    if enlarged {
        upsample_bilinear(&blurred.view(), &mut signal.fit((width, height)).view_mut())?;
    }

    // The vignetting, each pixel's gain, and the binned area.
    let settings = &shot.settings;
    let (bx, by) = settings.binning.factors();
    let (bx, by) = (f64::from(bx), f64::from(by));
    let (cx, cy) = (
        f64::from(shot.sensor.0) / 2.0,
        f64::from(shot.sensor.1) / 2.0,
    );
    let corner = cx * cx + cy * cy;
    let (x0, y0) = (
        f64::from(settings.roi.offset_x),
        f64::from(settings.roi.offset_y),
    );
    let (gains, profile) = (key(sensor.seed, PRNU), optics.vignetting.profile());
    for_each_row(&mut signal.view_mut(), |y, line| {
        let first_row = y0 + y as f64 * by;
        let dy = first_row + by / 2.0 - cy;
        for (x, value) in line.iter_mut().enumerate() {
            let first_column = x0 + x as f64 * bx;
            let dx = first_column + bx / 2.0 - cx;
            let vignetting = profile.at((dx * dx + dy * dy) / corner);
            let gain = 1.0
                + sensor.prnu
                    * f64::from(gaussian(hash(gains, first_column as u64, first_row as u64)));
            *value *= (bx * by * vignetting * gain) as f32;
        }
    });
    Ok(Rendered {
        detail,
        blur: [least, most],
        cached: false,
    })
}
