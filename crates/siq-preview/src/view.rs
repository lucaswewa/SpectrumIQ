//! The preview pipelines, a frame at a time: each keeps its buffers and its LUT between frames,
//! so a steady stream allocates only its JPEGs.

use jpeg_encoder::{ColorType, Encoder};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use siq_calib::shading::FlatField;
use siq_imgproc::{
    BinMode, Colormap, DisplayMapping, FocusMeasure, Histogram, Lut, balanced, bin, copy, demosaic,
    downsample_area, focus, histogram, histogram_cfa, map_colours, white_gains,
};
use siq_types::{Binning, Cfa, FrameMeta, ImageView, ImageViewMut, OwnedImage, RectPx};

use crate::PreviewError;

/// The point of each colour's values that automatic white balance shows white.
const WHITE: f64 = 0.999;

/// How raw values show.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Contrast {
    /// A fixed window, and a gamma.
    Manual {
        /// The value shown black, and below.
        min: u16,
        /// The value shown full, and above.
        max: u16,
        /// The gamma: above 1 brightens the shadows.
        gamma: f32,
    },
    /// The window between two percentiles of the frame's values, at gamma 1.
    Auto {
        /// The low percentile, from 0 to 100.
        low_pct: f64,
        /// The high percentile, from 0 to 100.
        high_pct: f64,
    },
}

/// How a colour frame's colours are balanced: red's and blue's gains, green's being 1.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WhiteBalance {
    /// Each frame's brightest shows white: red's and blue's 99.9th percentiles are brought to
    /// green's.
    #[default]
    Auto,
    /// Fixed gains.
    Fixed {
        /// Red's gain.
        red: f32,
        /// Blue's gain.
        blue: f32,
    },
}

/// How the previews show a frame: the contrast, the colours, and a colour frame's white balance.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct DisplaySettings {
    /// The window, fixed or automatic: on a colour frame, green's, after the white balance.
    pub contrast: Contrast,
    /// The colours of a mono frame. A colour frame shows its own, inverted or not.
    pub colormap: Colormap,
    /// How a colour frame's colours are balanced.
    #[serde(default)]
    pub white_balance: WhiteBalance,
}

impl Default for DisplaySettings {
    /// Automatic, from the 0.1th to the 99.9th percentile, in grey, with automatic white balance.
    fn default() -> Self {
        Self {
            contrast: Contrast::Auto {
                low_pct: 0.1,
                high_pct: 99.9,
            },
            colormap: Colormap::Grey,
            white_balance: WhiteBalance::Auto,
        }
    }
}

/// How a frame's values show: a mono frame's mapping, or a colour frame's filter array at its top
/// left and its red's, green's and blue's mappings.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Mapping {
    /// A mono frame's.
    Mono(DisplayMapping),
    /// A colour frame's.
    Colour(Cfa, [DisplayMapping; 3]),
}

/// How much of the sensor one ROI preview pixel shows.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Zoom {
    /// One frame pixel per preview pixel.
    #[default]
    Native,
    /// 2 × 2 frame pixels per preview pixel, averaged.
    Half,
}

impl Zoom {
    fn factor(self) -> u32 {
        match self {
            Zoom::Native => 1,
            Zoom::Half => 2,
        }
    }
}

/// The ROI stream's region and zoom.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RoiSettings {
    /// The region, in sensor pixels; the middle of the frame if `None`.
    pub region: Option<RectPx>,
    /// The zoom.
    pub zoom: Zoom,
}

/// The largest size within `max` with the aspect of `size`, never larger than `size`.
pub fn fit((width, height): (u32, u32), (max_width, max_height): (u32, u32)) -> (u32, u32) {
    let scale = (f64::from(max_width) / f64::from(width))
        .min(f64::from(max_height) / f64::from(height))
        .min(1.0);
    let side = |n: u32| ((f64::from(n) * scale).round() as u32).max(1);
    (side(width), side(height))
}

/// Where the ROI preview reads in the frame, in frame pixels, and its size, at most `max`.
///
/// - The region, in sensor pixels, is mapped through the frame's ROI and binning, and clamped to
///   the frame.
/// - It is cut to `max` times the zoom, about its middle, and aligned to the zoom, so that 2:1
///   averages whole pairs of pixels.
pub fn roi_region(
    meta: &FrameMeta,
    settings: &RoiSettings,
    (max_width, max_height): (u32, u32),
) -> (RectPx, (u32, u32)) {
    let (width, height) = meta.image_size();
    let ((bx, by), roi, z) = (meta.binning.factors(), meta.roi, settings.zoom.factor());
    // A sensor column or row, as a frame one, clamped to the frame.
    let frame = |sensor: u64, offset: u32, bin: u8, end: u32| {
        let at = (sensor as i64 - i64::from(offset)).div_euclid(i64::from(bin));
        at.clamp(0, i64::from(end)) as u32
    };
    let asked = settings.region.and_then(|r| {
        let (x0, y0) = (
            frame(r.x.into(), roi.offset_x, bx, width),
            frame(r.y.into(), roi.offset_y, by, height),
        );
        let x1 = frame(u64::from(r.x) + u64::from(r.width), roi.offset_x, bx, width);
        let y1 = frame(
            u64::from(r.y) + u64::from(r.height),
            roi.offset_y,
            by,
            height,
        );
        (x1 > x0 && y1 > y0).then_some((x0, y0, x1 - x0, y1 - y0))
    });
    // Nothing asked, or nothing of it in the frame: the middle of the frame.
    let (x, y, w, h) = asked.unwrap_or((0, 0, width, height));
    // At most `max` preview pixels, in whole steps of the zoom, about the region's middle.
    let cut = |n: u32, max: u32, end: u32| (n.min(max * z) / z * z).max(z).min(end);
    let (cw, ch) = (cut(w, max_width, width), cut(h, max_height, height));
    let start = |at: u32, n: u32, cut: u32, end: u32| {
        ((at + n / 2).saturating_sub(cut / 2) / z * z).min(end - cut)
    };
    let rect = RectPx::new(start(x, w, cw, width), start(y, h, ch, height), cw, ch);
    (rect, ((cw / z).max(1), (ch / z).max(1)))
}

/// A frame's preview, before and after encoding.
#[derive(Debug, Default)]
pub struct Renderer {
    scaled: Vec<u16>,
    rgb: Vec<[u16; 3]>,
    levels: Vec<u8>,
    colours: Vec<[u8; 3]>,
    lut: Option<(DisplayMapping, Lut)>,
    luts: Option<([DisplayMapping; 3], Box<[Lut; 3]>)>,
    histograms: Option<Box<[Histogram; 3]>>,
    gains: Option<(GainsKey, OwnedImage<f32>)>,
    shown: (u32, u32, bool),
}

/// A dark level and a flat: what corrects a preview (ADR-0050).
pub type Shading = (f32, FlatField);

/// What the last corrected preview's gains were for: its grid's origin and step on the sensor,
/// its size, and the flat's coefficients.
type GainsKey = ([f64; 4], (usize, usize), Vec<f64>);

/// Every `step`-th pixel both ways, for about a million samples.
fn sample_step(view: &ImageView<'_, u16>) -> usize {
    ((view.width() * view.height()) as f64 / 1e6)
        .sqrt()
        .ceil()
        .max(1.0) as usize
}

impl Renderer {
    /// The mapping `settings` give for `frame`, of `bits`-bit values, with `cfa` at its top left
    /// if it is a colour frame. An automatic window, and automatic white balance, come from
    /// strided histograms of the whole frame: on a colour frame, one per colour, the window being
    /// green's.
    pub fn mapping(
        &mut self,
        frame: &ImageView<'_, u16>,
        bits: u32,
        cfa: Option<Cfa>,
        settings: &DisplaySettings,
    ) -> Mapping {
        let auto_white = cfa.is_some() && settings.white_balance == WhiteBalance::Auto;
        let counts = self
            .histograms
            .get_or_insert_with(|| Box::new(std::array::from_fn(|_| Histogram::new(bits))));
        if auto_white || matches!(settings.contrast, Contrast::Auto { .. }) {
            **counts = std::array::from_fn(|_| Histogram::new(bits));
            let _ = match cfa {
                Some(cfa) => histogram_cfa(frame, cfa, sample_step(frame), counts),
                None => histogram(frame, sample_step(frame), &mut counts[1]),
            };
        }
        let (min, max, gamma) = match settings.contrast {
            Contrast::Manual { min, max, gamma } => (min, max.max(min.saturating_add(1)), gamma),
            Contrast::Auto { low_pct, high_pct } => {
                let (min, max) = counts[1].auto_window(low_pct / 100.0, high_pct / 100.0);
                (min, max, 1.0)
            }
        };
        let mapping = DisplayMapping {
            min,
            max,
            gamma,
            colormap: settings.colormap,
        };
        let Some(cfa) = cfa else {
            return Mapping::Mono(mapping);
        };
        let gains = match settings.white_balance {
            WhiteBalance::Auto => white_gains(counts, WHITE),
            WhiteBalance::Fixed { red, blue } => [red, 1.0, blue],
        };
        Mapping::Colour(cfa, balanced(&mapping, gains))
    }

    /// Shows `region` of `frame` at `size`, averaging by area to shrink, through `mapping`;
    /// corrected by `shading` if given, for the frame of `meta`: `(value − dark) × gain`, after
    /// shrinking, as the gain is smooth. A colour frame is demosaiced: at 1:1 by interpolation,
    /// and shrunk from whole cells.
    pub fn show(
        &mut self,
        frame: &ImageView<'_, u16>,
        region: RectPx,
        (width, height): (u32, u32),
        mapping: Mapping,
        shading: Option<(&Shading, &FrameMeta)>,
    ) -> Result<(), PreviewError> {
        let mapping = match mapping {
            Mapping::Mono(mapping) => mapping,
            Mapping::Colour(cfa, mappings) => {
                return self.show_colour(frame, region, (width, height), (cfa, mappings), shading);
            }
        };
        if self.lut.as_ref().is_none_or(|(kept, _)| *kept != mapping) {
            self.lut = Some((mapping, Lut::new(&mapping)?));
        }
        let input = frame.sub_view(region)?;
        let (w, h) = (width as usize, height as usize);
        let resized = (w, h) != (input.width(), input.height());
        if resized || shading.is_some() {
            self.scaled.resize(w * h, 0);
            let mut out = ImageViewMut::new(&mut self.scaled, w, h, w)?;
            match resized {
                true => downsample_area(&input, &mut out)?,
                false => copy(&input, &mut out)?,
            }
        }
        if let Some((shading, meta)) = shading {
            self.correct(shading, meta, region, (w, h), false);
        }
        let lut = &self.lut.as_ref().expect("a LUT").1;
        let source = match resized || shading.is_some() {
            true => ImageView::new(&self.scaled, w, h, w)?,
            false => input,
        };
        let colour = matches!(mapping.colormap, Colormap::Colour(_));
        if colour {
            self.colours.resize(w * h, [0; 3]);
            lut.apply_rgb(&source, &mut ImageViewMut::new(&mut self.colours, w, h, w)?)?;
        } else {
            self.levels.resize(w * h, 0);
            lut.apply(&source, &mut ImageViewMut::new(&mut self.levels, w, h, w)?)?;
        }
        self.shown = (width, height, colour);
        Ok(())
    }

    /// A colour frame's [`show`](Self::show), through red's, green's and blue's mappings.
    fn show_colour(
        &mut self,
        frame: &ImageView<'_, u16>,
        region: RectPx,
        (width, height): (u32, u32),
        (cfa, mappings): (Cfa, [DisplayMapping; 3]),
        shading: Option<(&Shading, &FrameMeta)>,
    ) -> Result<(), PreviewError> {
        if self.luts.as_ref().is_none_or(|(kept, _)| *kept != mappings) {
            let [red, green, blue] = mappings.each_ref().map(Lut::new);
            self.luts = Some((mappings, Box::new([red?, green?, blue?])));
        }
        let (w, h) = (width as usize, height as usize);
        self.rgb.resize(w * h, [0; 3]);
        let mut rgb = ImageViewMut::new(&mut self.rgb, w, h, w)?;
        let cfa = cfa.shifted(region.x, region.y);
        demosaic(&frame.sub_view(region)?, cfa, &mut rgb)?;
        if let Some((shading, meta)) = shading {
            self.correct(shading, meta, region, (w, h), true);
        }
        self.colours.resize(w * h, [0; 3]);
        let luts = &self.luts.as_ref().expect("the LUTs").1;
        let mut out = ImageViewMut::new(&mut self.colours, w, h, w)?;
        map_colours(luts, &ImageView::new(&self.rgb, w, h, w)?, &mut out)?;
        self.shown = (width, height, true);
        Ok(())
    }

    /// Corrects the shrunk preview of `region` of the frame of `meta`, of `size`, in place: its
    /// colours if `colour`.
    fn correct(
        &mut self,
        (dark, flat): &Shading,
        meta: &FrameMeta,
        region: RectPx,
        (w, h): (usize, usize),
        colour: bool,
    ) {
        let (bx, by) = meta.binning.factors();
        let (bx, by) = (f64::from(bx), f64::from(by));
        let grid = [
            f64::from(meta.roi.offset_x) + f64::from(region.x) * bx,
            f64::from(meta.roi.offset_y) + f64::from(region.y) * by,
            f64::from(region.width) / w as f64 * bx,
            f64::from(region.height) / h as f64 * by,
        ];
        let key = (grid, (w, h), flat.coefficients.clone());
        if self.gains.as_ref().is_none_or(|(kept, _)| *kept != key) {
            let gains = flat.gains_on([grid[0], grid[1]], [grid[2], grid[3]], (w, h));
            self.gains = Some((key, gains));
        }
        let gains = self.gains.as_ref().expect("the gains").1.view();
        let max = f32::from(meta.pixel_format.max_value());
        let corrected = |value: u16, gain: f32| {
            ((f32::from(value) - dark).max(0.0) * gain).round().min(max) as u16
        };
        let gains = gains.rows().flatten();
        if colour {
            for (pixel, &gain) in self.rgb.iter_mut().zip(gains) {
                *pixel = pixel.map(|value| corrected(value, gain));
            }
        } else {
            for (value, &gain) in self.scaled.iter_mut().zip(gains) {
                *value = corrected(*value, gain);
            }
        }
    }

    /// The last image shown: its size, and its bytes, one per pixel in grey or three in colour.
    pub fn shown(&self) -> ((u32, u32), &[u8]) {
        let (width, height, colour) = self.shown;
        let bytes = if colour {
            self.colours.as_flattened()
        } else {
            &self.levels[..]
        };
        ((width, height), bytes)
    }

    /// The last image shown, as a JPEG of `quality` from 1 to 100.
    pub fn encode(&self, quality: u8) -> Result<Vec<u8>, PreviewError> {
        let ((width, height), bytes) = self.shown();
        let colour = if self.shown.2 {
            ColorType::Rgb
        } else {
            ColorType::Luma
        };
        let mut jpeg = Vec::with_capacity(bytes.len() / 8);
        let size = |n: u32| u16::try_from(n).map_err(|_| PreviewError::TooLarge(width, height));
        let (width, height) = (size(width)?, size(height)?);
        Encoder::new(&mut jpeg, quality.clamp(1, 100)).encode(bytes, width, height, colour)?;
        Ok(jpeg)
    }
}

/// A histogram in 256 bins over the pixel format's range, and its 0.1th and 99.9th percentiles.
pub type Counts = ([u32; 256], (u16, u16));

/// What the statistics stream reports of a frame's ROI.
#[derive(Debug, Clone, PartialEq)]
pub struct FrameStats {
    /// The frame's metadata.
    pub meta: FrameMeta,
    /// The region measured, in frame pixels.
    pub region: RectPx,
    /// The histogram of the region's values, in 256 bins over the pixel format's range: on a
    /// colour frame, green's.
    pub histogram: [u32; 256],
    /// The 0.1th and 99.9th percentiles of the region's values: on a colour frame, green's.
    pub window: (u16, u16),
    /// The region's sharpness: its mean squared Sobel gradient (Tenengrad), on a colour frame of
    /// its luminance, each cell's mean.
    pub focus: f64,
    /// A colour frame's red's, green's and blue's histograms and windows.
    pub colours: Option<[Counts; 3]>,
}

/// What the statistics keep from frame to frame.
#[derive(Debug, Default)]
pub struct Measuring {
    counts: Option<Box<[Histogram; 3]>>,
    luminance: Vec<u16>,
}

/// The statistics of `region` of `frame`, of `bits`-bit values, through `scratch`.
pub fn measure(
    frame: &ImageView<'_, u16>,
    meta: &FrameMeta,
    bits: u32,
    region: RectPx,
    scratch: &mut Measuring,
) -> Result<FrameStats, PreviewError> {
    let input = frame.sub_view(region)?;
    let counts = scratch
        .counts
        .get_or_insert_with(|| Box::new(std::array::from_fn(|_| Histogram::new(bits))));
    **counts = std::array::from_fn(|_| Histogram::new(bits));
    let cfa = meta.cfa.map(|cfa| cfa.shifted(region.x, region.y));
    let sharpness = match cfa {
        None => {
            histogram(&input, sample_step(&input), &mut counts[1])?;
            focus(&input, FocusMeasure::Tenengrad)
        }
        Some(cfa) => {
            histogram_cfa(&input, cfa, sample_step(&input), counts)?;
            // On luminance, each cell's mean: a cell's colours aren't detail.
            let (w, h) = (input.width() / 2, input.height() / 2);
            scratch.luminance.resize(w * h, 0);
            let mut luminance = ImageViewMut::new(&mut scratch.luminance, w, h, w)?;
            let cell = Binning::new(2, 2).expect("2 × 2");
            bin(&input, cell, BinMode::Mean, &mut luminance)?;
            focus(&luminance.as_view(), FocusMeasure::Tenengrad)
        }
    };
    let shown = |counts: &Histogram| {
        let mut binned = [0u32; 256];
        for (i, &n) in counts.bins().iter().enumerate() {
            binned[i * 256 / counts.bins().len()] += n;
        }
        (binned, counts.auto_window(0.001, 0.999))
    };
    let (histogram, window) = shown(&counts[1]);
    Ok(FrameStats {
        meta: *meta,
        region,
        histogram,
        window,
        focus: sharpness,
        colours: cfa.map(|_| counts.each_ref().map(shown)),
    })
}
