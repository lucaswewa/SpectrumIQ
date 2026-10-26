//! Calibration (ADR-0049): the stage–camera affine of an objective, fitted from known stage moves
//! and the image shifts they cause, its backlash, and the registry that versions the results.
//! Pure: the routine that moves the stage and takes the frames is `siq-workflow`'s.
//!
//! - **The model** (architecture §13, ADR-0018): a point at stage position `S` appears at sensor
//!   pixel `c + L·(S − P)` with the stage at `P`. Moving the stage by `m` moves the image's
//!   content by `−L·m`. `L`, in pixels per µm, holds the pixel size, the camera's rotation, and any
//!   scale difference or shear between the axes.
//! - **[`fit`]** solves `L` by least squares from the moves and the shifts measured, with an
//!   offset common to them all (the reference frame's own position error), and each move's
//!   residual on the stage, in µm. Moves that don't span the plane are refused.
//! - **[`backlash`]** turns the shift between arriving at one place from either side into µm.
//! - **[`CalibrationRecord`]** is a calibration as it is kept, and the [`CalibrationRegistry`]
//!   keeps every one, in force or not.

mod registry;

use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use siq_types::{Affine2, CalibrationId, ObjectiveId};

pub use registry::CalibrationRegistry;

/// A stage move and the image shift it caused.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Move {
    /// The move, as the stage reports it, in µm: `x`, `y`.
    pub stage: [f64; 2],
    /// The image's content's shift, in sensor pixels: `x`, `y`.
    pub shift: [f64; 2],
}

/// The affine fitted from moves.
#[derive(Debug, Clone, PartialEq)]
pub struct Fit {
    /// `L`, stage µm to sensor pixels, with no translation.
    pub affine: Affine2,
    /// Each move's residual on the stage, in µm: the move reported, less the move its shift shows.
    pub residuals: Vec<[f64; 2]>,
}

impl Fit {
    /// The residuals' root mean square, in µm.
    pub fn residual_rms(&self) -> f64 {
        let squares: f64 = self.residuals.iter().map(|[x, y]| x * x + y * y).sum();
        (squares / self.residuals.len().max(1) as f64).sqrt()
    }

    /// The largest residual, in µm.
    pub fn residual_max(&self) -> f64 {
        self.residuals
            .iter()
            .map(|[x, y]| x.hypot(*y))
            .fold(0.0, f64::max)
    }

    /// The mean pixel size on the sample, in µm.
    pub fn pixel_size(&self) -> f64 {
        1.0 / self.affine.determinant().abs().sqrt()
    }

    /// Whether the fit meets `thresholds`: why not, if it doesn't.
    pub fn check(&self, thresholds: &Thresholds) -> Result<(), CalibError> {
        let reject = |why: String| Err(CalibError::Rejected(why));
        if self.residuals.len() < thresholds.min_moves {
            return reject(format!(
                "{} moves, of the {} needed",
                self.residuals.len(),
                thresholds.min_moves
            ));
        }
        if self.residual_rms() > thresholds.max_residual_rms_um {
            return reject(format!(
                "the residuals are {:.2} µm RMS, over {}",
                self.residual_rms(),
                thresholds.max_residual_rms_um
            ));
        }
        if self.residual_max() > thresholds.max_residual_um {
            return reject(format!(
                "a residual is {:.2} µm, over {}",
                self.residual_max(),
                thresholds.max_residual_um
            ));
        }
        Ok(())
    }
}

/// What a calibration must meet to be kept.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Thresholds {
    /// The fewest moves.
    pub min_moves: usize,
    /// The largest residuals' RMS, in µm.
    pub max_residual_rms_um: f64,
    /// The largest residual, in µm.
    pub max_residual_um: f64,
}

impl Default for Thresholds {
    /// Six moves, 1.5 µm RMS and 4 µm at most: on the stage, as its repeatability, not the
    /// registration's, bounds the residuals at any magnification but the lowest. A stage
    /// repeatable to a micrometre passes; a registration astray, tens of pixels off, doesn't.
    fn default() -> Self {
        Self {
            min_moves: 6,
            max_residual_rms_um: 1.5,
            max_residual_um: 4.0,
        }
    }
}

/// Why a calibration failed.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum CalibError {
    /// The moves don't span the plane: too few, too short, or all along one line.
    #[error("the moves don't span the plane: {0}")]
    Degenerate(String),
    /// The fit doesn't meet the thresholds.
    #[error("the calibration is refused: {0}")]
    Rejected(String),
    /// A calibration couldn't be read or kept.
    #[error("{0}")]
    Storage(String),
}

/// `L` from `moves` by least squares: `shift = t − L·stage` for each, `t` common to them all.
pub fn fit(moves: &[Move]) -> Result<Fit, CalibError> {
    if moves.len() < 3 {
        return Err(CalibError::Degenerate(format!(
            "{} moves, of at least 3",
            moves.len()
        )));
    }
    let n = moves.len() as f64;
    let mean = |of: fn(&Move) -> [f64; 2]| {
        let [x, y] = moves
            .iter()
            .map(of)
            .fold([0.0; 2], |[a, b], [x, y]| [a + x, b + y]);
        [x / n, y / n]
    };
    let (m0, s0) = (mean(|m| m.stage), mean(|m| m.shift));
    // About the means, L = −(Σ s·mᵀ)(Σ m·mᵀ)⁻¹.
    let (mut mm, mut sm) = ([[0.0; 2]; 2], [[0.0; 2]; 2]);
    for Move { stage, shift } in moves {
        let m = [stage[0] - m0[0], stage[1] - m0[1]];
        let s = [shift[0] - s0[0], shift[1] - s0[1]];
        for i in 0..2 {
            for j in 0..2 {
                mm[i][j] += m[i] * m[j];
                sm[i][j] += s[i] * m[j];
            }
        }
    }
    let det = mm[0][0] * mm[1][1] - mm[0][1] * mm[1][0];
    let trace = mm[0][0] + mm[1][1];
    // The smaller eigenvalue of Σ m·mᵀ against the larger: near 0 for moves along one line.
    let spread = (trace * trace - 4.0 * det).max(0.0).sqrt();
    let (small, large) = ((trace - spread) / 2.0, (trace + spread) / 2.0);
    if large <= 0.0 || small < 1e-3 * large {
        return Err(CalibError::Degenerate(
            "they lie along one line, or don't move".to_owned(),
        ));
    }
    let inverse = [
        [mm[1][1] / det, -mm[0][1] / det],
        [-mm[1][0] / det, mm[0][0] / det],
    ];
    let l = |i: usize, j: usize| -(sm[i][0] * inverse[0][j] + sm[i][1] * inverse[1][j]);
    let affine = Affine2::new([[l(0, 0), l(0, 1), 0.0], [l(1, 0), l(1, 1), 0.0]]);
    let inverse = affine
        .invert()
        .ok_or_else(|| CalibError::Degenerate("the image doesn't follow the moves".to_owned()))?;
    let [lx, ly] = affine.apply_vector(m0);
    let offset = [s0[0] + lx, s0[1] + ly];
    let residuals = moves
        .iter()
        .map(|Move { stage, shift }| {
            let [x, y] = inverse.apply_vector([offset[0] - shift[0], offset[1] - shift[1]]);
            [stage[0] - x, stage[1] - y]
        })
        .collect();
    Ok(Fit { affine, residuals })
}

/// The backlash on each stage axis, in µm, from `shift`: how far the image of a place arrived at
/// moving toward −x and −y is from the one arrived at moving toward +x and +y.
pub fn backlash(fit: &Fit, shift: [f64; 2]) -> [f64; 2] {
    let inverse = fit.affine.invert().unwrap_or(Affine2::IDENTITY);
    inverse.apply_vector(shift).map(f64::abs)
}

/// A calibration as it is kept: the affine of one objective, and how well it was measured.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CalibrationRecord {
    /// Its id, which frames taken under it carry.
    pub id: CalibrationId,
    /// The objective it is for.
    pub objective: ObjectiveId,
    /// When it was made.
    pub created: DateTime<Utc>,
    /// `L`, stage µm to sensor pixels: rows `[a, b]` and `[c, d]`.
    pub affine: [[f64; 2]; 2],
    /// The mean pixel size on the sample, in µm.
    pub px_size_um: f64,
    /// The camera's rotation against the stage, in degrees.
    pub rotation_deg: f64,
    /// The pixels per µm along the image's two axes.
    pub scale: [f64; 2],
    /// How far the image's second axis leans along its first.
    pub shear: f64,
    /// The moves' residuals' RMS on the stage, in µm.
    pub residual_rms_um: f64,
    /// The largest residual, in µm.
    pub residual_max_um: f64,
    /// The stage's backlash on `x` and `y`, in µm, if it was measured.
    pub backlash_um: Option<[f64; 2]>,
    /// The moves it was fitted from.
    pub moves: usize,
    /// How it was measured, such as `stage_camera`.
    pub method: String,
}

impl CalibrationRecord {
    /// The record of `fit`, for `objective`, made now.
    pub fn new(
        objective: ObjectiveId,
        fit: &Fit,
        backlash: Option<[f64; 2]>,
        method: &str,
    ) -> Self {
        let [[a, b, _], [c, d, _]] = fit.affine.rows();
        let params = fit.affine.decompose();
        Self {
            id: CalibrationId::generate(),
            objective,
            created: Utc::now(),
            affine: [[a, b], [c, d]],
            px_size_um: fit.pixel_size(),
            rotation_deg: params.map_or(0.0, |p| p.rotation.to_degrees()),
            scale: params.map_or([0.0; 2], |p| [p.scale_x, p.scale_y]),
            shear: params.map_or(0.0, |p| p.shear),
            residual_rms_um: fit.residual_rms(),
            residual_max_um: fit.residual_max(),
            backlash_um: backlash,
            moves: fit.residuals.len(),
            method: method.to_owned(),
        }
    }

    /// `L`, as an affine with no translation: what `StageToPixel::for_frame` takes.
    pub fn affine(&self) -> Affine2 {
        let [[a, b], [c, d]] = self.affine;
        Affine2::new([[a, b, 0.0], [c, d, 0.0]])
    }
}
