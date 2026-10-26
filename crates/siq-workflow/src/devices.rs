//! The instrument as the workflows see it.

use std::sync::Arc;

use siq_acquire::Acquisition;
use siq_calib::CalibrationRegistry;
use siq_config::model::ChannelConfig;
use siq_devices::{
    Camera, FilterWheel, FocusDrive, Illumination, ModalityUnit, ObjectiveTurret, Stage,
};
use siq_types::Clock;

/// Each device, the acquisition engine, the channels configured and the calibrations: shared
/// handles only, so that each run takes a clone.
#[derive(Clone)]
pub struct DeviceSet {
    /// The acquisition engine, which drives the camera.
    pub acquisition: Acquisition,
    /// The camera.
    pub camera: Arc<dyn Camera>,
    /// The XYZ stage.
    pub stage: Arc<dyn Stage>,
    /// The focus drive.
    pub focus: Arc<dyn FocusDrive>,
    /// The light sources, if fitted.
    pub illumination: Option<Arc<dyn Illumination>>,
    /// The filter wheel, if fitted.
    pub filter_wheel: Option<Arc<dyn FilterWheel>>,
    /// The objective turret, if fitted.
    pub turret: Option<Arc<dyn ObjectiveTurret>>,
    /// The modality unit, if fitted.
    pub modality: Option<Arc<dyn ModalityUnit>>,
    /// The channels configured.
    pub channels: Vec<ChannelConfig>,
    /// The stage–camera calibrations.
    pub calibrations: Arc<CalibrationRegistry>,
    /// The drivers' and the acquisition's clock.
    pub clock: Arc<dyn Clock>,
}
