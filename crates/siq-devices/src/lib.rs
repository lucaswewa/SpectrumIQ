//! Device traits: what the engines know of the camera, the stage, the focus drive and the
//! peripherals, whatever drives them (ADR-0023).
//!
//! - Every device is a [`Device`], with a [`DeviceInfo`]. Its operations return a [`BoxFuture`]
//!   of a [`DeviceResult`], so the traits are dyn-compatible: engines hold an `Arc<dyn Stage>`, and
//!   the server picks the driver from the configuration.
//! - What a device can do is data: [`CameraCapabilities`], [`Travel`], the names of a
//!   [`Changer`]'s positions.
//! - A device that changes state publishes it on a [`watch`] channel. Readers see the latest
//!   status, and wait for the next, without polling.
//! - [`mock`] has a mock of each trait that completes every operation at once, for engine tests.

use std::future::Future;
use std::pin::Pin;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub mod camera;
pub mod mock;
pub mod motion;
pub mod peripherals;

pub use camera::{
    Camera, CameraCapabilities, CameraSettings, CameraStats, FrameRateModel, FrameSink, RoiRules,
    TriggerMode,
};
pub use motion::{
    Approach, AxisStatus, FocusDrive, FocusStatus, Kinematics, MotionState, MoveOpts, Stage,
    StageStatus, Travel, Xyz,
};
pub use peripherals::{
    Changer, ChangerStatus, FilterWheel, Illumination, IlluminationStatus, Modality,
    ModalityStatus, ModalityUnit, ObjectiveTurret, Pattern, PatternSet,
};
pub use siq_types::ModalityMode;
pub use tokio::sync::watch;

/// A boxed future that can move between threads: what every device operation returns.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// The result of a device operation.
pub type DeviceResult<T> = Result<T, DeviceError>;

/// What every device is.
pub trait Device: Send + Sync {
    /// Who made it, and which one it is.
    fn info(&self) -> &DeviceInfo;
}

/// Who made a device, and which one it is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DeviceInfo {
    /// The maker: `"SpectrumIQ"` for simulators and mocks.
    pub vendor: String,
    /// The model.
    pub model: String,
    /// The serial number, if the device reports one.
    pub serial: Option<String>,
    /// The firmware version, if the device reports one.
    pub firmware: Option<String>,
}

impl DeviceInfo {
    /// A device with no serial number or firmware version.
    pub fn new(vendor: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            vendor: vendor.into(),
            model: model.into(),
            serial: None,
            firmware: None,
        }
    }
}

/// Why a device operation failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DeviceError {
    /// The device didn't answer, or didn't finish, in time.
    #[error("timed out: {0}")]
    Timeout(String),
    /// The connection to the device is gone.
    #[error("disconnected: {0}")]
    Disconnected(String),
    /// The device reported a fault, or answered what the driver can't understand.
    #[error("device fault: {0}")]
    Fault(String),
    /// The request is outside what the device can do, such as a move past its travel.
    #[error("invalid argument: {0}")]
    InvalidArgument(String),
    /// The device can't do that now, such as changing the ROI while acquiring.
    #[error("busy: {0}")]
    Busy(String),
    /// The device can't do that at all.
    #[error("not supported: {0}")]
    Unsupported(String),
    /// The device raised an alarm, with its code. It must be cleared, or the device homed.
    #[error("alarm {0}")]
    Alarm(u32),
    /// The operation was stopped before it finished, by `stop()`.
    #[error("stopped")]
    Stopped,
}
