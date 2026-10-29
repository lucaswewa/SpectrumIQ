//! The camera on the grabber of this PC (ADR-0070), with the `sdk` feature: 100 full frames
//! free-running, in sequence and evenly timed by the grabber's clock, with none lost; and one
//! frame for each software trigger. Each test reads the camera's own features first, and puts them
//! back after.
//!
//! ```text
//! set SIQ_EURESYS_DEVICE=0:0
//! cargo test --release -p siq-euresys --features sdk --test hardware -- --ignored --test-threads 1
//! ```
//!
//! `SIQ_EURESYS_DEVICE` is the grabber's interface and device index; without it the tests pass
//! without running. The camera's map is `config/cameras/adimec-d103a12c.toml`, or
//! `SIQ_EURESYS_MAP`. REQ-ACQ-001, REQ-ACQ-002, REQ-ACQ-003.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use siq_buffer::{Backend, FillGuard, FramePool, PoolConfig};
use siq_devices::{Camera, CameraSettings, FrameSink, TriggerMode};
use siq_euresys::{EGrabber, EuresysCamera, FeatureMap, Grabber, Module, module_of};
use siq_types::{FrameMeta, Millis, PixelFormat, Roi, SystemClock};

/// The camera's own features a test may change, in the order they are put back.
const KEPT: [&str; 12] = [
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
];

/// The grabber `SIQ_EURESYS_DEVICE` names, if it does.
fn grabber() -> Option<EGrabber> {
    let Ok(device) = std::env::var("SIQ_EURESYS_DEVICE") else {
        eprintln!("SIQ_EURESYS_DEVICE isn't set: skipped");
        return None;
    };
    let (interface, device) = device
        .split_once(':')
        .expect("SIQ_EURESYS_DEVICE is interface:device");
    Some(EGrabber::open(None, interface.parse().unwrap(), device.parse().unwrap()).unwrap())
}

/// The camera's features, as they are before a test: put back by [`put_back`].
fn saved() -> Option<Vec<(String, String)>> {
    let mut grabber = grabber()?;
    let read = |feature: &&str| {
        Some((
            (*feature).to_owned(),
            grabber.get(Module::Remote, feature).ok()?,
        ))
    };
    let mut saved: Vec<_> = KEPT.iter().filter_map(read).collect();
    // The grabber's camera controller, which software triggers use.
    let controller = grabber.get(Module::Device, "CameraControlMethod").unwrap();
    saved.push(("device:CameraControlMethod".to_owned(), controller));
    Some(saved)
}

/// Puts the camera's features back, its offsets cleared first so that its size fits.
fn put_back(saved: &[(String, String)]) {
    let mut grabber = grabber().unwrap();
    for feature in ["OffsetX", "OffsetY"] {
        grabber.set(Module::Remote, feature, "0").unwrap();
    }
    for (feature, value) in saved {
        let (module, name) = module_of(feature);
        grabber.set(module, name, value).unwrap();
    }
}

/// The camera, named by its map: `SIQ_EURESYS_MAP`, or the Adimec's.
fn open() -> EuresysCamera<EGrabber> {
    let map = std::env::var_os("SIQ_EURESYS_MAP").map_or_else(
        || {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../config/cameras/adimec-d103a12c.toml")
        },
        PathBuf::from,
    );
    let map = FeatureMap::load(&map).unwrap();
    EuresysCamera::new(grabber().unwrap(), map, Arc::new(SystemClock::new())).unwrap()
}

/// The metadata of every frame, each slot given back at once.
#[derive(Default)]
struct Record(Mutex<Vec<FrameMeta>>);

impl FrameSink for Record {
    fn frame(&self, slot: FillGuard, meta: FrameMeta) {
        self.0.lock().unwrap().push(meta);
        drop(slot.publish(meta));
    }
}

fn pool(settings: &CameraSettings) -> FramePool {
    let config = PoolConfig {
        name: "hardware".into(),
        slots: 8,
        slot_bytes: settings.frame_bytes(),
        backend: Backend::Heap,
    };
    FramePool::new(&config, Arc::new(SystemClock::new()), None).unwrap()
}

async fn wait_for(record: &Record, frames: usize, within: Duration) {
    let deadline = Instant::now() + within;
    while record.0.lock().unwrap().len() < frames && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test]
#[ignore = "hardware"]
async fn a_hundred_full_frames_come_in_sequence_and_evenly_timed() {
    let Some(saved) = saved() else { return };
    let camera = open();
    let original = camera.settings();
    let capabilities = camera.capabilities().clone();
    let settings = CameraSettings {
        roi: Roi::full(capabilities.sensor_width, capabilities.sensor_height),
        pixel_format: PixelFormat::Mono12,
        exposure: Millis(10.0),
        trigger: TriggerMode::FreeRun,
        ..original
    };
    camera.configure(settings).await.unwrap();
    let record = Arc::new(Record::default());
    camera.start(pool(&settings), record.clone()).await.unwrap();
    wait_for(&record, 100, Duration::from_secs(30)).await;
    camera.stop().await.unwrap();
    let stats = camera.stats();
    drop(camera);
    put_back(&saved);

    let frames = record.0.lock().unwrap();
    assert!(frames.len() >= 100, "{} frames in 30 s", frames.len());
    assert_eq!((stats.lost_no_slot, stats.lost_device), (0, 0), "{stats:?}");
    for (n, meta) in frames.iter().enumerate() {
        assert_eq!(meta.seq.0, n as u64);
        assert_eq!(meta.cfa, capabilities.cfa);
    }
    let times: Vec<u64> = frames
        .iter()
        .map(|meta| meta.device_ts.unwrap().0)
        .collect();
    let gaps: Vec<f64> = times
        .windows(2)
        .map(|pair| (pair[1] - pair[0]) as f64 / 1e3)
        .collect();
    let mean = gaps.iter().sum::<f64>() / gaps.len() as f64;
    let worst = gaps
        .iter()
        .map(|gap| (gap - mean).abs())
        .fold(0.0, f64::max);
    eprintln!(
        "{:.2} frames/s, a frame every {mean:.2} ms, ±{worst:.3} ms at worst",
        1e3 / mean
    );
    assert!(worst < 0.1 * mean, "uneven: ±{worst:.3} ms on {mean:.2} ms");
}

#[tokio::test]
#[ignore = "hardware"]
async fn each_software_trigger_takes_one_frame() {
    let Some(saved) = saved() else { return };
    let camera = open();
    let original = camera.settings();
    let settings = CameraSettings {
        exposure: Millis(5.0),
        trigger: TriggerMode::Software,
        ..original
    };
    camera.configure(settings).await.unwrap();
    let record = Arc::new(Record::default());
    camera.start(pool(&settings), record.clone()).await.unwrap();
    for n in 1..=5 {
        camera.trigger().await.unwrap();
        wait_for(&record, n, Duration::from_secs(2)).await;
    }
    // Nothing comes without a trigger.
    tokio::time::sleep(Duration::from_millis(500)).await;
    camera.stop().await.unwrap();
    drop(camera);
    put_back(&saved);
    assert_eq!(record.0.lock().unwrap().len(), 5);
}

/// The first frame's mean in each phase of its 2 × 2 filter cell, and the frames counted.
#[derive(Default)]
struct Phases(Mutex<(Option<[f64; 4]>, usize)>);

impl FrameSink for Phases {
    fn frame(&self, slot: FillGuard, meta: FrameMeta) {
        let lease = slot.publish(meta).unwrap();
        let mut seen = self.0.lock().unwrap();
        seen.1 += 1;
        if seen.0.is_none() {
            let image = lease.image::<u16>().unwrap();
            let mut sums = [0.0; 4];
            for y in 0..image.height() {
                for (x, value) in image.row(y).iter().enumerate() {
                    sums[(y % 2) * 2 + x % 2] += f64::from(*value);
                }
            }
            let quarter = (image.width() * image.height()) as f64 / 4.0;
            seen.0 = Some(sums.map(|sum| sum / quarter));
        }
    }
}

#[tokio::test]
#[ignore = "hardware"]
async fn a_binned_region_comes_at_its_size() {
    let Some(saved) = saved() else { return };
    let camera = open();
    let original = camera.settings();
    let roi = Roi {
        offset_x: 1024,
        offset_y: 512,
        width: 4096,
        height: 2048,
    };
    for bin in [1, 2] {
        let settings = CameraSettings {
            roi,
            binning: siq_types::Binning::new(bin, bin).unwrap(),
            pixel_format: PixelFormat::Mono12,
            exposure: Millis(10.0),
            trigger: TriggerMode::FreeRun,
            ..original
        };
        camera.configure(settings).await.unwrap();
        let phases = Arc::new(Phases::default());
        let lost = camera.stats().lost_device;
        camera.start(pool(&settings), phases.clone()).await.unwrap();
        tokio::time::sleep(Duration::from_secs(2)).await;
        camera.stop().await.unwrap();
        let (means, frames) = *phases.0.lock().unwrap();
        let rate = frames as f64 / 2.0;
        eprintln!("{bin} × {bin}: {frames} frames, about {rate:.0}/s; the phases' means {means:?}");
        assert!(frames > 5, "{frames} frames");
        assert_eq!(
            camera.stats().lost_device,
            lost,
            "a frame of the wrong size"
        );
    }
    drop(camera);
    put_back(&saved);
}
