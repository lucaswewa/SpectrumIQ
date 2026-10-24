//! Dark and flat-field correction (REQ-CAL-002): `(raw − dark) × gain`, rounded, and clamped to
//! the pixel format's range.

use siq_types::{ImageView, ImageViewMut};

use crate::bands::for_bands;
use crate::{KernelError, Sample, check, size, size_mut};

/// The calibration images of one objective and channel, each the size of the frames.
#[derive(Debug, Clone, Copy, Default)]
pub struct Calibration<'a> {
    /// The dark frame: the sensor's offset per pixel, subtracted first.
    pub dark: Option<ImageView<'a, u16>>,
    /// Each pixel's gain, from the flat field: about 1, and higher where the field is dimmer.
    pub gain: Option<ImageView<'a, f32>>,
}

impl Calibration<'_> {
    fn check(&self, frame: (usize, usize)) -> Result<(), KernelError> {
        self.dark
            .map_or(Ok(()), |dark| check("the dark frame", frame, size(&dark)))?;
        self.gain
            .map_or(Ok(()), |gain| check("the gain map", frame, size(&gain)))
    }
}

/// Corrects `raw` into `out`, clamping to `max`, such as 16383 for 14-bit pixels.
pub fn correct(
    raw: &ImageView<'_, u16>,
    calibration: &Calibration<'_>,
    max: u16,
    out: &mut ImageViewMut<'_, u16>,
) -> Result<(), KernelError> {
    check("the output", size(raw), size_mut(out))?;
    calibration.check(size(raw))?;
    for_bands(out.reborrow(), &|first, mut band| {
        for (y, row) in (first..).zip(band.rows_mut()) {
            row.copy_from_slice(raw.row(y));
            correct_row(row, calibration, y, max);
        }
    });
    Ok(())
}

/// Corrects `image` where it is, clamping to `max`.
pub fn correct_in_place(
    image: &mut ImageViewMut<'_, u16>,
    calibration: &Calibration<'_>,
    max: u16,
) -> Result<(), KernelError> {
    calibration.check(size_mut(image))?;
    for_bands(image.reborrow(), &|first, mut band| {
        for (y, row) in (first..).zip(band.rows_mut()) {
            correct_row(row, calibration, y, max);
        }
    });
    Ok(())
}

/// Corrects one row, row `y` of the frame, where it is.
fn correct_row(row: &mut [u16], calibration: &Calibration<'_>, y: usize, max: u16) {
    if let Some(dark) = calibration.dark {
        for (value, &offset) in row.iter_mut().zip(dark.row(y)) {
            *value = value.saturating_sub(offset);
        }
    }
    match calibration.gain {
        Some(gain) => {
            for (value, &gain) in row.iter_mut().zip(gain.row(y)) {
                *value = u16::from_f32(f32::from(*value) * gain).min(max);
            }
        }
        None => row.iter_mut().for_each(|value| *value = (*value).min(max)),
    }
}
