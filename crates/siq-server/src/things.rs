//! The instrument's Things (ADR-0033): thin adapters from teta-wot's affordances to the engines
//! and drivers, which hold no algorithms.
//!
//! - **`camera`:** its settings, live view and snaps, and the `overview` and `roi` MJPEG streams.
//! - **`display`:** the display mapping and the ROI region of the previews, and the latest
//!   statistics.
//! - **`stage`** and **`focus`:** their status, mirrored into observable properties, and their
//!   moves. Each `stop` bypasses the global lock.
//!
//! Each Thing finds the [`Instrument`] service when it starts.

use std::future::Future;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::Duration;

use anyhow::Context;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use siq_acquire::Mode;
use siq_buffer::FrameLease;
use siq_devices::{
    Approach, AxisStatus, CameraSettings, DeviceResult, FocusStatus, MotionState, MoveOpts,
    StageStatus, Travel, Xyz, watch,
};
use siq_preview::{
    Contrast, DisplaySettings, FrameStats, PreviewSink, Renderer, RoiSettings, StatsSink,
    StreamKind, fit,
};
use siq_types::{
    Binning, FrameMeta, Micrometers, MicrometersPerSecond, Millis, PixelFormat, RectPx, Roi,
};
use teta_wot::blob::Jpeg;
use teta_wot::prelude::{
    ActionCtx, ActionError, Blob, Event, MjpegStream, Prop, PropertyError, Thing, ThingCtx,
    ThingRef, thing_impl,
};
use tokio::task::JoinHandle;

use crate::Instrument;

/// Where a Thing finds the instrument, once it has started.
type Bound = OnceLock<Arc<Instrument>>;

fn bind(slot: &Bound, ctx: &ThingCtx) -> anyhow::Result<()> {
    let instrument = ctx
        .server()
        .service::<Instrument>()
        .context("the server has no instrument")?;
    let _ = slot.set(instrument);
    Ok(())
}

fn bound(slot: &Bound) -> &Instrument {
    slot.get()
        .expect("a Thing is used only once it has started")
}

/// The Thing that `ctx` is about, for a task that outlives the hook.
fn this<T: Thing>(ctx: &ThingCtx) -> anyhow::Result<Arc<T>> {
    let thing = ctx
        .server()
        .thing_ref::<T>(ctx.name())
        .context("the Thing itself")?;
    Ok(Arc::clone(ThingRef::thing(&thing)))
}

/// Calls `apply` with each status from `status`, the current one first.
fn follow<P: Copy + Send + Sync + 'static>(
    mut status: watch::Receiver<AxisStatus<P>>,
    apply: impl Fn(&AxisStatus<P>) + Send + 'static,
) -> JoinHandle<()> {
    apply(&status.borrow_and_update());
    tokio::spawn(async move {
        while status.changed().await.is_ok() {
            apply(&status.borrow_and_update());
        }
    })
}

fn failed(error: impl Into<anyhow::Error>) -> PropertyError {
    PropertyError::Failed(error.into())
}

fn stop_task(task: &Mutex<Option<JoinHandle<()>>>) {
    if let Some(task) = task.lock().unwrap_or_else(PoisonError::into_inner).take() {
        task.abort();
    }
}

// ---- The camera ----------------------------------------------------------------------------

/// What the camera has done since the service started.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CameraStats {
    /// `stopped`, `live` or `triggered`.
    pub mode: String,
    /// The rate of the last frames, in frames per second.
    pub fps: f64,
    /// The frames published.
    pub published: u64,
    /// The frames lost for want of a free slot.
    pub lost_no_slot: u64,
    /// The frames the camera or grabber lost.
    pub lost_device: u64,
}

/// The camera's sensor.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Sensor {
    /// Columns.
    pub width: u32,
    /// Rows.
    pub height: u32,
    /// The pixel pitch, in µm.
    pub pixel_size: f64,
    /// The pixel formats it offers.
    pub pixel_formats: Vec<PixelFormat>,
}

/// The camera, its live view and its preview streams.
#[derive(Thing)]
pub struct Camera {
    /// The live overview: the whole frame, shrunk to fit 1920 × 1080.
    #[stream]
    overview: MjpegStream,
    /// The live region of interest, at 1:1 or 2:1: the display's `roi_region`.
    #[stream]
    roi: MjpegStream,
    /// Frames were lost: the camera's statistics after the loss.
    #[event]
    frames_lost: Event<CameraStats>,
    instrument: Bound,
    watcher: Mutex<Option<JoinHandle<()>>>,
}

/// The camera's preview streams, as the previews' sink.
pub(crate) struct Streams {
    overview: MjpegStream,
    roi: MjpegStream,
}

impl PreviewSink for Streams {
    fn push_jpeg(&self, stream: StreamKind, jpeg: Vec<u8>, _: &FrameMeta) {
        let stream = match stream {
            StreamKind::Overview => &self.overview,
            StreamKind::Roi => &self.roi,
        };
        let _ = stream.add_frame(jpeg);
    }
}

impl Camera {
    /// The streams that the previews feed.
    pub(crate) fn streams(&self) -> Streams {
        Streams {
            overview: self.overview.clone(),
            roi: self.roi.clone(),
        }
    }

    fn statistics(&self) -> CameraStats {
        let stats = bound(&self.instrument).acquisition.stats();
        let mode = match stats.mode {
            Mode::Stopped => "stopped",
            Mode::Live => "live",
            Mode::Triggered => "triggered",
        };
        CameraStats {
            mode: mode.into(),
            fps: stats.fps,
            published: stats.published,
            lost_no_slot: stats.lost_no_slot,
            lost_device: stats.lost_device,
        }
    }

    async fn reconfigure(
        &self,
        change: impl FnOnce(&mut CameraSettings),
    ) -> Result<(), PropertyError> {
        let instrument = bound(&self.instrument);
        let mut settings = instrument.camera.settings();
        change(&mut settings);
        instrument
            .acquisition
            .configure(settings)
            .await
            .map_err(failed)
    }
}

/// `frame`'s preview, as the overview shows it, encoded as a JPEG.
fn preview(frame: &FrameLease, display: &DisplaySettings) -> anyhow::Result<Vec<u8>> {
    let meta = frame.meta();
    let image = frame
        .image::<u16>()
        .context("only 16-bit frames are previewed")?;
    let mut renderer = Renderer::default();
    let mapping = renderer.mapping(&image, meta.pixel_format.bits(), display);
    let (width, height) = meta.image_size();
    let whole = RectPx::new(0, 0, width, height);
    renderer.show(&image, whole, fit((width, height), (1920, 1080)), mapping)?;
    Ok(renderer.encode(85)?)
}

#[thing_impl]
impl Camera {
    #[on_start]
    async fn start(&self, ctx: ThingCtx) -> anyhow::Result<()> {
        bind(&self.instrument, &ctx)?;
        let camera = this::<Camera>(&ctx)?;
        // Losses are announced at most once a second, with the totals.
        let watcher = tokio::spawn(async move {
            let mut announced = 0;
            loop {
                tokio::time::sleep(Duration::from_secs(1)).await;
                let stats = camera.statistics();
                let lost = stats.lost_no_slot + stats.lost_device;
                if lost > announced {
                    announced = lost;
                    camera.frames_lost.emit(stats);
                }
            }
        });
        *self.watcher.lock().unwrap_or_else(PoisonError::into_inner) = Some(watcher);
        Ok(())
    }

    #[on_stop]
    async fn stop_streams(&self) {
        stop_task(&self.watcher);
        self.overview.stop();
        self.roi.stop();
    }

    /// The exposure time, in ms.
    #[property(unit = "ms")]
    async fn exposure_ms(&self) -> f64 {
        bound(&self.instrument).camera.settings().exposure.0
    }

    #[setter(exposure_ms)]
    async fn set_exposure_ms(&self, value: f64) -> Result<(), PropertyError> {
        self.reconfigure(|settings| settings.exposure = Millis(value))
            .await
    }

    /// The sensor region read out, in unbinned sensor pixels. Changing it while live restarts the
    /// camera.
    #[property]
    async fn region(&self) -> Roi {
        bound(&self.instrument).camera.settings().roi
    }

    #[setter(region)]
    async fn set_region(&self, value: Roi) -> Result<(), PropertyError> {
        self.reconfigure(|settings| settings.roi = value).await
    }

    /// The binning, across and down. Changing it while live restarts the camera.
    #[property]
    async fn binning(&self) -> Binning {
        bound(&self.instrument).camera.settings().binning
    }

    #[setter(binning)]
    async fn set_binning(&self, value: Binning) -> Result<(), PropertyError> {
        self.reconfigure(|settings| settings.binning = value).await
    }

    /// The pixel format. Changing it while live restarts the camera.
    #[property]
    async fn pixel_format(&self) -> PixelFormat {
        bound(&self.instrument).camera.settings().pixel_format
    }

    #[setter(pixel_format)]
    async fn set_pixel_format(&self, value: PixelFormat) -> Result<(), PropertyError> {
        self.reconfigure(|settings| settings.pixel_format = value)
            .await
    }

    /// The fastest frame rate the settings allow, in frames per second.
    #[property(unit = "Hz")]
    async fn frame_rate(&self) -> f64 {
        let camera = &bound(&self.instrument).camera;
        camera
            .capabilities()
            .frame_rate
            .max_rate(&camera.settings())
            .0
    }

    /// The sensor.
    #[property]
    async fn sensor(&self) -> Sensor {
        let capabilities = bound(&self.instrument).camera.capabilities();
        Sensor {
            width: capabilities.sensor_width,
            height: capabilities.sensor_height,
            pixel_size: capabilities.pixel_size.0,
            pixel_formats: capabilities.pixel_formats.clone(),
        }
    }

    /// What the camera has done since the service started: its mode, its rate, and the frames
    /// published and lost.
    #[property]
    async fn stats(&self) -> CameraStats {
        self.statistics()
    }

    /// Starts live view.
    #[action]
    async fn start_live(&self) -> Result<(), ActionError> {
        Ok(bound(&self.instrument).acquisition.start_live().await?)
    }

    /// Stops live view.
    #[action]
    async fn stop_live(&self) -> Result<(), ActionError> {
        Ok(bound(&self.instrument).acquisition.stop().await?)
    }

    /// Takes one frame once the stage and focus are still, software-triggered, and gives back its
    /// preview as a JPEG. With `save`, the frame is marked for the recorder.
    #[action]
    async fn snap(&self, #[param(default)] save: bool) -> Result<Blob<Jpeg>, ActionError> {
        let instrument = bound(&self.instrument);
        let frame = instrument.acquisition.snap(save).await?;
        let display = instrument.previews.display();
        let jpeg = tokio::task::spawn_blocking(move || preview(&frame, &display)).await??;
        Ok(Blob::from_bytes(jpeg))
    }
}

// ---- The display ---------------------------------------------------------------------------

/// The latest statistics of the ROI.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Histogram {
    /// The frame measured.
    pub frame: u64,
    /// The counts, in 256 bins over the pixel format's range.
    pub bins: Vec<u32>,
    /// The 0.1th and 99.9th percentiles.
    pub window: [u16; 2],
    /// The focus measure: the mean squared Sobel gradient.
    pub focus: f64,
}

/// The latest statistics, as the previews' statistics sink.
#[derive(Default)]
pub(crate) struct LatestStats(Mutex<Option<FrameStats>>);

impl StatsSink for LatestStats {
    fn push_stats(&self, stats: &FrameStats) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = Some(stats.clone());
    }
}

impl LatestStats {
    fn latest(&self) -> Option<FrameStats> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

/// How the previews show the frames, and what the latest frame's ROI measures.
#[derive(Thing)]
pub struct Display {
    instrument: Bound,
}

#[thing_impl]
impl Display {
    #[on_start]
    async fn start(&self, ctx: ThingCtx) -> anyhow::Result<()> {
        bind(&self.instrument, &ctx)
    }

    /// How the previews show a frame: a fixed window and gamma, or auto-contrast, and the colours.
    #[property(global_lock = false)]
    async fn lut(&self) -> DisplaySettings {
        bound(&self.instrument).previews.display()
    }

    #[setter(lut)]
    async fn set_lut(&self, value: DisplaySettings) -> Result<(), PropertyError> {
        bound(&self.instrument).previews.set_display(value);
        Ok(())
    }

    /// The ROI stream's region, in sensor pixels (the middle of the frame if `null`), and its
    /// zoom: `native` for 1:1, `half` for 2:1.
    #[property(global_lock = false)]
    async fn roi_region(&self) -> RoiSettings {
        bound(&self.instrument).previews.roi()
    }

    #[setter(roi_region)]
    async fn set_roi_region(&self, value: RoiSettings) -> Result<(), PropertyError> {
        bound(&self.instrument).previews.set_roi(value);
        Ok(())
    }

    /// The ROI's histogram, window and focus measure, from the latest statistics.
    #[property]
    async fn histogram(&self) -> Option<Histogram> {
        let stats = bound(&self.instrument).stats.latest()?;
        Some(Histogram {
            frame: stats.meta.seq.0,
            bins: stats.histogram.to_vec(),
            window: [stats.window.0, stats.window.1],
            focus: stats.focus,
        })
    }

    /// The ROI's focus measure, the mean squared Sobel gradient, from the latest statistics.
    #[property]
    async fn focus_measure(&self) -> Option<f64> {
        Some(bound(&self.instrument).stats.latest()?.focus)
    }

    /// Fixes the window at the ROI's latest 0.1–99.9 % range, at gamma 1.
    #[action(global_lock = false)]
    async fn auto_contrast_once(&self) -> Result<DisplaySettings, ActionError> {
        let instrument = bound(&self.instrument);
        let stats = instrument
            .stats
            .latest()
            .context("no frame has been measured yet: start live view")?;
        let (min, max) = stats.window;
        let contrast = Contrast::Manual {
            min,
            max,
            gamma: 1.0,
        };
        let settings = DisplaySettings {
            contrast,
            ..instrument.previews.display()
        };
        instrument.previews.set_display(settings);
        Ok(settings)
    }
}

// ---- The stage -----------------------------------------------------------------------------

/// The XYZ stage.
#[derive(Thing)]
pub struct Stage {
    /// Where the stage is, in µm.
    #[property(readonly, default = Xyz::default())]
    position: Prop<Xyz>,
    /// What the stage is doing.
    #[property(readonly, default = MotionState::Idle)]
    state: Prop<MotionState>,
    /// Whether the position is referenced: homed, and not lost since.
    #[property(readonly, default = false)]
    homed: Prop<bool>,
    /// The speed of moves, in µm/s: 0 for the controller's own.
    #[setting(default = 0.0, ge = 0, unit = "µm/s")]
    speed: Prop<f64>,
    /// The stage arrived where it was sent.
    #[event]
    arrived: Event<Xyz>,
    /// The controller raised an alarm, with its code.
    #[event]
    alarm: Event<u32>,
    instrument: Bound,
    follower: Mutex<Option<JoinHandle<()>>>,
}

/// How a move goes, from a Thing's speed setting and its `unidirectional` parameter.
fn options(speed: f64, unidirectional: bool) -> MoveOpts {
    MoveOpts {
        speed: (speed > 0.0).then_some(MicrometersPerSecond(speed)),
        approach: if unidirectional {
            Approach::Unidirectional
        } else {
            Approach::Direct
        },
    }
}

/// Runs a motion, and stops the device if the invocation is cancelled.
async fn moving<S>(
    ctx: &ActionCtx,
    motion: impl Future<Output = DeviceResult<S>>,
    stop: impl Future<Output = DeviceResult<()>>,
) -> Result<S, ActionError> {
    tokio::select! {
        done = motion => Ok(done?),
        cancelled = ctx.cancelled() => {
            let _ = stop.await;
            Err(cancelled.into())
        }
    }
}

impl Stage {
    fn mirror(&self, status: &StageStatus) {
        let before = self.state.get();
        let _ = self.position.set(status.position);
        let _ = self.state.set(status.state);
        let _ = self.homed.set(status.homed);
        if let MotionState::Alarm(code) = status.state
            && before != status.state
        {
            self.alarm.emit(code);
        }
    }

    async fn go(
        &self,
        ctx: &ActionCtx,
        target: Xyz,
        unidirectional: bool,
    ) -> Result<StageStatus, ActionError> {
        let stage = &bound(&self.instrument).stage;
        let opts = options(self.speed.get(), unidirectional);
        let status = moving(ctx, stage.move_to(target, opts), stage.stop()).await?;
        self.arrived.emit(status.position);
        Ok(status)
    }
}

#[thing_impl]
impl Stage {
    #[on_start]
    async fn start(&self, ctx: ThingCtx) -> anyhow::Result<()> {
        bind(&self.instrument, &ctx)?;
        let stage = this::<Stage>(&ctx)?;
        let follower = follow(bound(&self.instrument).stage.status(), move |status| {
            stage.mirror(status)
        });
        *self.follower.lock().unwrap_or_else(PoisonError::into_inner) = Some(follower);
        Ok(())
    }

    #[on_stop]
    async fn stop_following(&self) {
        stop_task(&self.follower);
    }

    /// Where each axis may go, in µm.
    #[property]
    async fn limits(&self) -> Travel<Xyz> {
        bound(&self.instrument).stage.limits()
    }

    /// Moves to a position, in µm. With `unidirectional`, each axis arrives from its configured
    /// side. Cancelling the invocation stops the stage.
    #[action]
    async fn move_to(
        &self,
        ctx: ActionCtx,
        x: f64,
        y: f64,
        z: f64,
        #[param(default)] unidirectional: bool,
    ) -> Result<StageStatus, ActionError> {
        self.go(&ctx, Xyz::from_um(x, y, z), unidirectional).await
    }

    /// Moves by a distance from where the stage is, in µm.
    #[action]
    async fn move_by(
        &self,
        ctx: ActionCtx,
        #[param(default)] dx: f64,
        #[param(default)] dy: f64,
        #[param(default)] dz: f64,
        #[param(default)] unidirectional: bool,
    ) -> Result<StageStatus, ActionError> {
        let here = self.position.get();
        self.go(&ctx, here + Xyz::from_um(dx, dy, dz), unidirectional)
            .await
    }

    /// Moves at a velocity, in µm/s, until stopped, jogged again, or at a limit.
    #[action]
    async fn jog(
        &self,
        #[param(default)] vx: f64,
        #[param(default)] vy: f64,
        #[param(default)] vz: f64,
    ) -> Result<(), ActionError> {
        let velocity = Xyz::new(
            MicrometersPerSecond(vx),
            MicrometersPerSecond(vy),
            MicrometersPerSecond(vz),
        );
        Ok(bound(&self.instrument).stage.jog(velocity).await?)
    }

    /// Homes every axis.
    #[action]
    async fn home(&self, ctx: ActionCtx) -> Result<StageStatus, ActionError> {
        let stage = &bound(&self.instrument).stage;
        let status = moving(&ctx, stage.home(), stage.stop()).await?;
        self.arrived.emit(status.position);
        Ok(status)
    }

    /// Stops any motion as quickly as the stage can. It doesn't wait for the global lock.
    #[action(global_lock = false)]
    async fn halt(&self) -> Result<(), ActionError> {
        Ok(bound(&self.instrument).stage.stop().await?)
    }
}

// ---- The focus drive -----------------------------------------------------------------------

/// The focus drive: the fine z.
#[derive(Thing)]
pub struct Focus {
    /// Where the focus is, in µm.
    #[property(readonly, default = 0.0, unit = "µm")]
    position: Prop<f64>,
    /// What the focus drive is doing.
    #[property(readonly, default = MotionState::Idle)]
    state: Prop<MotionState>,
    /// Whether the position is referenced.
    #[property(readonly, default = false)]
    homed: Prop<bool>,
    /// The focus drive arrived where it was sent, in µm.
    #[event]
    arrived: Event<f64>,
    instrument: Bound,
    follower: Mutex<Option<JoinHandle<()>>>,
}

impl Focus {
    fn mirror(&self, status: &FocusStatus) {
        let _ = self.position.set(status.position.0);
        let _ = self.state.set(status.state);
        let _ = self.homed.set(status.homed);
    }

    async fn go(
        &self,
        ctx: &ActionCtx,
        target: f64,
        unidirectional: bool,
    ) -> Result<FocusStatus, ActionError> {
        let focus = &bound(&self.instrument).focus;
        let opts = options(0.0, unidirectional);
        let status = moving(ctx, focus.move_to(Micrometers(target), opts), focus.stop()).await?;
        self.arrived.emit(status.position.0);
        Ok(status)
    }
}

#[thing_impl]
impl Focus {
    #[on_start]
    async fn start(&self, ctx: ThingCtx) -> anyhow::Result<()> {
        bind(&self.instrument, &ctx)?;
        let focus = this::<Focus>(&ctx)?;
        let follower = follow(bound(&self.instrument).focus.status(), move |status| {
            focus.mirror(status)
        });
        *self.follower.lock().unwrap_or_else(PoisonError::into_inner) = Some(follower);
        Ok(())
    }

    #[on_stop]
    async fn stop_following(&self) {
        stop_task(&self.follower);
    }

    /// Where the focus may go, in µm.
    #[property]
    async fn limits(&self) -> Travel {
        bound(&self.instrument).focus.limits()
    }

    /// Moves to a position, in µm. With `unidirectional`, it arrives going up.
    #[action]
    async fn move_to(
        &self,
        ctx: ActionCtx,
        z: f64,
        #[param(default)] unidirectional: bool,
    ) -> Result<FocusStatus, ActionError> {
        self.go(&ctx, z, unidirectional).await
    }

    /// Moves by a distance from where the focus is, in µm.
    #[action]
    async fn move_by(
        &self,
        ctx: ActionCtx,
        dz: f64,
        #[param(default)] unidirectional: bool,
    ) -> Result<FocusStatus, ActionError> {
        let here = self.position.get();
        self.go(&ctx, here + dz, unidirectional).await
    }

    /// Stops any motion as quickly as the drive can. It doesn't wait for the global lock.
    #[action(global_lock = false)]
    async fn halt(&self) -> Result<(), ActionError> {
        Ok(bound(&self.instrument).focus.stop().await?)
    }
}
