//! The `imaging` Thing (ADR-0035): the configured channels, and applying one to the devices.

use std::time::Duration;

use anyhow::Context;
use siq_acquire::AcquisitionContext;
use siq_config::model::ChannelConfig;
use siq_devices::CameraSettings;
use siq_imgproc::Colormap;
use siq_preview::DisplaySettings;
use siq_types::ChannelId;
use teta_wot::prelude::{ActionError, Event, Prop, Thing, ThingCtx, thing_impl};

use crate::Instrument;
use crate::things::{Bound, bind, bound};

/// The channels, and the one in use.
#[derive(Thing)]
pub struct Imaging {
    /// The channel in use: `null` before one is selected, and while one is being applied.
    #[property(readonly, default = None)]
    channel: Prop<Option<ChannelId>>,
    /// A channel was applied.
    #[event]
    channel_changed: Event<ChannelId>,
    instrument: Bound,
}

#[thing_impl]
impl Imaging {
    #[on_start]
    async fn start(&self, ctx: ThingCtx) -> anyhow::Result<()> {
        bind(&self.instrument, &ctx)
    }

    /// The channels: each one's illumination, filter, exposure and display colour.
    #[property]
    async fn channels(&self) -> Vec<ChannelConfig> {
        bound(&self.instrument).config.channels.clone()
    }

    /// Applies channel `id`, holding the global lock throughout: every light off, the filter, the
    /// exposure and the display colour, then the channel's light. Frames exposed meanwhile have no
    /// channel.
    #[action]
    async fn select_channel(&self, id: ChannelId) -> Result<ChannelConfig, ActionError> {
        let instrument = bound(&self.instrument);
        let channels = &instrument.config.channels;
        let channel = channels.iter().find(|channel| channel.id == id);
        let channel = channel
            .with_context(|| format!("there is no channel {id}"))?
            .clone();
        self.channel.set(None)?;
        apply(instrument, &channel).await?;
        self.channel.set(Some(id))?;
        self.channel_changed.emit(id);
        Ok(channel)
    }
}

/// Puts `channel` in place, in order, each step waiting for its device (ADR-0035).
pub(crate) async fn apply(instrument: &Instrument, channel: &ChannelConfig) -> anyhow::Result<()> {
    let (acquisition, peripherals) = (&instrument.acquisition, &instrument.peripherals);
    // From now until the channel is in place, frames have no channel.
    let unlabelled = AcquisitionContext {
        channel: None,
        ..acquisition.context()
    };
    acquisition.set_context(unlabelled);

    // 1. Every light off, so that no frame mixes the old light and the new filter.
    if let Some(lights) = &peripherals.illumination {
        let levels = lights.status().borrow().levels.clone();
        for (source, _) in levels.iter().enumerate().filter(|(_, level)| **level > 0.0) {
            lights.set_level(source, 0.0).await?;
        }
    }
    // 2. The filter.
    if let (Some(filter), Some(wheel)) = (&channel.filter, &peripherals.filter_wheel) {
        let position = wheel.positions().iter().position(|name| name == filter);
        let position = position.with_context(|| format!("the filter wheel has no `{filter}`"))?;
        wheel.select(position as u8).await?;
    }
    // 3. The exposure, from the next frame; 4. the display colour.
    let settings = CameraSettings {
        exposure: channel.exposure,
        ..instrument.camera.settings()
    };
    acquisition.configure(settings).await?;
    let colormap = match channel.colour.0 {
        [255, 255, 255] => Colormap::Grey,
        colour => Colormap::Colour(colour),
    };
    let display = instrument.previews.display();
    instrument.previews.set_display(DisplaySettings {
        colormap,
        ..display
    });
    // 5. The channel's light, settled.
    if let (Some(light), Some(lights)) = (&channel.illumination, &peripherals.illumination) {
        let source = lights
            .sources()
            .iter()
            .position(|name| *name == light.source);
        let source = source.with_context(|| format!("there is no source `{}`", light.source))?;
        lights.set_level(source, light.intensity).await?;
    }
    until_frames_begin_after(instrument).await;
    let labelled = AcquisitionContext {
        channel: Some(channel.id),
        ..acquisition.context()
    };
    acquisition.set_context(labelled);
    Ok(())
}

/// Waits half an exposure, at most a second. Frames are stamped at mid-exposure (ADR-0031), so a
/// frame stamped after this began its exposure after the wait did.
pub(crate) async fn until_frames_begin_after(instrument: &Instrument) {
    let exposure = instrument.camera.settings().exposure;
    let half = exposure.to_duration().unwrap_or_default() / 2;
    tokio::time::sleep(half.min(Duration::from_secs(1))).await;
}
