//! The stage–camera calibration's measurement (ADR-0049):
//!
//! 1. A frame where the stage is, the reference.
//! 2. The stage moved to each of the plan's places about it, each approached from the same side, so
//!    the backlash is the same at every arrival; a frame at each, registered against the
//!    reference.
//! 3. One place approached from either side, a few times over, for the backlash: each pair of
//!    frames registered against each other.
//! 4. Back where it started, whether it worked or not.
//!
//! The moves are about where the stage is, or, if they would leave its travel there, about the
//! nearest place where they don't.
//!
//! Image shifts are given in sensor pixels: a binned frame's are scaled up. Registrations run on
//! blocking threads, as a frame of 100 MP takes a good part of a second.

use std::sync::Arc;

use siq_acquire::Acquisition;
use siq_calib::Move;
use siq_devices::{Approach, DeviceError, MoveOpts, Stage, Xyz};
use siq_register::{RegisterError, Registrar};
use siq_types::{Micrometers, OwnedImage};

/// Where to move, and what a registration must reach.
#[derive(Debug, Clone, PartialEq)]
pub struct StageCameraPlan {
    /// The places, from where the stage is, in µm: `x`, `y`.
    pub moves: Vec<[f64; 2]>,
    /// How the backlash is measured; `None` not to measure it.
    pub backlash: Option<BacklashPlan>,
    /// The least peak-to-correlation energy a registration may have: below it, the field is
    /// taken as too bare to measure.
    pub min_pce: f64,
}

/// Where the backlash is measured, and how.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BacklashPlan {
    /// The place, from where the stage is, in µm.
    pub at: [f64; 2],
    /// How far beyond it on each side the approaches start, in µm.
    pub reach: f64,
    /// How many times it is approached from either side: the shifts are averaged.
    pub repeats: usize,
}

impl StageCameraPlan {
    /// For a field of view of `field` µm: a star of 16 places, eight out to 0.4 of the field each
    /// way and eight out to 0.3, along the axes and the diagonals; the backlash at the start, from
    /// 50 µm either side, four times over. The farther the moves, the less the stage's own
    /// unrepeatability weighs: at 0.4, a diagonal move still leaves a third of the frame to
    /// register on.
    pub fn for_field([width, height]: [f64; 2]) -> Self {
        let directions = [
            [1.0, 0.0],
            [1.0, 1.0],
            [0.0, 1.0],
            [-1.0, 1.0],
            [-1.0, 0.0],
            [-1.0, -1.0],
            [0.0, -1.0],
            [1.0, -1.0],
        ];
        let moves = [0.4, 0.3]
            .into_iter()
            .flat_map(|share| directions.map(|[dx, dy]| [dx * width * share, dy * height * share]))
            .collect();
        Self {
            moves,
            backlash: Some(BacklashPlan {
                at: [0.0, 0.0],
                reach: 50.0,
                repeats: 4,
            }),
            min_pce: 100.0,
        }
    }
}

/// What the stage's moves did to the image.
#[derive(Debug, Clone, PartialEq)]
pub struct Measurement {
    /// Each move, with the image's shift.
    pub moves: Vec<Move>,
    /// Each registration's peak-to-correlation energy.
    pub pce: Vec<f64>,
    /// The shift of the place approached toward −x and −y from the one approached toward +x and
    /// +y, in sensor pixels, averaged over the repeats, if measured.
    pub backlash_shift: Option<[f64; 2]>,
}

/// Why the measurement stopped.
#[derive(Debug, thiserror::Error)]
pub enum RoutineError {
    /// The stage or the camera failed.
    #[error(transparent)]
    Device(#[from] DeviceError),
    /// Two frames couldn't be registered.
    #[error(transparent)]
    Register(#[from] RegisterError),
    /// A frame matched too poorly to measure from: the field is bare, or the move left it.
    #[error(
        "the frame at ({:.1}, {:.1}) µm matched with a PCE of {pce:.0}: is anything in the field?",
        at[0],
        at[1]
    )]
    Featureless {
        /// The move.
        at: [f64; 2],
        /// The registration's energy.
        pce: f64,
    },
    /// A frame isn't of 16-bit pixels.
    #[error("only 16-bit frames can be measured")]
    Format,
    /// A registration's thread failed.
    #[error("the registration failed: {0}")]
    Failed(String),
    /// The frames couldn't be summed or fitted.
    #[error(transparent)]
    Calib(#[from] siq_calib::CalibError),
    /// The moves, reaching this far each way in µm, don't fit in the stage's travel.
    #[error(
        "the moves reach {:.0} × {:.0} µm each way, more than the stage's travel holds",
        .0[0],
        .0[1]
    )]
    Travel([f64; 2]),
}

/// A frame, copied out of the pool, and its binning.
async fn snap(acquisition: &Acquisition) -> Result<(OwnedImage<u16>, [f64; 2]), RoutineError> {
    let frame = acquisition.snap(false).await?;
    let image = frame.image::<u16>().ok_or(RoutineError::Format)?;
    let copy = OwnedImage::from_fn(image.width(), image.height(), |x, y| image.row(y)[x]);
    let (horizontal, vertical) = frame.meta().binning.factors();
    Ok((copy, [f64::from(horizontal), f64::from(vertical)]))
}

/// Runs `plan` about where `stage` is, or the nearest place where its moves stay within the
/// stage's travel: the moves and the shifts they made. The stage goes back where it was.
pub async fn measure(
    acquisition: &Acquisition,
    stage: &dyn Stage,
    registrar: &Arc<Registrar>,
    plan: &StageCameraPlan,
) -> Result<Measurement, RoutineError> {
    let register = |a: Arc<OwnedImage<u16>>, b: OwnedImage<u16>| {
        let registrar = Arc::clone(registrar);
        async move {
            let found = tokio::task::spawn_blocking(move || registrar.register(a.view(), b.view()));
            found
                .await
                .map_err(|error| RoutineError::Failed(error.to_string()))?
                .map_err(RoutineError::from)
        }
    };
    let start = stage.status().borrow().position;
    let backlash = plan
        .backlash
        .map(|b| [b.at[0].abs() + b.reach, b.at[1].abs() + b.reach]);
    let reach = super::reach(plan.moves.iter().copied().chain(backlash));
    let centre = super::centre_within(start, reach, &stage.limits())?;
    let at = |[x, y]: [f64; 2]| Xyz {
        x: centre.x + Micrometers(x),
        y: centre.y + Micrometers(y),
        z: centre.z,
    };
    let same_side = MoveOpts {
        speed: None,
        approach: Approach::Unidirectional,
    };
    let direct = MoveOpts {
        approach: Approach::Direct,
        ..same_side
    };
    let measured = async {
        stage.move_to(at([0.0, 0.0]), same_side).await?;
        let reference = Arc::new(snap(acquisition).await?.0);
        let mut measurement = Measurement {
            moves: Vec::with_capacity(plan.moves.len()),
            pce: Vec::with_capacity(plan.moves.len()),
            backlash_shift: None,
        };
        for &place in &plan.moves {
            stage.move_to(at(place), same_side).await?;
            let (frame, [bx, by]) = snap(acquisition).await?;
            let found = register(Arc::clone(&reference), frame).await?;
            if found.pce < plan.min_pce {
                return Err(RoutineError::Featureless {
                    at: place,
                    pce: found.pce,
                });
            }
            tracing::debug!(?place, shift = ?found.shift, pce = found.pce, "stage–camera move");
            measurement.moves.push(Move {
                stage: place,
                shift: [found.shift[0] * bx, found.shift[1] * by],
            });
            measurement.pce.push(found.pce);
        }
        if let Some(BacklashPlan {
            at: [x, y],
            reach,
            repeats,
        }) = plan.backlash
        {
            let mut sum = [0.0; 2];
            for _ in 0..repeats {
                stage.move_to(at([x - reach, y - reach]), direct).await?;
                stage.move_to(at([x, y]), direct).await?;
                let (rising, [bx, by]) = snap(acquisition).await?;
                stage.move_to(at([x + reach, y + reach]), direct).await?;
                stage.move_to(at([x, y]), direct).await?;
                let (falling, _) = snap(acquisition).await?;
                let found = register(Arc::new(rising), falling).await?;
                sum = [sum[0] + found.shift[0] * bx, sum[1] + found.shift[1] * by];
            }
            let n = repeats.max(1) as f64;
            measurement.backlash_shift = Some([sum[0] / n, sum[1] / n]);
        }
        Ok(measurement)
    }
    .await;
    stage.move_to(start, same_side).await?;
    measured
}
