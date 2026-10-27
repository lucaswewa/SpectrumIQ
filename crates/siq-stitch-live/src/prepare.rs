//! A frame made ready to paint (ADR-0059): binned to about the canvas's scale, by a whole factor,
//! then corrected by the dark and the flat in force, on the calling rayon pool.

use rayon::prelude::*;
use siq_calib::shading::FlatField;
use siq_imgproc::{BinMode, KernelError, bin};
use siq_types::{Binning, FrameMeta, ImageError, ImageView, ImageViewMut, OwnedImage};

use crate::paint::Frame;

/// A frame binned and corrected.
#[derive(Debug, Clone, PartialEq)]
pub struct Prepared {
    /// Its samples, row by row.
    pub samples: Vec<u16>,
    /// Its width, in pixels.
    pub width: u32,
    /// Its height, in pixels.
    pub height: u32,
    /// The sensor pixels a side of its pixels covers: the camera's binning times this one's.
    pub binning: u32,
}

impl Prepared {
    /// It, to paint.
    pub fn frame(&self) -> Frame<'_> {
        Frame {
            samples: &self.samples,
            width: self.width,
            height: self.height,
        }
    }

    /// It, as an image: an error if its samples aren't its size.
    pub fn view(&self) -> Result<ImageView<'_, u16>, ImageError> {
        let (width, height) = (self.width as usize, self.height as usize);
        ImageView::new(&self.samples, width, height, width)
    }
}

/// What the gains were made for: the frame's corner on the sensor, the sensor pixels a side of
/// its pixels, its size, and the flat's coefficients.
type GainsKey = ([u32; 2], u32, (usize, usize), Vec<f64>);

/// Prepares frames, keeping the flat's gains for the last frames' geometry.
#[derive(Debug, Default)]
pub struct Preparer {
    gains: Option<(GainsKey, OwnedImage<f32>)>,
}

impl Preparer {
    /// `image`, a frame of `meta`, binned to about `canvas_scale` canvas pixels a sensor pixel, and
    /// corrected, `(value − dark) × gain`, if `shading` (the dark level and the flat) is given.
    pub fn prepare(
        &mut self,
        image: &ImageView<'_, u16>,
        meta: &FrameMeta,
        canvas_scale: f64,
        shading: Option<(f32, &FlatField)>,
    ) -> Result<Prepared, KernelError> {
        let camera = u32::from(meta.binning.horizontal.get());
        let factor = (1.0 / (canvas_scale * f64::from(camera))).round();
        let factor = factor.clamp(1.0, f64::from(u8::MAX)) as u8;
        let by = Binning::new(factor, factor).expect("a factor of at least 1");
        let (width, height) = by.image_size(image.width() as u32, image.height() as u32);
        let (w, h) = (width as usize, height as usize);
        let mut samples = vec![0u16; w * h];
        let mut out = ImageViewMut::new(&mut samples, w, h, w)
            .map_err(|_| KernelError::Parameter("a binned frame's size"))?;
        bin(image, by, BinMode::Mean, &mut out)?;
        let step = camera * u32::from(factor);
        if let Some((dark, flat)) = shading
            && w > 0
        {
            let corner = [meta.roi.offset_x, meta.roi.offset_y];
            let key = (corner, step, (w, h), flat.coefficients.clone());
            if self.gains.as_ref().is_none_or(|(kept, _)| *kept != key) {
                let gains = flat.gains_on(corner.map(f64::from), [f64::from(step); 2], (w, h));
                self.gains = Some((key, gains));
            }
            let gains = &self.gains.as_ref().expect("the gains").1.view();
            let max = f32::from(meta.pixel_format.max_value());
            samples.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
                for (value, &gain) in row.iter_mut().zip(gains.row(y)) {
                    *value = ((f32::from(*value) - dark).max(0.0) * gain)
                        .round()
                        .min(max) as u16;
                }
            });
        }
        Ok(Prepared {
            samples,
            width,
            height,
            binning: step,
        })
    }
}
