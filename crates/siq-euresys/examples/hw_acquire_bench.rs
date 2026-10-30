//! The camera on this PC's grabber, mode by mode (P10.2, ADR-0070): each mode's frame rate against
//! the feature map's model, its DMA throughput and its losses; and the grabber's timestamps against
//! the host's clock, their drift, and what they mark.
//!
//! ```text
//! cargo run --release -p siq-euresys --features sdk --example hw_acquire_bench -- [seconds a mode]
//! ```
//!
//! Each mode runs free at a 1 ms exposure, 5 s by default. The modes: the full frame in each pixel
//! format, three regions about the centre, and 2 × 2 and 4 × 4 binning. It puts back the camera's
//! own features as it found them.

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use siq_buffer::{Backend, FillGuard, FramePool, PoolConfig};
use siq_devices::{Camera, CameraSettings, FrameSink, TriggerMode};
use siq_euresys::{EGrabber, EuresysCamera, FeatureMap, Grabber, module_of};
use siq_types::{Binning, FrameMeta, Millis, PixelFormat, Roi, SystemClock};

/// The camera's own features the bench changes, read first and put back after, in that order.
const KEPT: [&str; 13] = [
    "PixelFormat",
    "BinningHorizontal",
    "BinningVertical",
    "Width",
    "Height",
    "OffsetX",
    "OffsetY",
    "ExposureMode",
    "TriggerSource",
    // The period before the exposure: the camera shortens an exposure to fit its period.
    "AcquisitionFramePeriod",
    "ExposureTime",
    "Gain",
    "device:CameraControlMethod",
];

fn saved() -> Result<Vec<(String, String)>, String> {
    let mut grabber = EGrabber::open(None, 0, 0).map_err(|e| e.to_string())?;
    let read = |feature: &&str| {
        let (module, name) = module_of(feature);
        Some(((*feature).to_owned(), grabber.get(module, name).ok()?))
    };
    Ok(KEPT.iter().filter_map(read).collect())
}

fn put_back(saved: &[(String, String)]) -> Result<(), String> {
    let mut grabber = EGrabber::open(None, 0, 0).map_err(|e| e.to_string())?;
    let mut set = |feature: &str, value: &str| {
        let (module, name) = module_of(feature);
        grabber.set(module, name, value).map_err(|e| e.to_string())
    };
    set("OffsetX", "0")?;
    set("OffsetY", "0")?;
    saved
        .iter()
        .try_for_each(|(feature, value)| set(feature, value))
}

/// Each frame's grabber timestamp, in µs, with the host's time it arrived, in µs from the start.
struct Times {
    start: Instant,
    frames: Mutex<Vec<(f64, f64)>>,
}

impl FrameSink for Times {
    fn frame(&self, slot: FillGuard, meta: FrameMeta) {
        let host = self.start.elapsed().as_secs_f64() * 1e6;
        let device = meta.device_ts.map_or(0.0, |ts| ts.0 as f64);
        self.frames.lock().unwrap().push((device, host));
        drop(slot.publish(meta));
    }
}

/// What a mode did.
struct Measured {
    frames: usize,
    rate: f64,
    lost: u64,
    /// The host's time, less the grabber's, on average, in ms: when the frame arrived after its stamp.
    after_ms: f64,
    /// The host clock's drift against the grabber's, in ppm.
    drift_ppm: f64,
    /// The least lag of an arrival after its stamp, in ms: the clocks' offset and the least delay.
    least_ms: f64,
    /// How far each arrival is from the fit, in ms: rms and at worst.
    jitter_ms: (f64, f64),
}

fn measure(frames: &[(f64, f64)], lost: u64) -> Option<Measured> {
    let n = frames.len() as f64;
    if frames.len() < 3 {
        return None;
    }
    let (sx, sy) = frames
        .iter()
        .fold((0.0, 0.0), |(x, y), (d, h)| (x + d, y + h));
    let (mx, my) = (sx / n, sy / n);
    let (sxx, sxy) = frames.iter().fold((0.0, 0.0), |(xx, xy), (d, h)| {
        (xx + (d - mx).powi(2), xy + (d - mx) * (h - my))
    });
    let slope = sxy / sxx;
    let residuals: Vec<f64> = frames
        .iter()
        .map(|(d, h)| h - (my + slope * (d - mx)))
        .collect();
    let rms = (residuals.iter().map(|r| r * r).sum::<f64>() / n).sqrt();
    let worst = residuals.iter().fold(0.0f64, |w, r| w.max(r.abs()));
    let span = frames[frames.len() - 1].0 - frames[0].0;
    // The offset between the clocks is arbitrary: the least of the arrivals' lags marks the frame
    // whose wake-up was quickest, and the lags above it say how much later each came.
    let lags: Vec<f64> = frames.iter().map(|(d, h)| h - d).collect();
    let least = lags.iter().copied().fold(f64::INFINITY, f64::min);
    let after = lags.iter().map(|lag| lag - least).sum::<f64>() / n;
    Some(Measured {
        frames: frames.len(),
        rate: (n - 1.0) / span * 1e6,
        lost,
        after_ms: after / 1e3,
        drift_ppm: (slope - 1.0) * 1e6,
        least_ms: least / 1e3,
        jitter_ms: (rms / 1e3, worst / 1e3),
    })
}

async fn run() -> Result<(), String> {
    let seconds: f64 = std::env::args()
        .nth(1)
        .map_or(Ok(5.0), |s| s.parse())
        .map_err(|_| "seconds")?;
    let map = std::env::var_os("SIQ_EURESYS_MAP").map_or_else(
        || {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../config/cameras/adimec-d103a12c.toml")
        },
        PathBuf::from,
    );
    let map = FeatureMap::load(&map).map_err(|e| e.to_string())?;
    let saved = saved()?;
    let grabber = EGrabber::open(None, 0, 0).map_err(|e| e.to_string())?;
    let clock = Arc::new(SystemClock::new());
    let camera = EuresysCamera::new(grabber, map, clock.clone()).map_err(|e| e.to_string())?;
    let capabilities = camera.capabilities().clone();
    let (width, height) = (capabilities.sensor_width, capabilities.sensor_height);
    let full = Roi::full(width, height);
    let centred = |w: u32, h: u32| Roi {
        offset_x: (width - w) / 2 / 2 * 2,
        offset_y: (height - h) / 2 / 2 * 2,
        width: w,
        height: h,
    };
    let bin = |n: u8| Binning::new(n, n).unwrap();
    let modes = [
        ("full, mono12", full, Binning::NONE, PixelFormat::Mono12),
        ("full, mono10", full, Binning::NONE, PixelFormat::Mono10),
        ("full, mono8", full, Binning::NONE, PixelFormat::Mono8),
        (
            "4096 × 3072",
            centred(4096, 3072),
            Binning::NONE,
            PixelFormat::Mono12,
        ),
        (
            "2048 × 2048",
            centred(2048, 2048),
            Binning::NONE,
            PixelFormat::Mono12,
        ),
        (
            "1024 × 1024",
            centred(1024, 1024),
            Binning::NONE,
            PixelFormat::Mono12,
        ),
        ("full, binned 2 × 2", full, bin(2), PixelFormat::Mono12),
        ("full, binned 4 × 4", full, bin(4), PixelFormat::Mono12),
        ("full, 50 ms", full, Binning::NONE, PixelFormat::Mono12),
    ];
    let original = camera.settings();
    let start = Instant::now();
    let mut first_lag = None;
    println!(
        "{:<20} {:>7} {:>9} {:>9} {:>9} {:>5} {:>9} {:>8} {:>13} {:>9}",
        "mode",
        "frames",
        "frames/s",
        "model",
        "MB/s",
        "lost",
        "after ms",
        "ppm",
        "jitter ms",
        "lag ms"
    );
    for (name, roi, binning, pixel_format) in modes {
        let exposure = if name.ends_with("50 ms") { 50.0 } else { 1.0 };
        let settings = CameraSettings {
            roi,
            binning,
            pixel_format,
            exposure: Millis(exposure),
            trigger: TriggerMode::FreeRun,
            ..original
        };
        camera
            .configure(settings)
            .await
            .map_err(|e| format!("{name}: {e}"))?;
        let pool = PoolConfig {
            name: "bench".into(),
            slots: 8,
            slot_bytes: settings.frame_bytes(),
            backend: Backend::Heap,
        };
        let pool = FramePool::new(&pool, clock.clone(), None).map_err(|e| e.to_string())?;
        let times = Arc::new(Times {
            start,
            frames: Mutex::new(Vec::new()),
        });
        let before = camera.stats();
        camera
            .start(pool, times.clone())
            .await
            .map_err(|e| e.to_string())?;
        tokio::time::sleep(Duration::from_secs_f64(seconds)).await;
        camera.stop().await.map_err(|e| e.to_string())?;
        let after = camera.stats();
        let lost =
            after.lost_no_slot + after.lost_device - before.lost_no_slot - before.lost_device;
        let model = capabilities.frame_rate.max_rate(&settings).0;
        let frames = times.frames.lock().unwrap();
        let Some(m) = measure(&frames, lost) else {
            println!("{name:<20} {:>7}: too few to time", frames.len());
            continue;
        };
        let mb = m.rate * settings.frame_bytes() as f64 / 1e6;
        // The least lag, against the first mode's: the same clocks, so a difference is the
        // frame's own, such as its transfer, if the grabber stamps its start.
        let lag = m.least_ms - *first_lag.get_or_insert(m.least_ms);
        println!(
            "{name:<20} {:>7} {:>9.2} {model:>9.2} {mb:>9.0} {:>5} {:>9.2} {:>8.1} {:>6.3} {:>6.3} {lag:>9.2}",
            m.frames, m.rate, m.lost, m.after_ms, m.drift_ppm, m.jitter_ms.0, m.jitter_ms.1
        );
    }
    drop(camera);
    put_back(&saved)?;
    println!("the camera's features put back");
    Ok(())
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("hw_acquire_bench: {error}");
            ExitCode::FAILURE
        }
    }
}
