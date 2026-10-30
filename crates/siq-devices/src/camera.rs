//! The camera: what it can do, how it is set up, and where its frames go.
//!
//! A camera fills pool slots and hands each one, with the frame's metadata, to a [`FrameSink`]:
//! the acquisition engine (P2.1), which adds the pose and context and publishes the frame.

use std::sync::Arc;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use siq_buffer::{FillGuard, FramePool};
use siq_types::{
    Binning, Cfa, FrameFlags, FrameMeta, FrameSeq, Hertz, Micrometers, Millis, PixelFormat,
    PoseQuality, Roi, Timestamp,
};

use crate::{BoxFuture, Device, DeviceError, DeviceResult};

/// What a camera can do, as its driver reports it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CameraCapabilities {
    /// Sensor columns.
    pub sensor_width: u32,
    /// Sensor rows.
    pub sensor_height: u32,
    /// The sensor's pixel pitch.
    pub pixel_size: Micrometers,
    /// The pixel formats it can deliver.
    pub pixel_formats: Vec<PixelFormat>,
    /// A colour sensor's filter array at the sensor's top left; `None` for a mono sensor.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cfa: Option<Cfa>,
    /// The binning modes it offers, `Binning::NONE` among them.
    pub binnings: Vec<Binning>,
    /// What an ROI must align to.
    pub roi: RoiRules,
    /// The shortest exposure.
    pub exposure_min: Millis,
    /// The longest exposure.
    pub exposure_max: Millis,
    /// How fast it delivers frames.
    pub frame_rate: FrameRateModel,
}

/// What an ROI must align to, in sensor pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RoiRules {
    /// The step of the x offset and of the width.
    pub step_x: u32,
    /// The step of the y offset and of the height.
    pub step_y: u32,
    /// The narrowest ROI.
    pub min_width: u32,
    /// The shortest ROI.
    pub min_height: u32,
}

/// How fast a camera delivers frames. Reading the sensor takes `overhead`, plus `line_time` per
/// row read, and the next exposure overlaps the readout. Vertical binning reads fewer rows, unless
/// the sensor reads every row and bins after.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FrameRateModel {
    /// The fixed part of each readout.
    pub overhead: Millis,
    /// The time to read one row.
    pub line_time: Millis,
    /// Whether vertical binning reads fewer rows: `false` for a sensor that reads them all and bins
    /// after, as the Adimec D-103A12c does (P10.2).
    #[serde(default = "binned_rows")]
    pub binned_rows: bool,
}

const fn binned_rows() -> bool {
    true
}

impl FrameRateModel {
    /// The time to read out a frame with `settings`.
    pub fn readout(&self, settings: &CameraSettings) -> Millis {
        let rows = match self.binned_rows {
            true => settings.image_size().1,
            false => settings.roi.height,
        };
        self.overhead + self.line_time * f64::from(rows)
    }

    /// The highest frame rate with `settings`.
    pub fn max_rate(&self, settings: &CameraSettings) -> Hertz {
        let readout = self.readout(settings);
        Hertz::from_period(if settings.exposure > readout {
            settings.exposure
        } else {
            readout
        })
    }
}

impl CameraCapabilities {
    /// Checks `settings` against what the camera can do.
    pub fn check(&self, settings: &CameraSettings) -> DeviceResult<()> {
        let invalid = |why: String| Err(DeviceError::InvalidArgument(why));
        let (roi, rules) = (settings.roi, self.roi);
        let (bin_x, bin_y) = settings.binning.factors();
        let inside = |offset: u32, size: u32, sensor: u32| {
            u64::from(offset) + u64::from(size) <= u64::from(sensor)
        };
        if !self.pixel_formats.contains(&settings.pixel_format) {
            return invalid(format!(
                "pixel format {:?} isn't offered",
                settings.pixel_format
            ));
        }
        if !self.binnings.contains(&settings.binning) {
            return invalid(format!("binning {bin_x}×{bin_y} isn't offered"));
        }
        if !inside(roi.offset_x, roi.width, self.sensor_width)
            || !inside(roi.offset_y, roi.height, self.sensor_height)
        {
            return invalid(format!(
                "the ROI {roi:?} isn't within the {}×{} sensor",
                self.sensor_width, self.sensor_height
            ));
        }
        let aligned = |offset: u32, size: u32, step: u32| {
            offset.is_multiple_of(step) && size.is_multiple_of(step)
        };
        if !aligned(roi.offset_x, roi.width, rules.step_x)
            || !aligned(roi.offset_y, roi.height, rules.step_y)
        {
            return invalid(format!(
                "the ROI must align to {}×{} pixels",
                rules.step_x, rules.step_y
            ));
        }
        if roi.width < rules.min_width || roi.height < rules.min_height {
            return invalid(format!(
                "the ROI must be at least {}×{} pixels",
                rules.min_width, rules.min_height
            ));
        }
        if !roi.width.is_multiple_of(u32::from(bin_x))
            || !roi.height.is_multiple_of(u32::from(bin_y))
        {
            return invalid("the ROI must be a whole number of binned pixels".into());
        }
        if !(self.exposure_min <= settings.exposure && settings.exposure <= self.exposure_max) {
            return invalid(format!(
                "the exposure {} isn't within {} to {}",
                settings.exposure, self.exposure_min, self.exposure_max
            ));
        }
        if !(settings.gain.is_finite() && settings.gain > 0.0) {
            return invalid(format!("the gain {} must be positive", settings.gain));
        }
        Ok(())
    }
}

/// What starts each frame.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TriggerMode {
    /// Frames come at the camera's rate: live view.
    #[default]
    FreeRun,
    /// One frame per [`Camera::trigger`]: snaps, and stop-and-go scans.
    Software,
    /// One frame per signal on the camera's trigger input.
    External,
}

/// How a camera is set up.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CameraSettings {
    /// The sensor region read.
    pub roi: Roi,
    /// The binning applied to it.
    pub binning: Binning,
    /// The pixel format delivered.
    pub pixel_format: PixelFormat,
    /// The exposure time.
    pub exposure: Millis,
    /// The gain, as a factor.
    pub gain: f64,
    /// What starts each frame.
    pub trigger: TriggerMode,
}

impl CameraSettings {
    /// The whole sensor, unbinned, in the first format offered, at `exposure` and gain 1,
    /// free-running.
    pub fn full_frame(capabilities: &CameraCapabilities, exposure: Millis) -> Self {
        Self {
            roi: Roi::full(capabilities.sensor_width, capabilities.sensor_height),
            binning: Binning::NONE,
            pixel_format: capabilities
                .pixel_formats
                .first()
                .copied()
                .unwrap_or(PixelFormat::Mono16),
            exposure,
            gain: 1.0,
            trigger: TriggerMode::FreeRun,
        }
    }

    /// The width and height of the delivered image, after binning.
    pub fn image_size(&self) -> (u32, u32) {
        self.binning.image_size(self.roi.width, self.roi.height)
    }

    /// The metadata of frame `seq`, at `mid_exposure` on the host's clock, as far as the camera
    /// knows it: no device time, pose or context.
    pub fn frame_meta(&self, seq: u64, mid_exposure: Timestamp) -> FrameMeta {
        FrameMeta {
            seq: FrameSeq(seq),
            device_ts: None,
            host_ts_mid_exposure: mid_exposure,
            exposure: self.exposure,
            gain: self.gain,
            roi: self.roi,
            binning: self.binning,
            pixel_format: self.pixel_format,
            cfa: None,
            pose: None,
            pose_quality: PoseQuality::Unknown,
            channel: None,
            objective: None,
            calibration: None,
            flags: FrameFlags::default(),
        }
    }

    /// The bytes a frame takes in a pool slot.
    pub fn frame_bytes(&self) -> usize {
        let (width, height) = self.image_size();
        width as usize * height as usize * self.pixel_format.bytes_per_pixel()
    }
}

/// What a camera has delivered and lost since it was made.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CameraStats {
    /// Frames handed to the sink.
    pub delivered: u64,
    /// Frames lost because no pool slot was free: consumers were holding them all.
    pub lost_no_slot: u64,
    /// Frames the camera or the grabber lost or rejected.
    pub lost_device: u64,
}

/// Where a camera hands its frames.
pub trait FrameSink: Send + Sync {
    /// A filled slot, with the frame's metadata as far as the camera knows it: no pose or
    /// context yet. It is called on the camera's thread for every frame, so it must be quick.
    fn frame(&self, slot: FillGuard, meta: FrameMeta);
}

/// A camera.
///
/// `configure` checks the settings against the capabilities. While acquiring, the exposure and
/// the gain change from the next frame; other changes fail with
/// [`DeviceError::Busy`] until the camera is stopped.
pub trait Camera: Device {
    /// What it can do.
    fn capabilities(&self) -> &CameraCapabilities;

    /// The settings in force.
    fn settings(&self) -> CameraSettings;

    /// Applies `settings`.
    fn configure(&self, settings: CameraSettings) -> BoxFuture<'_, DeviceResult<()>>;

    /// Starts acquiring into `pool`'s slots, which must fit a frame, and hands each frame to
    /// `frames`. Frame numbers count from 0 again.
    fn start(&self, pool: FramePool, frames: Arc<dyn FrameSink>)
    -> BoxFuture<'_, DeviceResult<()>>;

    /// Stops acquiring, and gives back the slots it held.
    fn stop(&self) -> BoxFuture<'_, DeviceResult<()>>;

    /// Takes one frame, in [`TriggerMode::Software`] while acquiring. It resolves once the
    /// exposure has started; the frame arrives at the sink.
    fn trigger(&self) -> BoxFuture<'_, DeviceResult<()>>;

    /// What it has delivered and lost.
    fn stats(&self) -> CameraStats;
}
