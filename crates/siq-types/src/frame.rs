//! The metadata every camera frame carries (REQ-ACQ-004), and the modality unit's imaging modes.
//!
//! [`FrameMeta`] is `Copy` and holds no heap data, so the frame path can stamp and pass it at the
//! camera's rate without allocating.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::geometry::StagePose;
use crate::ids::{CalibrationId, ChannelId, ObjectiveId};
use crate::image::{Binning, Cfa, PixelFormat, Roi};
use crate::time::Timestamp;
use crate::units::{Micrometers, Millis};

/// A frame's number: it counts up by one for each frame the camera delivers, from 0 when the
/// camera starts. A gap means frames were lost.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(transparent)]
pub struct FrameSeq(pub u64);

impl FrameSeq {
    /// The frame after this one.
    pub const fn next(self) -> Self {
        Self(self.0 + 1)
    }
}

/// A timestamp from the camera or grabber, in microseconds of the device's own clock. It is only
/// comparable with other timestamps from the same device.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(transparent)]
pub struct DeviceMicros(pub u64);

/// How well a frame's stage pose is known.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PoseQuality {
    /// Every axis was still during the exposure: the pose is the stage's reading.
    Settled,
    /// The stage was moving: the pose is interpolated between readings.
    Interpolated {
        /// How far the true position may be from the interpolated one.
        uncertainty: Micrometers,
    },
    /// There was no stage reading to go by.
    Unknown,
}

/// Flags that say what happens to a frame.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FrameFlags {
    /// The recorder saves the frame to the dataset.
    pub save: bool,
    /// The stage was moving during the exposure.
    pub moving: bool,
}

/// What is known about a frame: when, how and where it was taken.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FrameMeta {
    /// The frame's number.
    pub seq: FrameSeq,
    /// The device's timestamp, if it gives one.
    pub device_ts: Option<DeviceMicros>,
    /// The middle of the exposure, on the host's monotonic clock.
    pub host_ts_mid_exposure: Timestamp,
    /// The exposure time.
    pub exposure: Millis,
    /// The analog gain, as a linear factor: 1 is unity.
    pub gain: f64,
    /// The sensor region read out.
    pub roi: Roi,
    /// The binning applied to the region.
    pub binning: Binning,
    /// The pixel format.
    pub pixel_format: PixelFormat,
    /// A colour sensor's filter array at the image's top left; `None` for a mono sensor.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cfa: Option<Cfa>,
    /// The stage and focus position, if known.
    pub pose: Option<StagePose>,
    /// How well [`pose`](Self::pose) is known.
    pub pose_quality: PoseQuality,
    /// The channel (illumination, filter and exposure) the frame was taken in.
    pub channel: Option<ChannelId>,
    /// The objective in use.
    pub objective: Option<ObjectiveId>,
    /// The stage–camera calibration in force for that objective (REQ-CAL-003).
    pub calibration: Option<CalibrationId>,
    /// What happens to the frame.
    pub flags: FrameFlags,
}

impl FrameMeta {
    /// The image's width and height, in pixels: the region after binning.
    pub const fn image_size(&self) -> (u32, u32) {
        self.binning.image_size(self.roi.width, self.roi.height)
    }
}

/// An imaging mode of the modality unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ModalityMode {
    /// Plain widefield.
    Widefield,
    /// Light field, with the microlens array in.
    LightField,
    /// Spinning-disk confocal.
    Confocal,
    /// Structured illumination (SIM or HiLo).
    StructuredIllumination,
}
