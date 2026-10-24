//! Live previews (REQ-PRV-001–004, ADR-0032): frames become two JPEG streams, an overview and a
//! region of interest, and statistics, within architecture §4.2's budget.
//!
//! - **Each stream is a latest-only consumer** of the acquisition's fan-out, on its own thread. Its
//!   kernels run on a shared frame-path rayon pool. A slow stream skips frames, and never holds up
//!   the camera.
//! - **Overview:** the whole frame, area-averaged to fit 1920 × 1080.
//! - **ROI:** a region in sensor pixels, at 1:1 or 2:1, clamped to the frame.
//! - **Statistics:** the ROI's histogram, window and focus measure.
//! - **Display mapping is the server's:** a window, gamma and colormap, fixed or from the frame's
//!   percentiles. Only 8-bit JPEGs and statistics leave, never raw frames (REQ-PRV-004).
//! - **Each stream has a rate cap.** Two histograms time its JPEGs: from the frame's mid-exposure
//!   (`latency_ms`), and from taking the frame, the pipeline's own work (`work_ms`).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use rayon::{ThreadPool, ThreadPoolBuilder};
use siq_acquire::Acquisition;
use siq_buffer::{FrameLease, Policy, RecvError, Subscriber};
use siq_devices::watch;
use siq_imgproc::{Histogram, KernelError};
use siq_telemetry::{Histogram as Latency, MetricError, Registry};
use siq_types::{Clock, FrameMeta, ImageError, RectPx};
use tracing::warn;

mod view;

pub use view::{
    Contrast, DisplaySettings, FrameStats, Renderer, RoiSettings, Zoom, fit, measure, roi_region,
};

/// A JPEG stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamKind {
    /// The whole frame, shrunk.
    Overview,
    /// A region of interest, at 1:1 or 2:1.
    Roi,
}

/// How the previews are made.
#[derive(Debug, Clone, PartialEq)]
pub struct PreviewConfig {
    /// The overview's largest size.
    pub overview_size: (u32, u32),
    /// The ROI preview's largest size.
    pub roi_size: (u32, u32),
    /// The JPEG quality, from 1 to 100.
    pub quality: u8,
    /// The most frames a second each stream takes: overview, ROI and statistics.
    pub rates: [f64; 3],
    /// The frame-path pool's threads; 0 for half the cores.
    pub threads: usize,
}

impl Default for PreviewConfig {
    /// 1920 × 1080 at quality 85; 10, 10 and 5 frames a second.
    fn default() -> Self {
        Self {
            overview_size: (1920, 1080),
            roi_size: (1920, 1080),
            quality: 85,
            rates: [10.0, 10.0, 5.0],
            threads: 0,
        }
    }
}

/// Where the JPEGs go: P2.3's MJPEG streams.
pub trait PreviewSink: Send + Sync {
    /// A frame's JPEG for `stream`.
    fn push_jpeg(&self, stream: StreamKind, jpeg: Vec<u8>, meta: &FrameMeta);
}

/// Where the statistics go.
pub trait StatsSink: Send + Sync {
    /// A frame's statistics.
    fn push_stats(&self, stats: &FrameStats);
}

/// Why a preview couldn't be made.
#[derive(Debug, thiserror::Error)]
pub enum PreviewError {
    /// A kernel refused its arguments.
    #[error(transparent)]
    Kernel(#[from] KernelError),
    /// An image view didn't fit.
    #[error(transparent)]
    Image(#[from] ImageError),
    /// The JPEG encoder failed.
    #[error("the JPEG encoder: {0}")]
    Jpeg(#[from] jpeg_encoder::EncodingError),
    /// A JPEG can't be this large.
    #[error("a {0} × {1} preview is larger than a JPEG can be")]
    TooLarge(u32, u32),
    /// The frame-path pool or a stream's thread couldn't start.
    #[error("the previews' threads: {0}")]
    Threads(String),
    /// A metric's name is taken by another kind.
    #[error(transparent)]
    Metric(#[from] MetricError),
}

/// The latency histogram's bounds, in ms.
const LATENCY: [f64; 8] = [10.0, 20.0, 35.0, 60.0, 100.0, 200.0, 500.0, 1000.0];

/// How long a stream's thread waits at a time, so that it notices a stop.
const STEP: Duration = Duration::from_millis(50);

/// The running previews.
pub struct Previews {
    display: Arc<watch::Sender<DisplaySettings>>,
    roi: Arc<watch::Sender<RoiSettings>>,
    stop: Arc<AtomicBool>,
    threads: Vec<JoinHandle<()>>,
}

/// A JPEG stream's work: one frame at a time, into its sink.
struct Jpegs {
    kind: StreamKind,
    sink: Arc<dyn PreviewSink>,
    display: Arc<watch::Sender<DisplaySettings>>,
    roi: Arc<watch::Sender<RoiSettings>>,
    size: (u32, u32),
    quality: u8,
    /// From the frame's mid-exposure, and from taking it, to its JPEG.
    latency: Option<[Arc<Latency>; 2]>,
    clock: Arc<dyn Clock>,
    renderer: Renderer,
}

impl Jpegs {
    fn preview(&mut self, frame: &FrameLease) -> Result<(), PreviewError> {
        let (meta, Some(image)) = (frame.meta(), frame.image::<u16>()) else {
            return Ok(());
        };
        let taken = self.clock.now();
        let renderer = &mut self.renderer;
        let mapping = renderer.mapping(&image, meta.pixel_format.bits(), &self.display.borrow());
        let (region, size) = match self.kind {
            StreamKind::Overview => {
                let (width, height) = meta.image_size();
                (
                    RectPx::new(0, 0, width, height),
                    fit((width, height), self.size),
                )
            }
            StreamKind::Roi => roi_region(meta, &self.roi.borrow(), self.size),
        };
        renderer.show(&image, region, size, mapping)?;
        let jpeg = renderer.encode(self.quality)?;
        if let Some([exposed, work]) = &self.latency {
            let now = self.clock.now();
            let ms = |since| now.saturating_since(since).as_secs_f64() * 1e3;
            exposed.record(ms(meta.host_ts_mid_exposure));
            work.record(ms(taken));
        }
        self.sink.push_jpeg(self.kind, jpeg, meta);
        Ok(())
    }
}

impl Previews {
    /// Starts the overview, ROI and statistics streams on `acquisition`'s frames, into `previews`
    /// and `stats`. Latencies are measured on `clock`, and recorded in `registry` if given.
    pub fn start(
        acquisition: &Acquisition,
        config: &PreviewConfig,
        previews: Arc<dyn PreviewSink>,
        stats: Arc<dyn StatsSink>,
        clock: Arc<dyn Clock>,
        registry: Option<&Registry>,
    ) -> Result<Self, PreviewError> {
        let cores = thread::available_parallelism().map_or(2, |n| n.get());
        let threads = match config.threads {
            0 => (cores / 2).max(1),
            n => n,
        };
        let pool = ThreadPoolBuilder::new()
            .num_threads(threads)
            .thread_name(|i| format!("frame-path-{i}"));
        let pool = pool
            .build()
            .map_err(|error| PreviewError::Threads(error.to_string()))?;
        let pool = Arc::new(pool);
        let display = Arc::new(watch::Sender::new(DisplaySettings::default()));
        let roi = Arc::new(watch::Sender::new(RoiSettings::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let jpegs = |kind, size, name: &str| -> Result<Jpegs, PreviewError> {
            let metric = |what: &str| {
                registry.map(|r| r.histogram(&format!("preview.{name}.{what}"), &LATENCY))
            };
            let (exposed, work) = (
                metric("latency_ms").transpose()?,
                metric("work_ms").transpose()?,
            );
            Ok(Jpegs {
                kind,
                sink: previews.clone(),
                display: display.clone(),
                roi: roi.clone(),
                size,
                quality: config.quality,
                latency: exposed.zip(work).map(|(exposed, work)| [exposed, work]),
                clock: clock.clone(),
                renderer: Renderer::default(),
            })
        };
        let mut overview = jpegs(StreamKind::Overview, config.overview_size, "overview")?;
        let mut region = jpegs(StreamKind::Roi, config.roi_size, "roi")?;
        let (rois, size, mut counts) = (roi.clone(), config.roi_size, Box::new(Histogram::new(16)));
        let statistics = move |frame: &FrameLease| -> Result<(), PreviewError> {
            let (meta, Some(image)) = (frame.meta(), frame.image::<u16>()) else {
                return Ok(());
            };
            let (rect, _) = roi_region(meta, &rois.borrow(), size);
            let bits = meta.pixel_format.bits();
            stats.push_stats(&measure(&image, meta, bits, rect, &mut counts)?);
            Ok(())
        };

        let subscribe = |name: &str| {
            acquisition.subscribe(format!("preview.{name}"), Policy::LatestOnly, |_| true)
        };
        let [overview_fps, roi_fps, stats_fps] = config.rates;
        let threads = [
            spawn(
                "overview",
                subscribe("overview"),
                overview_fps,
                &stop,
                &pool,
                move |frame| overview.preview(frame),
            ),
            spawn(
                "roi",
                subscribe("roi"),
                roi_fps,
                &stop,
                &pool,
                move |frame| region.preview(frame),
            ),
            spawn(
                "stats",
                subscribe("stats"),
                stats_fps,
                &stop,
                &pool,
                statistics,
            ),
        ];
        let threads = threads.into_iter().collect::<Result<_, _>>()?;
        Ok(Self {
            display,
            roi,
            stop,
            threads,
        })
    }

    /// Sets how frames show, from the next preview.
    pub fn set_display(&self, settings: DisplaySettings) {
        self.display.send_replace(settings);
    }

    /// How frames show.
    pub fn display(&self) -> DisplaySettings {
        *self.display.borrow()
    }

    /// Sets the ROI stream's region and zoom, from the next preview.
    pub fn set_roi(&self, settings: RoiSettings) {
        self.roi.send_replace(settings);
    }

    /// The ROI stream's region and zoom.
    pub fn roi(&self) -> RoiSettings {
        *self.roi.borrow()
    }
}

impl Drop for Previews {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        for thread in self.threads.drain(..) {
            let _ = thread.join();
        }
    }
}

/// Runs `work` on each frame `frames` takes, at most `fps` a second, on `pool`, until `stop`.
fn spawn(
    name: &str,
    mut frames: Subscriber,
    fps: f64,
    stop: &Arc<AtomicBool>,
    pool: &Arc<ThreadPool>,
    mut work: impl FnMut(&FrameLease) -> Result<(), PreviewError> + Send + 'static,
) -> Result<JoinHandle<()>, PreviewError> {
    let period = if fps > 0.0 {
        Duration::from_secs_f64(1.0 / fps)
    } else {
        Duration::ZERO
    };
    let (stop, pool) = (stop.clone(), pool.clone());
    let stream = name.to_owned();
    let run = move || {
        let mut next = Instant::now();
        while !stop.load(Ordering::Relaxed) {
            // The rate cap: no frame is taken before its turn, so the mailbox keeps the newest.
            let now = Instant::now();
            if now < next {
                thread::sleep((next - now).min(STEP));
                continue;
            }
            match frames.recv_timeout(STEP) {
                Ok(frame) => {
                    next = Instant::now() + period;
                    if let Err(error) = pool.install(|| work(&frame)) {
                        warn!(stream, %error, "a preview failed");
                    }
                }
                Err(RecvError::Timeout) => {}
                Err(RecvError::Closed) => return,
            }
        }
    };
    let thread = thread::Builder::new()
        .name(format!("preview-{name}"))
        .spawn(run);
    thread.map_err(|error| PreviewError::Threads(error.to_string()))
}
