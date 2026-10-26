//! The stage and the focus drive: their positions, moves and status (REQ-STG-003).
//!
//! The stage's z is the coarse height; the focus drive is the fine z (ADR-0016). Both work in the
//! stage frame of ADR-0018, in µm.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use siq_types::{Micrometers, MicrometersPerSecond, Timestamp};

use crate::{BoxFuture, Device, DeviceResult, watch};

/// A position, or a velocity, on the stage's three axes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Xyz<T = Micrometers> {
    /// The x axis.
    pub x: T,
    /// The y axis.
    pub y: T,
    /// The coarse z axis.
    pub z: T,
}

impl<T> Xyz<T> {
    /// The three axes' values.
    pub const fn new(x: T, y: T, z: T) -> Self {
        Self { x, y, z }
    }
}

impl Xyz {
    /// A position from three values in µm.
    pub const fn from_um(x: f64, y: f64, z: f64) -> Self {
        Self::new(Micrometers(x), Micrometers(y), Micrometers(z))
    }
}

impl std::ops::Add for Xyz {
    type Output = Self;
    fn add(self, other: Self) -> Self {
        Self::new(self.x + other.x, self.y + other.y, self.z + other.z)
    }
}

/// Where an axis, or each of the stage's axes, may go: from `min` to `max`, both included.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Travel<P = Micrometers> {
    /// The lowest position.
    pub min: P,
    /// The highest position.
    pub max: P,
}

impl Travel {
    /// Whether `position` is within the travel.
    pub fn contains(&self, position: Micrometers) -> bool {
        self.min <= position && position <= self.max
    }
}

impl Travel<Xyz> {
    /// Whether `position` is within the travel on every axis.
    pub fn contains(&self, position: Xyz) -> bool {
        let axis = |min, max, at| Travel::<Micrometers> { min, max }.contains(at);
        axis(self.min.x, self.max.x, position.x)
            && axis(self.min.y, self.max.y, position.y)
            && axis(self.min.z, self.max.z, position.z)
    }
}

/// How a move arrives at its target.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Approach {
    /// Straight to the target.
    #[default]
    Direct,
    /// Always from the same side, overshooting first when needed, so the backlash is the same on
    /// every arrival. The side is the driver's configuration (P1.9).
    Unidirectional,
}

/// How to move.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct MoveOpts {
    /// The speed, or `None` for the device's configured speed.
    pub speed: Option<MicrometersPerSecond>,
    /// How to arrive.
    pub approach: Approach,
}

/// What an axis, or the stage, is doing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MotionState {
    /// Still, and ready.
    #[default]
    Idle,
    /// Moving to a target.
    Moving,
    /// Moving at a velocity, until stopped.
    Jogging,
    /// Finding its reference.
    Homing,
    /// Paused by a feed hold, on the way to stopping.
    Holding,
    /// In alarm, with the device's code. Motion is refused until it is cleared.
    Alarm(u32),
    /// The connection is gone.
    Disconnected,
}

/// Where an axis, or the stage, is, what it is doing, and when that was true.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AxisStatus<P = Micrometers> {
    /// The position, as the device reports it.
    pub position: P,
    /// What it is doing.
    pub state: MotionState,
    /// Whether the position is referenced: homed, and not lost since, as a reset during a move
    /// loses it.
    pub homed: bool,
    /// When the position was true, on the shared clock. A polled driver takes the reply's receive
    /// time less half the round trip.
    pub ts: Timestamp,
}

impl<P> AxisStatus<P> {
    /// Whether it is still and ready.
    pub fn is_idle(&self) -> bool {
        self.state == MotionState::Idle
    }
}

/// The stage's status.
pub type StageStatus = AxisStatus<Xyz>;

/// The focus drive's status.
pub type FocusStatus = AxisStatus<Micrometers>;

/// The XYZ stage: x, y and the coarse z.
///
/// A move resolves once the stage has stopped at its target, with the settled status. `stop`
/// makes a move that is waiting fail with [`DeviceError::Stopped`](crate::DeviceError::Stopped).
pub trait Stage: Device {
    /// Moves to `target`.
    fn move_to(&self, target: Xyz, opts: MoveOpts) -> BoxFuture<'_, DeviceResult<StageStatus>>;

    /// Moves by `delta` from where it is.
    fn move_by(&self, delta: Xyz, opts: MoveOpts) -> BoxFuture<'_, DeviceResult<StageStatus>>;

    /// Moves at `velocity` until stopped, jogged again, or at a limit.
    fn jog(&self, velocity: Xyz<MicrometersPerSecond>) -> BoxFuture<'_, DeviceResult<()>>;

    /// Stops any motion, as quickly as the device can, and resolves when it is still.
    fn stop(&self) -> BoxFuture<'_, DeviceResult<()>>;

    /// Homes every axis, and resolves when the stage is referenced.
    fn home(&self) -> BoxFuture<'_, DeviceResult<StageStatus>>;

    /// The status: the latest now, and each change as it comes.
    fn status(&self) -> watch::Receiver<StageStatus>;

    /// Where each axis may go.
    fn limits(&self) -> Travel<Xyz>;

    /// How fast it moves in the plane, for estimates: `None` if the device doesn't say.
    fn kinematics(&self) -> Option<Kinematics> {
        None
    }
}

/// How fast a stage moves in the plane: the slower axis's top speed and acceleration.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Kinematics {
    /// The top speed, in µm/s.
    pub speed_um_s: f64,
    /// The acceleration, in µm/s².
    pub acceleration_um_s2: f64,
}

/// The focus drive: the fine z, for z-stacks, autofocus and focus maps. Its operations work as
/// the [`Stage`]'s do, on one axis.
pub trait FocusDrive: Device {
    /// Moves to `target`.
    fn move_to(
        &self,
        target: Micrometers,
        opts: MoveOpts,
    ) -> BoxFuture<'_, DeviceResult<FocusStatus>>;

    /// Moves by `delta` from where it is.
    fn move_by(
        &self,
        delta: Micrometers,
        opts: MoveOpts,
    ) -> BoxFuture<'_, DeviceResult<FocusStatus>>;

    /// Moves at `velocity` until stopped, jogged again, or at a limit.
    fn jog(&self, velocity: MicrometersPerSecond) -> BoxFuture<'_, DeviceResult<()>>;

    /// Stops any motion, and resolves when it is still.
    fn stop(&self) -> BoxFuture<'_, DeviceResult<()>>;

    /// Homes the drive, and resolves when it is referenced.
    fn home(&self) -> BoxFuture<'_, DeviceResult<FocusStatus>>;

    /// The status: the latest now, and each change as it comes.
    fn status(&self) -> watch::Receiver<FocusStatus>;

    /// Where the drive may go.
    fn limits(&self) -> Travel;
}
