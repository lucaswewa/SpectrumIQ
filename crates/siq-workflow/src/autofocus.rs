//! Autofocus and focus surfaces (ADR-0054).
//!
//! - **Autofocus** steps the focus drive through a range about where it is, a frame at each, and
//!   scores each frame's sharpness over its middle three quarters: a coarse sweep over the range,
//!   then a fine one about the coarse sweep's estimate of focus. Each estimate is the vertex of a
//!   Gaussian fitted by least squares to the frames above half the sweep's peak, which their noise
//!   moves little. A coarse sweep whose sharpness rises by less than a share of its peak has
//!   nothing to focus on, and the drive goes back where it was. The camera is binned for the
//!   coarse sweep, at an exposure that keeps the light the same; the fine sweep is at its own
//!   resolution, where the little that the blur changes near focus shows.
//! - **A focus surface** is fitted to the focus found at places: a plane by least squares, with
//!   places far off it set aside, or a thin-plate spline through them. Each place's residual is
//!   the surface's error there, the spline's with the place left out of its fit.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use siq_devices::{Approach, CameraSettings, MoveOpts};
use siq_imgproc::{FocusMeasure, focus};
use siq_types::{Binning, Micrometers, Millis, RectPx};

use crate::DeviceSet;
use crate::routines::stage_camera::RoutineError;

/// How autofocus searches.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AutofocusPlan {
    /// The range searched, about where the focus is, in µm.
    pub range_um: f64,
    /// The coarse sweep's frames.
    pub coarse: u32,
    /// The fine sweep's frames, over one and a half coarse steps either side of the coarse focus.
    pub fine: u32,
    /// The least rise of sharpness over a coarse sweep, of its peak, with something to focus on.
    pub min_rise: f64,
    /// The binning of the coarse sweep's frames, if the camera has it.
    pub binning: u8,
}

impl Default for AutofocusPlan {
    /// 50 µm, 11 coarse frames and 13 fine, a rise of a fifth, binned 2 × 2.
    fn default() -> Self {
        Self {
            range_um: 50.0,
            coarse: 11,
            fine: 13,
            min_rise: 0.2,
            binning: 2,
        }
    }
}

/// What autofocus found.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Focused {
    /// Where the focus is, in µm: `None` with nothing to focus on.
    pub z: Option<f64>,
    /// Each frame's focus and sharpness, `[z, sharpness]`, in the order taken.
    pub curve: Vec<[f64; 2]>,
    /// How much the coarse sweep's sharpness rose, of its peak: the confidence.
    pub rise: f64,
}

/// The depth of field, in µm, of an objective of numerical aperture `na` in a medium of index
/// `n`, at `wavelength_um`, with pixels of `pixel_um` on the sample (Berek's).
pub fn depth_of_field(na: f64, n: f64, wavelength_um: f64, pixel_um: f64) -> f64 {
    wavelength_um * n / (na * na) + n * pixel_um / na
}

/// The vertex of the Gaussian fitted to `curve`'s points above half its peak, by least squares of
/// a parabola to their logarithms: the sharpest point's place if they don't make one.
fn vertex(curve: &[[f64; 2]]) -> f64 {
    let best = curve[sharpest(curve)];
    let above: Vec<[f64; 2]> = curve
        .iter()
        .copied()
        .filter(|p| p[1] >= best[1] / 2.0)
        .collect();
    let (mut a, mut b) = (vec![vec![0.0; 3]; 3], vec![0.0; 3]);
    for [z, m] in &above {
        let dz = z - best[0];
        let terms = [1.0, dz, dz * dz];
        for i in 0..3 {
            for j in 0..3 {
                a[i][j] += terms[i] * terms[j];
            }
            b[i] += terms[i] * m.max(1e-12).ln();
        }
    }
    let fitted = (above.len() >= 3).then(|| solve(a, b)).flatten();
    match fitted.as_deref() {
        Some([_, slope, curvature]) if *curvature < 0.0 => {
            let (low, high) = (above[0][0], above[above.len() - 1][0]);
            (best[0] - slope / (2.0 * curvature)).clamp(low.min(high), low.max(high))
        }
        _ => best[0],
    }
}

/// Autofocuses at the place the stage is, by `plan`, and leaves the focus drive at the focus
/// found, or where it was with nothing to focus on.
pub async fn autofocus(devices: &DeviceSet, plan: &AutofocusPlan) -> Result<Focused, RoutineError> {
    let (focus_drive, acquisition) = (&devices.focus, &devices.acquisition);
    let opts = MoveOpts {
        speed: None,
        approach: Approach::Unidirectional,
    };
    let start = focus_drive.status().borrow().position.0;
    let travel = focus_drive.limits();
    let within = |z: f64| z.clamp(travel.min.0, travel.max.0);
    let settings = devices.camera.settings();
    let b = plan.binning.max(1);
    let binned = Binning::new(b, b).unwrap_or(Binning::NONE);
    let fast = match devices.camera.capabilities().binnings.contains(&binned) {
        true => CameraSettings {
            binning: binned,
            exposure: Millis(settings.exposure.0 / f64::from(b) / f64::from(b)),
            ..settings
        },
        false => settings,
    };
    acquisition.configure(fast).await?;
    let found = async {
        let mut curve = Vec::new();
        let sweep = async |zs: Vec<f64>, curve: &mut Vec<[f64; 2]>| {
            for z in zs {
                focus_drive.move_to(Micrometers(z), opts).await?;
                let frame = acquisition.snap(false).await?;
                let image = frame.image::<u16>().ok_or(RoutineError::Format)?;
                let (w, h) = (image.width() as u32, image.height() as u32);
                let middle = image.sub_view(RectPx::new(w / 8, h / 8, w * 3 / 4, h * 3 / 4));
                let middle = middle.map_err(|error| RoutineError::Failed(error.to_string()))?;
                curve.push([z, focus(&middle, FocusMeasure::Tenengrad)]);
            }
            Ok::<_, RoutineError>(())
        };
        let steps = |from: f64, to: f64, n: u32| -> Vec<f64> {
            let n = n.max(2);
            (0..n)
                .map(|k| within(from + (to - from) * f64::from(k) / f64::from(n - 1)))
                .collect()
        };
        let half = plan.range_um / 2.0;
        sweep(steps(start - half, start + half, plan.coarse), &mut curve).await?;
        let (low, high) = curve
            .iter()
            .fold((f64::MAX, 0.0f64), |(l, h), p| (l.min(p[1]), h.max(p[1])));
        let rise = if high > 0.0 { (high - low) / high } else { 0.0 };
        if rise < plan.min_rise {
            focus_drive.move_to(Micrometers(start), opts).await?;
            return Ok(Focused {
                z: None,
                curve,
                rise,
            });
        }
        // Centred on the coarse sweep's own vertex: about its sharpest frame, a flat top would
        // leave the fine sweep lopsided, and its fit pulled to one side.
        let centre = vertex(&curve);
        let step = plan.range_um / f64::from(plan.coarse.max(2) - 1);
        // The fine sweep at the camera's own resolution: near focus, the blur hardly changes, and
        // binning would hide what it does.
        acquisition.configure(settings).await?;
        let fine_from = curve.len();
        sweep(
            steps(centre - 1.5 * step, centre + 1.5 * step, plan.fine),
            &mut curve,
        )
        .await?;
        let z = vertex(&curve[fine_from..]);
        focus_drive.move_to(Micrometers(within(z)), opts).await?;
        Ok(Focused {
            z: Some(within(z)),
            curve,
            rise,
        })
    }
    .await;
    acquisition.configure(settings).await?;
    found
}

/// The index of `curve`'s sharpest point.
fn sharpest(curve: &[[f64; 2]]) -> usize {
    (0..curve.len())
        .max_by(|&i, &j| curve[i][1].total_cmp(&curve[j][1]))
        .unwrap_or(0)
}

/// What a focus surface is fitted as.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SurfaceKind {
    /// A plane, by least squares, places more than three deviations off it set aside.
    Plane,
    /// A thin-plate spline through every place: a tilt and a curve both.
    #[default]
    ThinPlate,
}

/// The focus over the stage, from the focus found at places.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FocusSurface {
    /// `z = a + b·x + c·y`.
    Plane {
        /// `[a, b, c]`.
        coefficients: [f64; 3],
    },
    /// A thin-plate spline through the places, in coordinates about their centre over a scale.
    ThinPlate {
        /// The places' centre, in µm.
        centre: [f64; 2],
        /// The scale, in µm.
        scale: f64,
        /// The places, scaled.
        places: Vec<[f64; 2]>,
        /// The spline's weight at each place.
        weights: Vec<f64>,
        /// Its plane: `[a, b, c]`, in scaled coordinates.
        plane: [f64; 3],
    },
}

/// A surface fitted, and how well.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FittedSurface {
    /// The surface.
    pub surface: FocusSurface,
    /// Each place's residual, the focus found less the surface's, in µm.
    pub residuals: Vec<f64>,
    /// The places set aside, by index.
    pub outliers: Vec<usize>,
}

fn kernel(r2: f64) -> f64 {
    if r2 > 0.0 { r2 * r2.ln() / 2.0 } else { 0.0 }
}

impl FocusSurface {
    /// The focus at `(x, y)`, in µm.
    pub fn z(&self, x: f64, y: f64) -> f64 {
        match self {
            Self::Plane {
                coefficients: [a, b, c],
            } => a + b * x + c * y,
            Self::ThinPlate {
                centre,
                scale,
                places,
                weights,
                plane: [a, b, c],
            } => {
                let (u, v) = ((x - centre[0]) / scale, (y - centre[1]) / scale);
                let bend: f64 = places
                    .iter()
                    .zip(weights)
                    .map(|(p, w)| w * kernel((u - p[0]).powi(2) + (v - p[1]).powi(2)))
                    .sum();
                a + b * u + c * v + bend
            }
        }
    }
}

/// Solves `a·x = b` by elimination with partial pivoting.
fn solve(mut a: Vec<Vec<f64>>, mut b: Vec<f64>) -> Option<Vec<f64>> {
    let n = b.len();
    for col in 0..n {
        let pivot = (col..n).max_by(|&i, &j| a[i][col].abs().total_cmp(&a[j][col].abs()))?;
        if a[pivot][col].abs() < 1e-12 {
            return None;
        }
        a.swap(col, pivot);
        b.swap(col, pivot);
        for row in col + 1..n {
            let factor = a[row][col] / a[col][col];
            let top = a[col].clone();
            a[row]
                .iter_mut()
                .zip(&top)
                .skip(col)
                .for_each(|(x, t)| *x -= factor * t);
            b[row] -= factor * b[col];
        }
    }
    let mut x = vec![0.0; n];
    for row in (0..n).rev() {
        let rest: f64 = (row + 1..n).map(|k| a[row][k] * x[k]).sum();
        x[row] = (b[row] - rest) / a[row][row];
    }
    Some(x)
}

fn plane(points: &[[f64; 3]]) -> Option<[f64; 3]> {
    let (mut a, mut b) = (vec![vec![0.0; 3]; 3], vec![0.0; 3]);
    for [x, y, z] in points {
        let terms = [1.0, *x, *y];
        for i in 0..3 {
            for j in 0..3 {
                a[i][j] += terms[i] * terms[j];
            }
            b[i] += terms[i] * z;
        }
    }
    let [c0, c1, c2] = solve(a, b)?[..] else {
        return None;
    };
    Some([c0, c1, c2])
}

fn thin_plate(points: &[[f64; 3]]) -> Option<FocusSurface> {
    let n = points.len();
    let mean = |axis: usize| points.iter().map(|p| p[axis]).sum::<f64>() / n as f64;
    let centre = [mean(0), mean(1)];
    let scale = points
        .iter()
        .map(|p| (p[0] - centre[0]).hypot(p[1] - centre[1]))
        .fold(0.0, f64::max)
        .max(1e-9);
    let places: Vec<[f64; 2]> = points
        .iter()
        .map(|p| [(p[0] - centre[0]) / scale, (p[1] - centre[1]) / scale])
        .collect();
    let mut a = vec![vec![0.0; n + 3]; n + 3];
    let mut b = vec![0.0; n + 3];
    for i in 0..n {
        for j in 0..n {
            let d2 = (places[i][0] - places[j][0]).powi(2) + (places[i][1] - places[j][1]).powi(2);
            a[i][j] = kernel(d2);
        }
        let terms = [1.0, places[i][0], places[i][1]];
        for (k, term) in terms.into_iter().enumerate() {
            (a[i][n + k], a[n + k][i]) = (term, term);
        }
        b[i] = points[i][2];
    }
    let solution = solve(a, b)?;
    Some(FocusSurface::ThinPlate {
        centre,
        scale,
        places,
        weights: solution[..n].to_vec(),
        plane: [solution[n], solution[n + 1], solution[n + 2]],
    })
}

/// The surface of `kind` through `points`, `[x, y, z]` in µm: three or more, not on one line.
pub fn fit_surface(points: &[[f64; 3]], kind: SurfaceKind) -> Result<FittedSurface, RoutineError> {
    let degenerate =
        || RoutineError::Failed("a focus surface needs three places off one line".into());
    if points.len() < 3 {
        return Err(degenerate());
    }
    match kind {
        SurfaceKind::Plane => {
            let mut kept: Vec<usize> = (0..points.len()).collect();
            let mut coefficients = [0.0; 3];
            for _ in 0..3 {
                let chosen: Vec<[f64; 3]> = kept.iter().map(|&i| points[i]).collect();
                coefficients = plane(&chosen).ok_or_else(degenerate)?;
                let [a, b, c] = coefficients;
                let off = |p: &[f64; 3]| (p[2] - (a + b * p[0] + c * p[1])).abs();
                let mut deviations: Vec<f64> = chosen.iter().map(off).collect();
                deviations.sort_by(f64::total_cmp);
                // Three deviations of the median's, at least half a micrometre.
                let limit = (3.0 * 1.4826 * deviations[deviations.len() / 2]).max(0.5);
                let next: Vec<usize> = (0..points.len())
                    .filter(|&i| off(&points[i]) <= limit)
                    .collect();
                if next.len() < 3 || next == kept {
                    break;
                }
                kept = next;
            }
            let surface = FocusSurface::Plane { coefficients };
            let residuals = points
                .iter()
                .map(|p| p[2] - surface.z(p[0], p[1]))
                .collect();
            let outliers = (0..points.len()).filter(|i| !kept.contains(i)).collect();
            Ok(FittedSurface {
                surface,
                residuals,
                outliers,
            })
        }
        SurfaceKind::ThinPlate => {
            let surface = thin_plate(points).ok_or_else(degenerate)?;
            // Each place's residual with it left out, where three others remain.
            let residuals = (0..points.len())
                .map(|i| {
                    let others: Vec<[f64; 3]> = (0..points.len())
                        .filter(|&j| j != i)
                        .map(|j| points[j])
                        .collect();
                    let left_out = (others.len() >= 3).then(|| thin_plate(&others)).flatten();
                    left_out.map_or(0.0, |s| points[i][2] - s.z(points[i][0], points[i][1]))
                })
                .collect();
            Ok(FittedSurface {
                surface,
                residuals,
                outliers: Vec::new(),
            })
        }
    }
}
