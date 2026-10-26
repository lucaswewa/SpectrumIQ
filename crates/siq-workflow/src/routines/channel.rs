//! Putting a channel in place (ADR-0035), in order, each step waiting for its device: every light
//! off, so that no frame mixes the old light and the new filter; the filter; the exposure; then
//! the channel's light, settled. Frames exposed meanwhile have no channel; then they have it.

use std::time::Duration;

use siq_acquire::AcquisitionContext;
use siq_config::model::ChannelConfig;
use siq_devices::{Camera, CameraSettings};

use crate::DeviceSet;
use crate::routines::stage_camera::RoutineError;

/// Puts `channel` in place on `devices`.
pub async fn apply(devices: &DeviceSet, channel: &ChannelConfig) -> Result<(), RoutineError> {
    let acquisition = &devices.acquisition;
    let unlabelled = AcquisitionContext {
        channel: None,
        ..acquisition.context()
    };
    acquisition.set_context(unlabelled);
    if let Some(lights) = &devices.illumination {
        let levels = lights.status().borrow().levels.clone();
        for (source, _) in levels.iter().enumerate().filter(|(_, level)| **level > 0.0) {
            lights.set_level(source, 0.0).await?;
        }
    }
    if let (Some(filter), Some(wheel)) = (&channel.filter, &devices.filter_wheel) {
        let position = wheel.positions().iter().position(|name| name == filter);
        let position = position
            .ok_or_else(|| RoutineError::Channel(format!("the filter wheel has no `{filter}`")))?;
        wheel.select(position as u8).await?;
    }
    let settings = CameraSettings {
        exposure: channel.exposure,
        ..devices.camera.settings()
    };
    acquisition.configure(settings).await?;
    if let (Some(light), Some(lights)) = (&channel.illumination, &devices.illumination) {
        let source = lights
            .sources()
            .iter()
            .position(|name| *name == light.source);
        let why = || RoutineError::Channel(format!("there is no source `{}`", light.source));
        lights
            .set_level(source.ok_or_else(why)?, light.intensity)
            .await?;
    }
    until_frames_begin_after(devices.camera.as_ref()).await;
    let labelled = AcquisitionContext {
        channel: Some(channel.id),
        ..acquisition.context()
    };
    acquisition.set_context(labelled);
    Ok(())
}

/// Waits half an exposure, at most a second. Frames are stamped at mid-exposure (ADR-0031), so a
/// frame stamped after this began its exposure after the wait did.
pub async fn until_frames_begin_after(camera: &dyn Camera) {
    let half = camera.settings().exposure.to_duration().unwrap_or_default() / 2;
    tokio::time::sleep(half.min(Duration::from_secs(1))).await;
}
