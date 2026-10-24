//! 2D affine transforms, and the typed mapping between the stage frame and a frame's pixels.
//!
//! [`Affine2`] is unit-free: `p' = A·p + t` on `[f64; 2]`. [`StageToPixel`] and [`PixelToStage`]
//! wrap one for the commonest use, with typed points:
//!
//! ```text
//! sensor_px = sensor_centre + L·(S − P)          (L: the calibration, px per µm)
//! image_px  = (sensor_px − roi_offset) / binning
//! ```
//!
//! for a sample point `S` in a frame taken with the stage at `P` (ADR-0018).

#[cfg(test)]
mod tests;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::geometry::{PointPx, Vec2Um};
use crate::image::{Binning, Roi};

/// A 2D affine transform, `p' = A·p + t`, stored as the rows `[a, b, tx]` and `[c, d, ty]`.
///
/// It serializes as those two rows: `[[a, b, tx], [c, d, ty]]`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct Affine2 {
    rows: [[f64; 3]; 2],
}

/// The parameters of an affine transform: `A = R(rotation) · Shear(shear) · Scale(scale_x, scale_y)`.
///
/// `R(θ) = [[cos θ, −sin θ], [sin θ, cos θ]]`, `Shear(k) = [[1, k], [0, 1]]`, and
/// `Scale(sx, sy) = [[sx, 0], [0, sy]]`. [`Affine2::decompose`] always gives a positive
/// `scale_x`; a reflection shows as a negative `scale_y`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AffineParams {
    /// Scale along the first axis.
    pub scale_x: f64,
    /// Scale along the second axis; negative for a reflection.
    pub scale_y: f64,
    /// Rotation, in radians.
    pub rotation: f64,
    /// How far the second axis leans along the first.
    pub shear: f64,
    /// Translation, `t`.
    pub translation: [f64; 2],
}

impl Affine2 {
    /// The identity.
    pub const IDENTITY: Self = Self::new([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]);

    /// A transform from its rows `[a, b, tx]` and `[c, d, ty]`.
    pub const fn new(rows: [[f64; 3]; 2]) -> Self {
        Self { rows }
    }

    /// The rows `[a, b, tx]` and `[c, d, ty]`.
    pub const fn rows(&self) -> [[f64; 3]; 2] {
        self.rows
    }

    /// A translation by `[tx, ty]`.
    pub const fn translation([tx, ty]: [f64; 2]) -> Self {
        Self::new([[1.0, 0.0, tx], [0.0, 1.0, ty]])
    }

    /// A scaling of each axis.
    pub const fn scale(scale_x: f64, scale_y: f64) -> Self {
        Self::new([[scale_x, 0.0, 0.0], [0.0, scale_y, 0.0]])
    }

    /// The transform with these parameters.
    pub fn from_params(params: &AffineParams) -> Self {
        let (sin, cos) = params.rotation.sin_cos();
        let (sx, sy, k) = (params.scale_x, params.scale_y, params.shear);
        let [tx, ty] = params.translation;
        Self::new([
            [cos * sx, (cos * k - sin) * sy, tx],
            [sin * sx, (sin * k + cos) * sy, ty],
        ])
    }

    /// The parameters of this transform, or `None` if it is singular.
    pub fn decompose(&self) -> Option<AffineParams> {
        self.invert()?;
        let [[a, b, tx], [c, d, ty]] = self.rows;
        let rotation = c.atan2(a);
        let (sin, cos) = rotation.sin_cos();
        // R(−θ)·A = [[sx, k·sy], [0, sy]].
        let scale_y = cos * d - sin * b;
        Some(AffineParams {
            scale_x: a.hypot(c),
            scale_y,
            rotation,
            shear: (cos * b + sin * d) / scale_y,
            translation: [tx, ty],
        })
    }

    /// Maps a point.
    pub fn apply(&self, [x, y]: [f64; 2]) -> [f64; 2] {
        let [[a, b, tx], [c, d, ty]] = self.rows;
        [a * x + b * y + tx, c * x + d * y + ty]
    }

    /// Maps a displacement: the linear part only, without the translation.
    pub fn apply_vector(&self, [x, y]: [f64; 2]) -> [f64; 2] {
        let [[a, b, _], [c, d, _]] = self.rows;
        [a * x + b * y, c * x + d * y]
    }

    /// The linear part, with the translation removed.
    pub fn linear(&self) -> Self {
        let [[a, b, _], [c, d, _]] = self.rows;
        Self::new([[a, b, 0.0], [c, d, 0.0]])
    }

    /// The determinant of the linear part. It is negative for a reflection.
    pub fn determinant(&self) -> f64 {
        let [[a, b, _], [c, d, _]] = self.rows;
        a * d - b * c
    }

    /// `self ∘ inner`: the transform that applies `inner` first, then `self`.
    pub fn compose(&self, inner: &Self) -> Self {
        let [[a, b, tx], [c, d, ty]] = self.rows;
        let [[e, f, ux], [g, h, uy]] = inner.rows;
        Self::new([
            [a * e + b * g, a * f + b * h, a * ux + b * uy + tx],
            [c * e + d * g, c * f + d * h, c * ux + d * uy + ty],
        ])
    }

    /// The inverse, or `None` if the transform is singular or not finite.
    pub fn invert(&self) -> Option<Self> {
        let [[a, b, tx], [c, d, ty]] = self.rows;
        let det = self.determinant();
        // Singular relative to the matrix's own size.
        let threshold = (a.abs() + b.abs()) * (c.abs() + d.abs()) * 1e-12;
        let finite = self.rows.as_flattened().iter().all(|v| v.is_finite());
        if !finite || !det.is_finite() || det.abs() <= threshold {
            return None;
        }
        let (ia, ib, ic, id) = (d / det, -b / det, -c / det, a / det);
        Some(Self::new([
            [ia, ib, -(ia * tx + ib * ty)],
            [ic, id, -(ic * tx + id * ty)],
        ]))
    }
}

/// The mapping from the stage frame (µm) to a frame's image pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StageToPixel(pub Affine2);

/// The mapping from a frame's image pixels to the stage frame (µm).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PixelToStage(pub Affine2);

impl StageToPixel {
    /// The mapping for a frame taken with the stage at `stage`.
    ///
    /// - `calibration` is the linear stage-to-sensor transform, in px per µm. Its translation is
    ///   ignored.
    /// - `sensor_centre` is the optical axis on the full, unbinned sensor, usually its centre.
    /// - `roi` and `binning` are the frame's.
    pub fn for_frame(
        calibration: &Affine2,
        stage: Vec2Um,
        sensor_centre: PointPx,
        roi: Roi,
        binning: Binning,
    ) -> Self {
        let to_sensor = Affine2::translation([
            sensor_centre.x - f64::from(roi.offset_x),
            sensor_centre.y - f64::from(roi.offset_y),
        ])
        .compose(&calibration.linear())
        .compose(&Affine2::translation((-stage).to_um()));
        let (horizontal, vertical) = binning.factors();
        let to_image = Affine2::scale(1.0 / f64::from(horizontal), 1.0 / f64::from(vertical));
        Self(to_image.compose(&to_sensor))
    }

    /// Where `point` appears in the image.
    pub fn apply(&self, point: Vec2Um) -> PointPx {
        let [x, y] = self.0.apply(point.to_um());
        PointPx::new(x, y)
    }

    /// The inverse mapping, or `None` if this one is singular.
    pub fn inverse(&self) -> Option<PixelToStage> {
        self.0.invert().map(PixelToStage)
    }
}

impl PixelToStage {
    /// The stage-frame position of `point`.
    pub fn apply(&self, point: PointPx) -> Vec2Um {
        let [x, y] = self.0.apply(point.to_array());
        Vec2Um::from_um(x, y)
    }

    /// The inverse mapping, or `None` if this one is singular.
    pub fn inverse(&self) -> Option<StageToPixel> {
        self.0.invert().map(StageToPixel)
    }
}
