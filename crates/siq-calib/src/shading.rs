//! Shading correction (ADR-0050): the dark, each pixel's offset, and the flat, how the light falls
//! across the sensor. A frame corrected is `(raw − dark) × gain`, the gain `1 / flat` at each
//! pixel's place on the sensor (`siq_imgproc::correct`).
//!
//! - **[`Accumulator`]** sums frames for their mean: the dark's, or the flat's.
//! - **[`FlatField`]** is a smooth surface: a polynomial of degree [`DEGREE`] over the sensor,
//!   fitted by least squares to a measured flat shrunk to at most [`GRID`] cells a side, relative
//!   to its mean. Dust, the sample and each pixel's own gain don't fit it.
//! - **[`Estimator`]** gives it from many frames of different places (BaSiC, simplified): each
//!   shrunk and divided by its mean, the median of each cell across them, then fitted.
//! - **[`FlatFieldView`]** corrects frames of one geometry as they are read.
//! - **[`ShadingRecord`]** is a dark or a flat as it is kept, per objective and channel.

use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use siq_imgproc::{Calibration, correct, downsample_area, for_each_row};
use siq_types::{
    Binning, CalibrationId, ChannelId, FrameMeta, ImageView, ObjectiveId, OwnedImage, RectPx, Roi,
};

use crate::CalibError;
use crate::registry::{Registry, Versioned};

/// The flat's polynomial degree: enough for a cos⁴ fall-off to a corner at 0.6 of the centre.
pub const DEGREE: usize = 6;
/// The most cells a side a flat is fitted on.
pub const GRID: usize = 128;
/// The polynomial's terms.
const TERMS: usize = (DEGREE + 1) * (DEGREE + 2) / 2;

fn shading(why: impl Into<String>) -> CalibError {
    CalibError::Shading(why.into())
}

/// The sum of frames of one size, pixel by pixel.
#[derive(Debug, Clone)]
pub struct Accumulator {
    sum: Vec<u32>,
    width: usize,
    frames: u32,
}

impl Accumulator {
    /// No frames yet, of `width` × `height`.
    pub fn new(width: usize, height: usize) -> Self {
        Self {
            sum: vec![0; width * height],
            width,
            frames: 0,
        }
    }

    /// Adds `frame`.
    pub fn add(&mut self, frame: &ImageView<'_, u16>) -> Result<(), CalibError> {
        if frame.width() != self.width || frame.width() * frame.height() != self.sum.len() {
            return Err(shading("the frames differ in size"));
        }
        for (sums, row) in self
            .sum
            .chunks_exact_mut(self.width.max(1))
            .zip(frame.rows())
        {
            sums.iter_mut()
                .zip(row)
                .for_each(|(sum, &v)| *sum += u32::from(v));
        }
        self.frames += 1;
        Ok(())
    }

    /// The frames added.
    pub fn frames(&self) -> u32 {
        self.frames
    }

    /// Their mean.
    pub fn mean(&self) -> OwnedImage<f32> {
        let (n, width) = (self.frames.max(1) as f32, self.width.max(1));
        OwnedImage::from_fn(self.width, self.sum.len() / width, |x, y| {
            self.sum[y * width + x] as f32 / n
        })
    }

    /// Their mean, rounded, as the dark of frames of the whole sensor at `binning`.
    pub fn dark(&self, binning: Binning) -> Dark {
        let (n, width) = (self.frames.max(1), self.width.max(1));
        let image = OwnedImage::from_fn(self.width, self.sum.len() / width, |x, y| {
            ((self.sum[y * width + x] + n / 2) / n) as u16
        });
        Dark { image, binning }
    }
}

/// A dark frame: each pixel's offset, at the binning it was taken at, over the whole sensor.
#[derive(Debug, Clone)]
pub struct Dark {
    /// The offsets.
    pub image: OwnedImage<u16>,
    /// The binning.
    pub binning: Binning,
}

impl Dark {
    /// Its mean level.
    pub fn level(&self) -> f64 {
        let view = self.image.view();
        let sum: u64 = view.rows().flatten().map(|&v| u64::from(v)).sum();
        sum as f64 / (view.width() * view.height()).max(1) as f64
    }
}

/// How the light falls across the sensor, relative to its mean.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FlatField {
    /// The sensor's width and height, in pixels.
    pub sensor: [u32; 2],
    /// The coefficients of `xⁱ yʲ`, `i + j ≤ DEGREE`, `j` major, with `x` and `y` from −1 to 1
    /// across the sensor.
    pub coefficients: Vec<f64>,
}

/// `xⁱ yʲ` for each term.
fn basis(x: f64, y: f64) -> [f64; TERMS] {
    let mut terms = [0.0; TERMS];
    let (mut k, mut yj) = (0, 1.0);
    for j in 0..=DEGREE {
        let mut xi = yj;
        for _ in 0..=DEGREE - j {
            terms[k] = xi;
            (k, xi) = (k + 1, xi * x);
        }
        yj *= y;
    }
    terms
}

/// Solves `a·x = b` for a symmetric positive `a`, by elimination with partial pivoting.
fn solve(mut a: Vec<[f64; TERMS]>, mut b: [f64; TERMS]) -> Option<[f64; TERMS]> {
    for col in 0..TERMS {
        let pivot = (col..TERMS).max_by(|&i, &j| a[i][col].abs().total_cmp(&a[j][col].abs()))?;
        if a[pivot][col].abs() < 1e-12 {
            return None;
        }
        a.swap(col, pivot);
        b.swap(col, pivot);
        let (above, below) = a.split_at_mut(col + 1);
        let top = &above[col];
        for (offset, row) in below.iter_mut().enumerate() {
            let factor = row[col] / top[col];
            row[col..]
                .iter_mut()
                .zip(&top[col..])
                .for_each(|(a, t)| *a -= factor * t);
            b[col + 1 + offset] -= factor * b[col];
        }
    }
    let mut x = [0.0; TERMS];
    for row in (0..TERMS).rev() {
        let rest: f64 = (row + 1..TERMS).map(|k| a[row][k] * x[k]).sum();
        x[row] = (b[row] - rest) / a[row][row];
    }
    Some(x)
}

/// `image` shrunk by area to at most [`GRID`] cells a side.
fn shrink<T: siq_imgproc::Sample + Default>(image: &ImageView<'_, T>) -> OwnedImage<T> {
    let (width, height) = (image.width(), image.height());
    let step = width.max(height).div_ceil(GRID).max(1);
    let mut grid = OwnedImage::new(width.div_ceil(step), height.div_ceil(step));
    downsample_area(image, &mut grid.view_mut()).expect("a smaller grid");
    grid
}

impl FlatField {
    /// The flat of `image`, the light measured over `area` of a sensor of `sensor` pixels, less
    /// `dark`.
    pub fn fit(
        image: &ImageView<'_, f32>,
        area: Roi,
        sensor: [u32; 2],
        dark: f32,
    ) -> Result<Self, CalibError> {
        let grid = shrink(image);
        let view = grid.view();
        let (columns, rows) = (view.width(), view.height());
        if columns < 4 || rows < 4 {
            return Err(shading("the frame is too small to fit a flat to"));
        }
        let (mut a, mut b) = (vec![[0.0; TERMS]; TERMS], [0.0; TERMS]);
        let at = |n: usize, count: usize, offset: u32, extent: u32| {
            f64::from(offset) + (n as f64 + 0.5) * f64::from(extent) / count as f64
        };
        let (w, h) = (f64::from(sensor[0]), f64::from(sensor[1]));
        for (row, values) in view.rows().enumerate() {
            let y = at(row, rows, area.offset_y, area.height) * 2.0 / h - 1.0;
            for (column, &value) in values.iter().enumerate() {
                let x = at(column, columns, area.offset_x, area.width) * 2.0 / w - 1.0;
                let terms = basis(x, y);
                for (i, row) in a.iter_mut().enumerate() {
                    row.iter_mut()
                        .zip(&terms)
                        .for_each(|(a, t)| *a += terms[i] * t);
                    b[i] += terms[i] * f64::from(value - dark);
                }
            }
        }
        let coefficients = solve(a, b).ok_or_else(|| shading("the flat can't be fitted"))?;
        let mut flat = Self {
            sensor,
            coefficients: coefficients.to_vec(),
        };
        let [low, high, mean] = flat.spread(area);
        if mean <= 0.0 || low <= 0.05 * mean || !high.is_finite() {
            return Err(shading("too little light reached the sensor for a flat"));
        }
        flat.coefficients.iter_mut().for_each(|c| *c /= mean);
        Ok(flat)
    }

    /// The light at sensor pixel position `(x, y)`, relative to the mean.
    pub fn at(&self, x: f64, y: f64) -> f64 {
        let (w, h) = (f64::from(self.sensor[0]), f64::from(self.sensor[1]));
        let terms = basis(x * 2.0 / w - 1.0, y * 2.0 / h - 1.0);
        terms
            .iter()
            .zip(&self.coefficients)
            .map(|(t, c)| t * c)
            .sum()
    }

    /// The least, greatest and mean light over `area`, on a grid of 64 × 64 places.
    fn spread(&self, area: Roi) -> [f64; 3] {
        let place = |n: u32, offset: u32, extent: u32| {
            f64::from(offset) + (f64::from(n) + 0.5) * f64::from(extent) / 64.0
        };
        let (mut low, mut high, mut sum) = (f64::MAX, f64::MIN, 0.0);
        for j in 0..64 {
            for i in 0..64 {
                let light = self.at(
                    place(i, area.offset_x, area.width),
                    place(j, area.offset_y, area.height),
                );
                (low, high, sum) = (low.min(light), high.max(light), sum + light);
            }
        }
        [low, high, sum / 4096.0]
    }

    /// The least and greatest light over the sensor: the corners' fall-off, and the brightest
    /// place.
    pub fn range(&self) -> [f64; 2] {
        let [low, high, _] = self.spread(Roi::full(self.sensor[0], self.sensor[1]));
        [low, high]
    }

    /// The gains, `1 / flat`, of the pixels of a frame of `size` over `roi` at `binning`.
    pub fn gains(&self, roi: Roi, binning: Binning, size: (usize, usize)) -> OwnedImage<f32> {
        let (bx, by) = binning.factors();
        let origin = [f64::from(roi.offset_x), f64::from(roi.offset_y)];
        self.gains_on(origin, [f64::from(bx), f64::from(by)], size)
    }

    /// The gains of a grid of `size` pixels, each `step` sensor pixels a side, from `origin` on the
    /// sensor: a binned frame's, or a preview's.
    pub fn gains_on(
        &self,
        origin: [f64; 2],
        step: [f64; 2],
        (width, height): (usize, usize),
    ) -> OwnedImage<f32> {
        let (w, h) = (f64::from(self.sensor[0]), f64::from(self.sensor[1]));
        let mut gains = OwnedImage::new(width, height);
        for_each_row(&mut gains.view_mut(), |row, line| {
            let y = (origin[1] + (row as f64 + 0.5) * step[1]) * 2.0 / h - 1.0;
            // The polynomial in x along this row.
            let mut along = [0.0; DEGREE + 1];
            let (mut k, mut yj) = (0, 1.0);
            for j in 0..=DEGREE {
                for slot in along.iter_mut().take(DEGREE + 1 - j) {
                    (*slot, k) = (*slot + self.coefficients[k] * yj, k + 1);
                }
                yj *= y;
            }
            for (column, gain) in line.iter_mut().enumerate() {
                let x = (origin[0] + (column as f64 + 0.5) * step[0]) * 2.0 / w - 1.0;
                let light = along.iter().rev().fold(0.0, |sum, c| sum * x + c);
                *gain = (1.0 / light.max(0.05)) as f32;
            }
        });
        gains
    }
}

/// The flat of frames of different places (BaSiC, simplified), a frame at a time: each frame,
/// less the dark, shrunk and divided by its own mean, so that a darker place weighs as a brighter
/// one; then the median of each cell across the frames, which the sample's features at any one
/// place don't move, fitted.
#[derive(Debug, Clone)]
pub struct Estimator {
    dark: f32,
    size: (usize, usize),
    grids: Vec<Vec<f32>>,
}

impl Estimator {
    /// No frames yet, to be measured less `dark`.
    pub fn new(dark: f32) -> Self {
        Self {
            dark,
            size: (0, 0),
            grids: Vec::new(),
        }
    }

    /// Adds `frame`, of the same size as the others.
    pub fn add(&mut self, frame: &ImageView<'_, u16>) -> Result<(), CalibError> {
        let grid = shrink(frame);
        let view = grid.view();
        let size = (view.width(), view.height());
        if !self.grids.is_empty() && size != self.size {
            return Err(shading("the frames differ in size"));
        }
        let values: Vec<f32> = view
            .rows()
            .flatten()
            .map(|&v| f32::from(v) - self.dark)
            .collect();
        let mean = values.iter().sum::<f32>() / values.len().max(1) as f32;
        if mean <= 0.0 {
            return Err(shading("a frame holds no light above the dark"));
        }
        self.size = size;
        self.grids
            .push(values.into_iter().map(|v| v / mean).collect());
        Ok(())
    }

    /// The frames added.
    pub fn frames(&self) -> usize {
        self.grids.len()
    }

    /// The flat, of frames each over `area` of a sensor of `sensor` pixels: of five frames or
    /// more.
    pub fn finish(&self, area: Roi, sensor: [u32; 2]) -> Result<FlatField, CalibError> {
        if self.grids.len() < 5 {
            return Err(shading("an estimate needs five frames or more"));
        }
        let (width, mut column) = (self.size.0, Vec::with_capacity(self.grids.len()));
        let median = OwnedImage::from_fn(width, self.size.1, |x, y| {
            column.clear();
            column.extend(self.grids.iter().map(|values| values[y * width + x]));
            let middle = column.len() / 2;
            *column.select_nth_unstable_by(middle, f32::total_cmp).1
        });
        FlatField::fit(&median.view(), area, sensor, 0.0)
    }
}

/// The correction of frames of one geometry, made once and applied to each frame as it is read:
/// `(raw − dark) × gain`.
#[derive(Debug, Clone)]
pub struct FlatFieldView {
    dark: Option<OwnedImage<u16>>,
    gains: Option<OwnedImage<f32>>,
}

impl FlatFieldView {
    /// For frames of `meta`'s region and binning: `dark`, cropped to the region if it was taken
    /// at their binning or as its mean level if not, and `flat`'s gains.
    pub fn new(meta: &FrameMeta, dark: Option<&Dark>, flat: Option<&FlatField>) -> Self {
        let size = meta.image_size();
        let size = (size.0 as usize, size.1 as usize);
        let dark = dark.map(|dark| {
            let (bx, by) = meta.binning.factors();
            let (x, y) = (
                meta.roi.offset_x / u32::from(bx),
                meta.roi.offset_y / u32::from(by),
            );
            let rect = RectPx::new(x, y, size.0 as u32, size.1 as u32);
            match dark.image.view().sub_view(rect) {
                Ok(view) if dark.binning == meta.binning => {
                    OwnedImage::from_fn(size.0, size.1, |i, j| view.row(j)[i])
                }
                _ => {
                    let level = dark.level().round() as u16;
                    OwnedImage::from_fn(size.0, size.1, |_, _| level)
                }
            }
        });
        let gains = flat.map(|flat| flat.gains(meta.roi, meta.binning, size));
        Self { dark, gains }
    }

    /// `frame` corrected, clamped to `max`, such as 16383 for 14-bit pixels.
    pub fn read(
        &self,
        frame: &ImageView<'_, u16>,
        max: u16,
    ) -> Result<OwnedImage<u16>, CalibError> {
        let calibration = Calibration {
            dark: self.dark.as_ref().map(OwnedImage::view),
            gain: self.gains.as_ref().map(OwnedImage::view),
        };
        let mut out = OwnedImage::new(frame.width(), frame.height());
        correct(frame, &calibration, max, &mut out.view_mut())
            .map_err(|error| shading(error.to_string()))?;
        Ok(out)
    }
}

/// What a shading calibration is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ShadingKind {
    /// Each pixel's offset, taken with no light: the data beside it, `dark.npy`.
    Dark,
    /// How the light falls across the sensor.
    Flat,
}

/// A dark or a flat as it is kept, for one objective and channel.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ShadingRecord {
    /// Its id.
    pub id: CalibrationId,
    /// A dark or a flat.
    pub kind: ShadingKind,
    /// The objective it is for.
    pub objective: Option<ObjectiveId>,
    /// The channel it is for.
    pub channel: Option<ChannelId>,
    /// When it was made.
    pub created: DateTime<Utc>,
    /// How it was made: `capture`, or `estimate` from a dataset's frames.
    pub method: String,
    /// The frames it was made from.
    pub frames: usize,
    /// Their exposure, in ms.
    pub exposure_ms: f64,
    /// Their binning.
    pub binning: Binning,
    /// A dark's mean level, or the dark level a flat was measured less, in ADU.
    pub level: f64,
    /// A flat's surface.
    pub flat: Option<FlatField>,
    /// A flat's least and greatest light, relative to its mean.
    pub range: Option<[f64; 2]>,
}

/// The shading calibrations.
pub type ShadingRegistry = Registry<ShadingRecord>;

impl Versioned for ShadingRecord {
    type Key = (Option<ObjectiveId>, Option<ChannelId>, ShadingKind);
    const FOLDER: &'static str = "shading";

    fn id(&self) -> CalibrationId {
        self.id
    }

    fn created(&self) -> DateTime<Utc> {
        self.created
    }

    fn key(&self) -> Self::Key {
        (self.objective, self.channel, self.kind)
    }

    fn key_folder(&self) -> String {
        let id = |id: Option<String>| id.unwrap_or_else(|| "none".to_owned());
        let objective = id(self.objective.map(|objective| objective.to_string()));
        format!(
            "objective-{objective}_channel-{}",
            id(self.channel.map(|channel| channel.to_string()))
        )
    }
}

impl ShadingRegistry {
    /// Keeps `dark`, taken under `meta`'s settings from `frames` frames, for `objective` and
    /// `channel`: its record.
    pub fn save_dark(
        &self,
        (objective, channel): (Option<ObjectiveId>, Option<ChannelId>),
        dark: &Dark,
        meta: &FrameMeta,
        frames: usize,
    ) -> Result<ShadingRecord, CalibError> {
        let record = ShadingRecord {
            kind: ShadingKind::Dark,
            level: dark.level(),
            ..ShadingRecord::new((objective, channel), meta, frames, "capture")
        };
        self.save(
            &record,
            &[("dark.npy", &crate::npy::write(&dark.image.view()))],
        )?;
        Ok(record)
    }

    /// The dark of `record`.
    pub fn dark(&self, record: &ShadingRecord) -> Result<Dark, CalibError> {
        let path = self.path(record, "dark.npy");
        let bytes = std::fs::read(&path).map_err(|e| crate::registry::storage(&path, e))?;
        let image = crate::npy::read(&bytes).map_err(|e| crate::registry::storage(&path, e))?;
        Ok(Dark {
            image,
            binning: record.binning,
        })
    }
}

impl ShadingRecord {
    /// A flat or dark of `objective` and `channel`, made now from `frames` frames under `meta`'s
    /// settings by `method`: a flat, until it is given its kind.
    pub fn new(
        (objective, channel): (Option<ObjectiveId>, Option<ChannelId>),
        meta: &FrameMeta,
        frames: usize,
        method: &str,
    ) -> Self {
        Self {
            id: CalibrationId::generate(),
            kind: ShadingKind::Flat,
            objective,
            channel,
            created: Utc::now(),
            method: method.to_owned(),
            frames,
            exposure_ms: meta.exposure.0,
            binning: meta.binning,
            level: 0.0,
            flat: None,
            range: None,
        }
    }

    /// The flat record of `flat`, measured less `level`.
    pub fn with_flat(self, flat: FlatField, level: f64) -> Self {
        Self {
            range: Some(flat.range()),
            flat: Some(flat),
            level,
            ..self
        }
    }
}
