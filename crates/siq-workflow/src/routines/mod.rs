//! Routines: short sequences of moves and snaps that measure the instrument, on the device traits
//! and the acquisition engine, for any instrument, simulated or real.
//!
//! - [`stage_camera`]: the moves and image shifts that `siq-calib` fits the stage–camera affine
//!   from (ADR-0049).
//! - [`shading`]: the frames of the dark and of the flat (ADR-0050).

pub mod shading;
pub mod stage_camera;

use siq_devices::{Travel, Xyz};
use siq_types::Micrometers;

use stage_camera::RoutineError;

/// The place about which moves reaching `reach` µm each way stay within `travel`: `start` if they
/// do there, or else the nearest place to it where they do, at the same `z`.
pub(crate) fn centre_within(
    start: Xyz,
    reach: [f64; 2],
    travel: &Travel<Xyz>,
) -> Result<Xyz, RoutineError> {
    let fit = |at: Micrometers, low: Micrometers, high: Micrometers, reach: f64| {
        let (low, high) = (low.0 + reach, high.0 - reach);
        (low <= high).then(|| Micrometers(at.0.clamp(low, high)))
    };
    let x = fit(start.x, travel.min.x, travel.max.x, reach[0]);
    let y = fit(start.y, travel.min.y, travel.max.y, reach[1]);
    match (x, y) {
        (Some(x), Some(y)) => Ok(Xyz { x, y, z: start.z }),
        _ => Err(RoutineError::Travel(reach)),
    }
}

/// How far `places` reach each way, in µm.
pub(crate) fn reach(places: impl IntoIterator<Item = [f64; 2]>) -> [f64; 2] {
    let far = |[a, b]: [f64; 2], [x, y]: [f64; 2]| [a.max(x.abs()), b.max(y.abs())];
    places.into_iter().fold([0.0; 2], far)
}
