//! The shading's captures (ADR-0050): frames for the dark, with every light off, and frames for
//! the flat, of a blank slide or of the sample out of focus, spread over places so that what is in
//! the field averages away. Each gives the frames' sum, and the settings they were taken at, for
//! `siq_calib::shading` to make the dark or fit the flat; and leaves the instrument as it was.

use siq_acquire::Acquisition;
use siq_calib::shading::Accumulator;
use siq_devices::{Approach, FocusDrive, Illumination, MoveOpts, Stage, Xyz};
use siq_types::{FrameMeta, Micrometers};

use crate::routines::stage_camera::RoutineError;

/// Frames, summed, and the settings of the last.
#[derive(Debug, Clone)]
pub struct Captured {
    /// Their sum.
    pub sum: Accumulator,
    /// The last frame's metadata: its region, binning and exposure, as all of them.
    pub meta: FrameMeta,
}

/// Adds `frames` frames to `captured`.
async fn take(
    acquisition: &Acquisition,
    frames: usize,
    captured: &mut Option<Captured>,
) -> Result<(), RoutineError> {
    for _ in 0..frames {
        let frame = acquisition.snap(false).await?;
        let image = frame.image::<u16>().ok_or(RoutineError::Format)?;
        let (width, height) = (image.width(), image.height());
        let taken = captured.get_or_insert_with(|| Captured {
            sum: Accumulator::new(width, height),
            meta: *frame.meta(),
        });
        taken.sum.add(&image)?;
        taken.meta = *frame.meta();
    }
    Ok(())
}

/// Takes `frames` frames with every light off, then turns each back to its level. Without the
/// lights' device, the light path must be blocked by hand.
pub async fn capture_dark(
    acquisition: &Acquisition,
    lights: Option<&dyn Illumination>,
    frames: usize,
) -> Result<Captured, RoutineError> {
    let levels = lights.map_or_else(Vec::new, |lights| lights.status().borrow().levels.clone());
    let on = || levels.iter().enumerate().filter(|(_, level)| **level > 0.0);
    if let Some(lights) = lights {
        for (source, _) in on() {
            lights.set_level(source, 0.0).await?;
        }
    }
    let mut captured = None;
    let taken = take(acquisition, frames.max(1), &mut captured).await;
    if let Some(lights) = lights {
        for (source, &level) in on() {
            lights.set_level(source, level).await?;
        }
    }
    taken?;
    captured.ok_or(RoutineError::Format)
}

/// Where a flat's frames are taken, and how far out of focus.
#[derive(Debug, Clone, PartialEq)]
pub struct FlatPlan {
    /// The places, from where the stage is, in µm.
    pub places: Vec<[f64; 2]>,
    /// The frames at each.
    pub frames: usize,
    /// How far the focus moves away first, in µm: 0 for a blank slide.
    pub defocus: f64,
}

impl FlatPlan {
    /// For a field of `field` µm: nine places a field apart on a 3 × 3 grid about the stage, two
    /// frames at each, `defocus` µm out of focus.
    pub fn for_field([width, height]: [f64; 2], defocus: f64) -> Self {
        let steps = [-1.0, 0.0, 1.0];
        let places = steps
            .iter()
            .flat_map(|dy| steps.iter().map(move |dx| [dx * width, dy * height]))
            .collect();
        Self {
            places,
            frames: 2,
            defocus,
        }
    }
}

/// Takes `plan`'s frames for a flat, about where the stage is or the nearest place where its
/// places stay within the stage's travel, then puts the stage and the focus back.
pub async fn capture_flat(
    acquisition: &Acquisition,
    stage: &dyn Stage,
    focus: &dyn FocusDrive,
    plan: &FlatPlan,
) -> Result<Captured, RoutineError> {
    let opts = MoveOpts {
        speed: None,
        approach: Approach::Direct,
    };
    let (start, focused) = (
        stage.status().borrow().position,
        focus.status().borrow().position,
    );
    let reach = super::reach(plan.places.iter().copied());
    let centre = super::centre_within(start, reach, &stage.limits())?;
    let mut captured = None;
    let taken = async {
        if plan.defocus != 0.0 {
            focus.move_by(Micrometers(plan.defocus), opts).await?;
        }
        for &[x, y] in &plan.places {
            let at = Xyz {
                x: centre.x + Micrometers(x),
                y: centre.y + Micrometers(y),
                z: centre.z,
            };
            stage.move_to(at, opts).await?;
            take(acquisition, plan.frames.max(1), &mut captured).await?;
        }
        Ok::<_, RoutineError>(())
    }
    .await;
    stage.move_to(start, opts).await?;
    focus.move_to(focused, opts).await?;
    taken?;
    captured.ok_or(RoutineError::Format)
}
