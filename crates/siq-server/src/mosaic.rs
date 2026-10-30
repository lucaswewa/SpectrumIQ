//! The `mosaic_live` Thing (ADR-0059): the live mosaic, painted from the frames as the stage
//! moves, and the part of it the client looks at, streamed as JPEGs.
//!
//! - **Painting** runs while `enabled`, on a thread of its own: a latest-only consumer of the
//!   frames, so a slow GPU skips frames and never holds up the camera. A frame with a pose and a
//!   calibration, settled or moving with its pose within `max_uncertainty_px` (2 canvas pixels),
//!   is binned to the canvas's scale and corrected by the dark and flat in force on the frame-path pool, then
//!   painted on the GPU's thread where its calibration and pose put it.
//! - **The view** (`viewport`) is rendered in the display's window, the newest frame's if
//!   automatic, at most 10 times a second and only when something changed, into `view`.
//! - **The canvas follows** the frames' calibration and `canvas_scale`: a change of either clears
//!   it.
//! - **`save_snapshot`** writes what was painted to a new dataset: one OME-Zarr image at canvas
//!   scale, a band of chunks at a time read back from the canvas.
//! - **Without a GPU** there is no live mosaic: enabling it is refused, saying why.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use anyhow::{Context, anyhow};
use jpeg_encoder::{ColorType, Encoder};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use siq_buffer::{FrameLease, Policy, RecvError, Subscriber};
use siq_gpu::lut::Table;
use siq_gpu::{Compute, GpuHandle};
use siq_imgproc::{DisplayMapping, Lut};
use siq_jobs::Priority;
use siq_jobs::kinds::PyramidJob;
use siq_preview::{Mapping, Renderer};
use siq_stitch_live::{
    Canvas, CanvasGeometry, CanvasOptions, PAGE_BYTES, Preparer, Viewport, frame_to_canvas, paints,
};
use siq_storage::zarr::{CHUNK, ChannelInfo, ImageInfo, SampleType, Size5, ZarrImage};
use siq_storage::{Dataset, DatasetKind, NewDataset, Snapshot};
use siq_types::{Affine2, CalibrationId, ImageView, Micrometers, RectPx};
use teta_wot::prelude::{ActionError, MjpegStream, PropertyError, Thing, ThingCtx, thing_impl};
use teta_wot::validate::{LocItem, ValidationError};
use tracing::warn;

use crate::Instrument;
use crate::datasets::DatasetEntry;
use crate::things::{Bound, bind, bound, failed};

/// The most views a second.
const VIEWS_PER_SECOND: f64 = 10.0;
/// How long the painter waits for a frame at a time, so that it notices a stop or a stale view.
const STEP: Duration = Duration::from_millis(20);
/// The view's JPEG quality.
const QUALITY: u8 = 85;
/// The image a snapshot is.
const SNAPSHOT_IMAGE: &str = "mosaic";

/// The live mosaic, and the view of it the client looks at.
#[derive(Thing)]
pub struct MosaicLive {
    /// The viewport, rendered: grey JPEGs in the display's window.
    #[stream]
    view: MjpegStream,
    instrument: Bound,
    shared: OnceLock<Arc<Shared>>,
    painter: Mutex<Option<Painter>>,
}

/// What was painted, on the stage, in µm.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct MosaicBounds {
    /// The least x.
    pub x0: f64,
    /// The least y.
    pub y0: f64,
    /// The greatest x.
    pub x1: f64,
    /// The greatest y.
    pub y1: f64,
}

/// What the live mosaic has done, and the GPU memory it takes.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct MosaicStats {
    /// Frames painted since the service started.
    pub painted: u64,
    /// Frames passed over: moving too fast to place.
    pub passed_over: u64,
    /// How long the last frame took to prepare and paint, in ms.
    pub last_paint_ms: Option<f64>,
    /// Views rendered.
    pub views: u64,
    /// The canvas's pages on the GPU.
    pub pages_on_gpu: usize,
    /// Its pages waiting in RAM, shrunk.
    pub pages_in_ram: usize,
    /// The GPU memory its pages take, in bytes.
    pub vram_bytes: u64,
    /// The GPU memory they may take: `[gpu] mosaic_vram_gb`.
    pub vram_budget_bytes: u64,
}

/// A snapshot saved: its dataset, its image, and the background job building the image's pyramid.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct MosaicSnapshot {
    /// The dataset made.
    pub dataset: DatasetEntry,
    /// The image's name: `mosaic`.
    pub image: String,
    /// The pyramid's job.
    pub pyramid: u64,
}

/// What the Thing and its painter share.
struct Shared {
    canvas: Arc<Mutex<Canvas>>,
    budget: u64,
    /// The canvas's place on the stage, and the calibration it follows: `None` until a frame
    /// paints.
    geometry: Mutex<Option<(CalibrationId, CanvasGeometry)>>,
    viewport: Mutex<Viewport>,
    canvas_scale: Mutex<f64>,
    max_uncertainty_px: Mutex<f64>,
    last_paint_ms: Mutex<Option<f64>>,
    /// The display's mapping of the newest frame painted.
    mapping: Mutex<Option<DisplayMapping>>,
    /// Whether the view is out of date.
    stale: AtomicBool,
    painted: AtomicU64,
    passed_over: AtomicU64,
    views: AtomicU64,
    view: MjpegStream,
}

/// The painter's thread, and its stop.
struct Painter {
    stop: Arc<AtomicBool>,
    thread: JoinHandle<()>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The client's error (HTTP 422), with why.
fn refused(value: impl Serialize, why: &str) -> PropertyError {
    let input = serde_json::to_value(value).unwrap_or_default();
    ValidationError::value_error(vec![LocItem::Key("body".into())], why, input).into()
}

impl Shared {
    /// The canvas's geometry for frames of calibration `id`, `l`, at `scale`: the canvas is
    /// cleared when either changes.
    fn follow(&self, id: CalibrationId, l: &Affine2, scale: f64) -> CanvasGeometry {
        let geometry = CanvasGeometry::for_calibration(l, scale);
        let mut kept = lock(&self.geometry);
        if *kept != Some((id, geometry)) {
            lock(&self.canvas).clear();
            *kept = Some((id, geometry));
        }
        geometry
    }

    /// Renders the view into the stream: nothing until a frame was painted.
    fn render(&self, gpu: &GpuHandle) -> anyhow::Result<()> {
        let (Some((_, geometry)), Some(mapping)) = (*lock(&self.geometry), *lock(&self.mapping))
        else {
            return Ok(());
        };
        let viewport = *lock(&self.viewport);
        let lut = Lut::new(&mapping)?;
        let table: Arc<Table> = Arc::new(std::array::from_fn(|value| lut.level(value as u16)));
        let (view, canvas) = (viewport.on_canvas(&geometry), Arc::clone(&self.canvas));
        let pixels = gpu.run_blocking(move |gpu| lock(&canvas).render(gpu, &view, &table))?;
        let mut jpeg = Vec::with_capacity(pixels.len() / 8);
        // A viewport is checked to be at most 1920 × 1080.
        let (width, height) = (viewport.width as u16, viewport.height as u16);
        Encoder::new(&mut jpeg, QUALITY).encode(&pixels, width, height, ColorType::Luma)?;
        let _ = self.view.add_frame(jpeg);
        self.views.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
}

/// Paints the frames of `frames` on `gpu` until `stop`, rendering the view when it is stale.
fn paint_frames(
    shared: &Shared,
    instrument: &Instrument,
    gpu: &GpuHandle,
    mut frames: Subscriber,
    stop: &AtomicBool,
) {
    let (mut preparer, mut renderer) = (Preparer::default(), Renderer::default());
    let period = Duration::from_secs_f64(1.0 / VIEWS_PER_SECOND);
    let mut shown: Option<Instant> = None;
    while !stop.load(Ordering::Relaxed) {
        match frames.recv_timeout(STEP) {
            Ok(frame) => {
                match paint(shared, instrument, gpu, frame, &mut preparer, &mut renderer) {
                    Ok(Some(took)) => {
                        *lock(&shared.last_paint_ms) = Some(took.as_secs_f64() * 1e3);
                        shared.painted.fetch_add(1, Ordering::Relaxed);
                        shared.stale.store(true, Ordering::Relaxed);
                    }
                    Ok(None) => {
                        shared.passed_over.fetch_add(1, Ordering::Relaxed);
                    }
                    Err(error) => warn!(%error, "a frame wasn't painted"),
                }
            }
            Err(RecvError::Timeout) => {}
            Err(RecvError::Closed) => return,
        }
        if shown.is_none_or(|at| at.elapsed() >= period)
            && shared.stale.swap(false, Ordering::Relaxed)
        {
            if let Err(error) = shared.render(gpu) {
                warn!(%error, "the mosaic's view wasn't rendered");
            }
            shown = Some(Instant::now());
        }
    }
}

/// Paints `frame` where its calibration and pose put it, and how long that took: `None` if it
/// moved too fast to place.
fn paint(
    shared: &Shared,
    instrument: &Instrument,
    gpu: &GpuHandle,
    frame: FrameLease,
    preparer: &mut Preparer,
    renderer: &mut Renderer,
) -> anyhow::Result<Option<Duration>> {
    let began = Instant::now();
    let meta = *frame.meta();
    let (Some(image), Some(pose), Some(id)) = (frame.image::<u16>(), meta.pose, meta.calibration)
    else {
        return Ok(None);
    };
    let record = instrument.calibrations.get(id);
    let l = record.context("the frame's calibration")?.affine();
    let scale = *lock(&shared.canvas_scale);
    let geometry = shared.follow(id, &l, scale);
    let threshold = *lock(&shared.max_uncertainty_px) / geometry.px_per_um;
    if !paints(&meta.pose_quality, Micrometers(threshold)) {
        return Ok(None);
    }
    let shading = instrument.previews.flat().shading;
    let shading = shading.as_deref().map(|(dark, flat)| (*dark, flat));
    let pool = instrument.previews.pool();
    let prepared = pool.install(|| preparer.prepare(&image, &meta, scale, shading))?;
    drop(frame);
    let display = instrument.previews.display();
    // The prepared frame is mono: its colour waits for P10.7.
    let bits = meta.pixel_format.bits();
    let mapping = match renderer.mapping(&prepared.view()?, bits, None, &display) {
        Mapping::Mono(mapping) | Mapping::Colour(_, [_, mapping, _]) => mapping,
    };
    *lock(&shared.mapping) = Some(mapping);
    let size = [prepared.width, prepared.height];
    let at = [pose.x.0, pose.y.0];
    let m = frame_to_canvas(&l, size, prepared.binning, at, &geometry)
        .context("the calibration is singular")?;
    let canvas = Arc::clone(&shared.canvas);
    gpu.run_blocking(move |gpu| lock(&canvas).paint(gpu, prepared.frame(), &m))?;
    Ok(Some(began.elapsed()))
}

/// Writes what `shared`'s canvas holds to a new dataset `name`.
fn snapshot(
    instrument: &Instrument,
    shared: &Shared,
    name: &str,
) -> anyhow::Result<MosaicSnapshot> {
    let nothing = || anyhow!("nothing was painted");
    let (id, geometry) = (*lock(&shared.geometry)).ok_or_else(nothing)?;
    let [x0, y0, x1, y1] = lock(&shared.canvas).bounds().ok_or_else(nothing)?;
    let Compute::Gpu(gpu) = instrument.gpu() else {
        return Err(nothing());
    };
    let (left, top) = (x0.floor() as i64, y0.floor() as i64);
    let (width, height) = (
        (x1.ceil() as i64 - left) as u64,
        (y1.ceil() as i64 - top) as u64,
    );
    let mut new = NewDataset::new(DatasetKind::Mosaic, name);
    new.instrument = Some(Snapshot::of(&instrument.config)?);
    let record = instrument.calibrations.get(id);
    new.calibration = record.map(|record| Snapshot::of(&record)).transpose()?;
    let dataset = Dataset::create(instrument.catalog.data_root(), new)?;
    let path = dataset
        .zarr_dir()
        .join(format!("{SNAPSHOT_IMAGE}.ome.zarr"));
    let (um, corner) = (
        1.0 / geometry.px_per_um,
        geometry.to_stage([left as f64 + 0.5, top as f64 + 0.5]),
    );
    let mapping = *lock(&shared.mapping);
    let window = mapping.map_or((0.0, 65_535.0), |m| (f64::from(m.min), f64::from(m.max)));
    let info = ImageInfo {
        name: SNAPSHOT_IMAGE.to_owned(),
        sample: SampleType::U16,
        size: Size5 {
            t: 1,
            c: 1,
            z: 1,
            y: height,
            x: width,
        },
        levels: 1,
        chunk: CHUNK,
        pixel_size: [1.0, um, um],
        origin: [0.0, corner[1], corner[0]],
        channels: vec![ChannelInfo {
            label: "live mosaic".to_owned(),
            colour: [255; 3],
            window,
        }],
        cfa: None,
    };
    let image = ZarrImage::create(&path, info)?;
    for row in 0..height.div_ceil(CHUNK) {
        let rows = CHUNK.min(height - row * CHUNK);
        let (canvas, origin) = (
            Arc::clone(&shared.canvas),
            [left, top + (row * CHUNK) as i64],
        );
        let size = [width as u32, rows as u32];
        let words = gpu.run_blocking(move |gpu| lock(&canvas).region(gpu, origin, size))?;
        let values: Vec<u16> = words.iter().map(|&word| word as u16).collect();
        let band = ImageView::new(&values, width as usize, rows as usize, width as usize)?;
        for column in 0..width.div_ceil(CHUNK) {
            let x = column * CHUNK;
            let rect = RectPx::new(x as u32, 0, CHUNK.min(width - x) as u32, rows as u32);
            image.write_chunk(0, (0, 0, 0), (row, column), band.sub_view(rect)?)?;
        }
    }
    let folder = Some(dataset.path().to_owned());
    let pyramid = instrument.jobs.submit(
        Box::new(PyramidJob::new(&path)),
        Priority::Interactive,
        folder,
    );
    instrument.catalog.record(dataset.path())?;
    let summary = instrument
        .catalog
        .get(dataset.manifest().id)
        .context("the dataset made")?;
    Ok(MosaicSnapshot {
        dataset: DatasetEntry::of(&summary),
        image: SNAPSHOT_IMAGE.to_owned(),
        pyramid: pyramid.0,
    })
}

impl MosaicLive {
    fn shared(&self) -> &Arc<Shared> {
        self.shared
            .get()
            .expect("a Thing is used only once it has started")
    }

    async fn start_painting(&self) -> Result<(), PropertyError> {
        let instrument = self
            .instrument
            .get()
            .context("not started")
            .map_err(failed)?;
        let instrument = Arc::clone(instrument);
        let opening = Arc::clone(&instrument);
        let compute = tokio::task::spawn_blocking(move || opening.gpu().clone());
        let gpu = match compute.await.map_err(failed)? {
            Compute::Gpu(gpu) => gpu,
            Compute::Cpu { why } => {
                return Err(refused(
                    true,
                    &format!("the live mosaic needs a GPU: {why}"),
                ));
            }
        };
        let mut painter = lock(&self.painter);
        if painter.is_some() {
            return Ok(());
        }
        let frames = instrument
            .acquisition
            .subscribe("mosaic-live", Policy::LatestOnly, |meta| {
                meta.pose.is_some() && meta.calibration.is_some()
            });
        let (stop, shared) = (Arc::new(AtomicBool::new(false)), Arc::clone(self.shared()));
        let stopping = Arc::clone(&stop);
        shared.stale.store(true, Ordering::Relaxed);
        let thread = thread::Builder::new()
            .name("mosaic-live".into())
            .spawn(move || paint_frames(&shared, &instrument, &gpu, frames, &stopping))
            .map_err(failed)?;
        *painter = Some(Painter { stop, thread });
        Ok(())
    }

    /// Stops painting, once the frame being painted is.
    async fn stop_painting(&self) {
        let Some(painter) = lock(&self.painter).take() else {
            return;
        };
        painter.stop.store(true, Ordering::Relaxed);
        let _ = tokio::task::spawn_blocking(move || painter.thread.join()).await;
    }

    /// Marks the view stale: the painter renders it, or, while nothing is painting, this.
    async fn refresh(&self) {
        let shared = Arc::clone(self.shared());
        shared.stale.store(true, Ordering::Relaxed);
        let painting = lock(&self.painter).is_some();
        let opened = bound(&self.instrument).gpu.get().cloned();
        let (false, Some(Compute::Gpu(gpu))) = (painting, opened) else {
            return;
        };
        let rendered = tokio::task::spawn_blocking(move || {
            shared.stale.store(false, Ordering::Relaxed);
            shared.render(&gpu)
        });
        if let Ok(Err(error)) = rendered.await {
            warn!(%error, "the mosaic's view wasn't rendered");
        }
    }
}

#[thing_impl]
impl MosaicLive {
    #[on_start]
    async fn start(&self, ctx: ThingCtx) -> anyhow::Result<()> {
        bind(&self.instrument, &ctx)?;
        let budget = (bound(&self.instrument).config.gpu.mosaic_vram_gb * 1e9) as u64;
        let options = CanvasOptions {
            budget_bytes: budget,
            ..CanvasOptions::default()
        };
        let _ = self.shared.set(Arc::new(Shared {
            canvas: Arc::new(Mutex::new(Canvas::new(options))),
            budget,
            geometry: Mutex::new(None),
            viewport: Mutex::new(Viewport::default()),
            canvas_scale: Mutex::new(0.5),
            max_uncertainty_px: Mutex::new(2.0),
            last_paint_ms: Mutex::new(None),
            mapping: Mutex::new(None),
            stale: AtomicBool::new(false),
            painted: AtomicU64::new(0),
            passed_over: AtomicU64::new(0),
            views: AtomicU64::new(0),
            view: self.view.clone(),
        }));
        Ok(())
    }

    #[on_stop]
    async fn stop(&self) {
        self.stop_painting().await;
        self.view.stop();
    }

    /// Whether the frames are painted as they come. Enabling it opens the GPU the first time,
    /// and is refused without one.
    #[property(global_lock = false)]
    async fn enabled(&self) -> bool {
        lock(&self.painter).is_some()
    }

    #[setter(enabled)]
    async fn set_enabled(&self, value: bool) -> Result<(), PropertyError> {
        if value {
            self.start_painting().await
        } else {
            self.stop_painting().await;
            Ok(())
        }
    }

    /// What the client looks at: the stage point at its centre in µm, its zoom (view pixels a
    /// canvas pixel, from 1/1024 to 64), and its size, at most 1920 × 1080.
    #[property(global_lock = false)]
    async fn viewport(&self) -> Viewport {
        *lock(&self.shared().viewport)
    }

    #[setter(viewport)]
    async fn set_viewport(&self, value: Viewport) -> Result<(), PropertyError> {
        value.check().map_err(|why| refused(value, &why))?;
        *lock(&self.shared().viewport) = value;
        self.refresh().await;
        Ok(())
    }

    /// Canvas pixels a sensor pixel, from 1/64 to 1: ½ by default. Frames are binned to about it,
    /// by a whole factor. Changing it clears the mosaic.
    #[property(global_lock = false)]
    async fn canvas_scale(&self) -> f64 {
        *lock(&self.shared().canvas_scale)
    }

    #[setter(canvas_scale)]
    async fn set_canvas_scale(&self, value: f64) -> Result<(), PropertyError> {
        if !(1.0 / 64.0..=1.0).contains(&value) {
            return Err(refused(value, "the canvas scale is from 1/64 to 1"));
        }
        let shared = self.shared();
        if std::mem::replace(&mut *lock(&shared.canvas_scale), value) != value {
            *lock(&shared.geometry) = None;
            lock(&shared.canvas).clear();
            self.refresh().await;
        }
        Ok(())
    }

    /// How uncertain a moving frame's pose may be for it to paint, in canvas pixels: 2 by
    /// default, M7's placement criterion; 0 for settled frames only. The uncertainty is how far
    /// the stage went between the readings either side, so a frame moving faster than this over
    /// the stage's polling interval waits for the stage to stop.
    #[property(global_lock = false)]
    async fn max_uncertainty_px(&self) -> f64 {
        *lock(&self.shared().max_uncertainty_px)
    }

    #[setter(max_uncertainty_px)]
    async fn set_max_uncertainty_px(&self, value: f64) -> Result<(), PropertyError> {
        if !(0.0..=10_000.0).contains(&value) {
            return Err(refused(
                value,
                "the uncertainty is from 0 to 10 000 canvas pixels",
            ));
        }
        *lock(&self.shared().max_uncertainty_px) = value;
        Ok(())
    }

    /// What was painted, on the stage in µm: `null` while nothing is.
    #[property(global_lock = false)]
    async fn bounds(&self) -> Option<MosaicBounds> {
        let shared = self.shared();
        let (_, geometry) = (*lock(&shared.geometry))?;
        let [x0, y0, x1, y1] = lock(&shared.canvas).bounds()?;
        let ([x0, y0], [x1, y1]) = (geometry.to_stage([x0, y0]), geometry.to_stage([x1, y1]));
        Some(MosaicBounds { x0, y0, x1, y1 })
    }

    /// Canvas pixels a µm, as the frames' calibration and `canvas_scale` make them: `null` until
    /// a frame is painted.
    #[property(global_lock = false)]
    async fn px_per_um(&self) -> Option<f64> {
        lock(&self.shared().geometry).map(|(_, geometry)| geometry.px_per_um)
    }

    /// What the live mosaic has done, and the GPU memory its pages take.
    #[property(global_lock = false)]
    async fn stats(&self) -> MosaicStats {
        let shared = self.shared();
        let (on_gpu, in_ram) = {
            let canvas = lock(&shared.canvas);
            (canvas.on_gpu(), canvas.on_host())
        };
        let count = |counter: &AtomicU64| counter.load(Ordering::Relaxed);
        MosaicStats {
            painted: count(&shared.painted),
            passed_over: count(&shared.passed_over),
            last_paint_ms: *lock(&shared.last_paint_ms),
            views: count(&shared.views),
            pages_on_gpu: on_gpu,
            pages_in_ram: in_ram,
            vram_bytes: on_gpu as u64 * PAGE_BYTES,
            vram_budget_bytes: shared.budget,
        }
    }

    /// Empties the mosaic. It doesn't wait for the global lock.
    #[action(global_lock = false)]
    async fn clear(&self) -> Result<(), ActionError> {
        lock(&self.shared().canvas).clear();
        self.refresh().await;
        Ok(())
    }

    /// Saves what was painted to a new dataset, `name` ("live mosaic" by default): one OME-Zarr
    /// image, `mosaic`, at canvas scale, whose pyramid a background job builds. It doesn't wait
    /// for the global lock.
    #[action(global_lock = false)]
    async fn save_snapshot(
        &self,
        #[param(default)] name: Option<String>,
    ) -> Result<MosaicSnapshot, ActionError> {
        let instrument = Arc::clone(self.instrument.get().context("not started")?);
        let shared = Arc::clone(self.shared());
        let name = name.unwrap_or_else(|| "live mosaic".to_owned());
        let saved = tokio::task::spawn_blocking(move || snapshot(&instrument, &shared, &name));
        Ok(saved.await??)
    }
}
