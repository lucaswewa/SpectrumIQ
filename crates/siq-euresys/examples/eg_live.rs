//! Live view from the camera on this PC's grabber into a frame pool, for a while: the frame rate,
//! the frames lost, and the jitter of the grabber's timestamps (ADR-0070).
//!
//! ```text
//! cargo run --release -p siq-euresys --features sdk --example eg_live -- [seconds] [mono8|mono10|mono12] [exposure ms]
//! ```
//!
//! It opens camera 0 of grabber 0 with `config/cameras/adimec-d103a12c.toml` (or `SIQ_EURESYS_MAP`),
//! grabs the full frame free-running, 10 s in `mono12` at 10 ms by default, and puts back the
//! camera's own features as it found them.

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use siq_buffer::{Backend, FillGuard, FramePool, PoolConfig};
use siq_devices::{Camera, CameraSettings, FrameSink, TriggerMode};
use siq_euresys::{EGrabber, EuresysCamera, FeatureMap, Grabber, module_of};
use siq_types::{FrameMeta, Millis, PixelFormat, Roi, SystemClock};

/// The camera's own features the example changes, read first and put back after, in that order.
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

/// Each frame's grabber timestamp, in µs; its slot goes back at once.
#[derive(Default)]
struct Times(Mutex<Vec<u64>>);

impl FrameSink for Times {
    fn frame(&self, slot: FillGuard, meta: FrameMeta) {
        self.0
            .lock()
            .unwrap()
            .push(meta.device_ts.map_or(0, |ts| ts.0));
        drop(slot.publish(meta));
    }
}

async fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let seconds: f64 = args
        .next()
        .map_or(Ok(10.0), |s| s.parse())
        .map_err(|_| "seconds")?;
    let format = match args.next().as_deref() {
        None | Some("mono12") => PixelFormat::Mono12,
        Some("mono10") => PixelFormat::Mono10,
        Some("mono8") => PixelFormat::Mono8,
        Some(other) => return Err(format!("{other}: mono8, mono10 or mono12")),
    };
    let exposure: f64 = args
        .next()
        .map_or(Ok(10.0), |s| s.parse())
        .map_err(|_| "exposure")?;
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
    let info = siq_devices::Device::info(&camera);
    println!(
        "{} {} ({})",
        info.vendor,
        info.model,
        info.serial.as_deref().unwrap_or("?")
    );
    let original = camera.settings();
    let capabilities = camera.capabilities();
    let settings = CameraSettings {
        roi: Roi::full(capabilities.sensor_width, capabilities.sensor_height),
        pixel_format: format,
        exposure: Millis(exposure),
        trigger: TriggerMode::FreeRun,
        ..original
    };
    camera
        .configure(settings)
        .await
        .map_err(|e| e.to_string())?;
    let pool = PoolConfig {
        name: "eg_live".into(),
        slots: 8,
        slot_bytes: settings.frame_bytes(),
        backend: Backend::Heap,
    };
    let pool = FramePool::new(&pool, clock, None).map_err(|e| e.to_string())?;
    let times = Arc::new(Times::default());
    println!(
        "{} × {} {format:?} at {exposure} ms, for {seconds} s",
        settings.roi.width, settings.roi.height
    );
    camera
        .start(pool, times.clone())
        .await
        .map_err(|e| e.to_string())?;
    tokio::time::sleep(Duration::from_secs_f64(seconds)).await;
    camera.stop().await.map_err(|e| e.to_string())?;
    let stats = camera.stats();
    drop(camera);
    put_back(&saved)?;

    let times = times.0.lock().unwrap();
    let gaps: Vec<f64> = times
        .windows(2)
        .map(|w| (w[1] - w[0]) as f64 / 1e3)
        .collect();
    if gaps.is_empty() {
        return Err(format!("{} frames: too few to time", times.len()));
    }
    let mean = gaps.iter().sum::<f64>() / gaps.len() as f64;
    let spread = (gaps.iter().map(|g| (g - mean).powi(2)).sum::<f64>() / gaps.len() as f64).sqrt();
    let worst = gaps.iter().map(|g| (g - mean).abs()).fold(0.0, f64::max);
    println!(
        "{} frames at {:.2} frames/s: a frame every {mean:.3} ms, jitter {spread:.3} ms rms, {worst:.3} ms at worst",
        times.len(),
        1e3 / mean
    );
    println!(
        "lost: {} for want of a slot, {} by the grabber",
        stats.lost_no_slot, stats.lost_device
    );
    Ok(())
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("eg_live: {error}");
            ExitCode::FAILURE
        }
    }
}
