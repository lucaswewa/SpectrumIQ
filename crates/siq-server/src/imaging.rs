//! The `imaging` Thing (ADR-0035): the configured channels, and applying one to the devices.

use anyhow::Context;
use siq_config::model::ChannelConfig;
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

/// Puts `channel` in place, in order, each step waiting for its device (ADR-0035), as the
/// workflows do; then the display colour, and the previews' dark and flat, are the channel's.
pub(crate) async fn apply(instrument: &Instrument, channel: &ChannelConfig) -> anyhow::Result<()> {
    siq_workflow::routines::channel::apply(&instrument.devices(), channel).await?;
    let colormap = match channel.colour.0 {
        [255, 255, 255] => Colormap::Grey,
        colour => Colormap::Colour(colour),
    };
    let display = instrument.previews.display();
    instrument.previews.set_display(DisplaySettings {
        colormap,
        ..display
    });
    crate::calibration::stamp(instrument);
    Ok(())
}

/// Waits half an exposure, at most a second: frames stamped after it began their exposure after
/// the wait did.
pub(crate) async fn until_frames_begin_after(instrument: &Instrument) {
    siq_workflow::routines::channel::until_frames_begin_after(instrument.camera.as_ref()).await;
}
