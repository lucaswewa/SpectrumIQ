//! The peripherals: illumination, the filter wheel, the objective turret and the modality unit
//! (REQ-DEV-001). Their models aren't chosen yet, so simulators drive them until P10.5.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use siq_types::ModalityMode;

use crate::{BoxFuture, Device, DeviceError, DeviceResult, watch};

/// Light sources, each with a level from 0 (off) to 1.
pub trait Illumination: Device {
    /// The sources' names, by index.
    fn sources(&self) -> &[String];

    /// Sets source `source` to `level`, and resolves when the light is steady.
    fn set_level(&self, source: usize, level: f64) -> BoxFuture<'_, DeviceResult<()>>;

    /// Every source's level: the latest now, and each change as it comes.
    fn status(&self) -> watch::Receiver<IlluminationStatus>;
}

/// Every light source's level.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct IlluminationStatus {
    /// Each source's level, by index, from 0 (off) to 1.
    pub levels: Vec<f64>,
}

/// A device that puts one of its positions in the light path: a filter wheel or an objective
/// turret.
pub trait Changer: Device {
    /// Each position's name, from position 0.
    fn positions(&self) -> &[String];

    /// Moves `position` into the light path, and resolves when it is there.
    fn select(&self, position: u8) -> BoxFuture<'_, DeviceResult<()>>;

    /// Which position is in the path: the latest now, and each change as it comes.
    fn status(&self) -> watch::Receiver<ChangerStatus>;
}

/// Which position of a [`Changer`] is in the light path.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ChangerStatus {
    /// The position in the path: `None` while moving, and before the first selection.
    pub position: Option<u8>,
    /// Whether it is moving.
    pub moving: bool,
}

/// The filter wheel: a [`Changer`] of filters.
pub trait FilterWheel: Changer {}

/// The objective turret: a [`Changer`] of objectives. The configuration gives each position's
/// optics.
pub trait ObjectiveTurret: Changer {}

/// A structured-illumination pattern: an orientation and a phase, by index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Pattern {
    /// The orientation, from 0.
    pub orientation: u8,
    /// The phase, from 0.
    pub phase: u8,
}

/// The structured-illumination patterns a modality unit can show.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PatternSet {
    /// The number of orientations.
    pub orientations: u8,
    /// The number of phases per orientation.
    pub phases: u8,
}

impl PatternSet {
    /// Whether `pattern` is one of the set.
    pub fn contains(&self, pattern: Pattern) -> bool {
        pattern.orientation < self.orientations && pattern.phase < self.phases
    }
}

/// An imaging mode, with its pattern for structured illumination.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Modality {
    /// The mode.
    pub mode: ModalityMode,
    /// The pattern, for [`ModalityMode::StructuredIllumination`] only.
    pub pattern: Option<Pattern>,
}

impl Modality {
    /// Plain widefield.
    pub const WIDEFIELD: Self = Self {
        mode: ModalityMode::Widefield,
        pattern: None,
    };

    /// Checks that a unit offering `modes` and `patterns` can take it.
    pub fn check(&self, modes: &[ModalityMode], patterns: Option<PatternSet>) -> DeviceResult<()> {
        let invalid = |why: &str| Err(DeviceError::InvalidArgument(why.to_owned()));
        if !modes.contains(&self.mode) {
            return invalid("the unit doesn't offer that mode");
        }
        let structured = self.mode == ModalityMode::StructuredIllumination;
        match self.pattern {
            Some(pattern) if structured && patterns.is_some_and(|set| set.contains(pattern)) => {
                Ok(())
            }
            _ if structured => invalid("structured illumination needs one of the unit's patterns"),
            Some(_) => invalid("only structured illumination takes a pattern"),
            None => Ok(()),
        }
    }
}

/// The modality unit's mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ModalityStatus {
    /// The modality in place, or the one it is moving to.
    pub modality: Modality,
    /// Whether it is moving.
    pub moving: bool,
}

/// The modality unit: widefield, the light-field microlens array, the confocal disk, and
/// structured-illumination patterns.
pub trait ModalityUnit: Device {
    /// The modes it offers.
    fn modes(&self) -> &[ModalityMode];

    /// The structured-illumination patterns it can show, if it offers that mode.
    fn patterns(&self) -> Option<PatternSet>;

    /// Puts `modality` in place, checked against the modes and patterns, and resolves when it is
    /// there.
    fn set_modality(&self, modality: Modality) -> BoxFuture<'_, DeviceResult<()>>;

    /// The mode: the latest now, and each change as it comes.
    fn status(&self) -> watch::Receiver<ModalityStatus>;
}
