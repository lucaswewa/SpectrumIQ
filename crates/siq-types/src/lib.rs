//! SpectrumIQ's shared vocabulary: the types every other crate speaks.
//!
//! - [`units`]: physical quantities as `f64` newtypes ([`Micrometers`], [`Millis`], …).
//! - [`geometry`]: stage positions and displacements in µm, pixel points and rectangles.
//! - [`affine`]: 2D affine transforms, and the typed stage↔pixel mapping of a frame.
//! - [`image`]: pixel formats, ROI and binning, and borrowed image views that never allocate.
//! - [`frame`]: the metadata every camera frame carries, and the imaging modes.
//! - [`ids`]: dataset, calibration, channel and objective ids.
//! - [`time`]: monotonic timestamps and the [`Clock`] that makes time testable.
//!
//! The coordinate frames, units and handedness are fixed by ADR-0018.

pub mod affine;
pub mod frame;
pub mod geometry;
pub mod ids;
pub mod image;
pub mod time;
pub mod units;

pub use affine::{Affine2, AffineParams, PixelToStage, StageToPixel};
pub use frame::{DeviceMicros, FrameFlags, FrameMeta, FrameSeq, ModalityMode, PoseQuality};
pub use geometry::{PointPx, RectPx, RectUm, StagePose, Vec2Um};
pub use ids::{CalibrationId, ChannelId, DatasetId, ObjectiveId};
pub use image::{Binning, ImageError, ImageView, ImageViewMut, OwnedImage, PixelFormat, Roi};
pub use time::{Clock, ManualClock, SystemClock, Timestamp};
pub use units::{Hertz, Micrometers, MicrometersPerSecond, Millimeters, Millis, Nanometers};
