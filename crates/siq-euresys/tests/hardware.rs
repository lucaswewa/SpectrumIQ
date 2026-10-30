//! The camera on the grabber of this PC (ADR-0070), with the `sdk` feature: 100 full frames
//! free-running, in sequence and evenly timed by the grabber's clock, with none lost; one frame
//! for each software trigger; and a strobe for each (P10.4, ADR-0072). Each test reads the
//! camera's own features first, and the grabber's it changes, and puts them back after.
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
    open_on(Arc::new(SystemClock::new()))
}

/// [`open`], timed by `clock`.
fn open_on(clock: Arc<dyn siq_types::Clock>) -> EuresysCamera<EGrabber> {
    let map = std::env::var_os("SIQ_EURESYS_MAP").map_or_else(
        || {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../config/cameras/adimec-d103a12c.toml")
        },
        PathBuf::from,
    );
    let map = FeatureMap::load(&map).unwrap();
    EuresysCamera::new(grabber().unwrap(), map, clock).unwrap()
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

/// REQ-ACQ-003: the full frame, two regions about the centre, 2 × 2 binning, and the full frame at
/// 150 ms, each at the rate the map's readout model gives, within 5 %, with none lost (P10.2).
#[tokio::test]
#[ignore = "hardware"]
async fn each_mode_reaches_the_rate_its_model_gives() {
    let Some(saved) = saved() else { return };
    let camera = open();
    let original = camera.settings();
    let capabilities = camera.capabilities().clone();
    let (width, height) = (capabilities.sensor_width, capabilities.sensor_height);
    let centred = |side: u32| Roi {
        offset_x: (width - side) / 4 * 2,
        offset_y: (height - side) / 4 * 2,
        width: side,
        height: side,
    };
    let full = Roi::full(width, height);
    // The last, longer than the readout: a period no longer than it would skip every other frame.
    let modes = [
        (full, siq_types::Binning::NONE, 1.0),
        (centred(2048), siq_types::Binning::NONE, 1.0),
        (centred(1024), siq_types::Binning::NONE, 1.0),
        (full, siq_types::Binning::new(2, 2).unwrap(), 1.0),
        (full, siq_types::Binning::NONE, 150.0),
    ];
    let mut missed = Vec::new();
    for (roi, binning, exposure) in modes {
        let settings = CameraSettings {
            roi,
            binning,
            pixel_format: PixelFormat::Mono12,
            exposure: Millis(exposure),
            trigger: TriggerMode::FreeRun,
            ..original
        };
        camera.configure(settings).await.unwrap();
        let record = Arc::new(Record::default());
        let lost = |camera: &EuresysCamera<EGrabber>| {
            let stats = camera.stats();
            stats.lost_no_slot + stats.lost_device
        };
        let before = lost(&camera);
        camera.start(pool(&settings), record.clone()).await.unwrap();
        tokio::time::sleep(Duration::from_secs(3)).await;
        camera.stop().await.unwrap();
        let frames = record.0.lock().unwrap();
        let times: Vec<f64> = frames
            .iter()
            .map(|meta| meta.device_ts.unwrap().0 as f64)
            .collect();
        let rate = (times.len() - 1) as f64 / (times[times.len() - 1] - times[0]) * 1e6;
        let model = capabilities.frame_rate.max_rate(&settings).0;
        let lost = lost(&camera) - before;
        let (across, down) = binning.factors();
        eprintln!(
            "{} × {}, binned {across} × {down}: {rate:.2} frames/s, the model {model:.2}, {lost} lost",
            roi.width, roi.height
        );
        if (rate - model).abs() > 0.05 * model || lost > 0 {
            missed.push(format!(
                "{} × {}: {rate:.2} of {model:.2}, {lost} lost",
                roi.width, roi.height
            ));
        }
    }
    drop(camera);
    put_back(&saved);
    assert!(missed.is_empty(), "{missed:?}");
}

/// The middle of each exposure, on the host's clock, follows the grabber's stamps (P10.2): after
/// the first frames, consecutive frames are as far apart in it as their stamps are, the host's
/// wake-ups' jitter left out.
#[tokio::test]
#[ignore = "hardware"]
async fn each_exposures_middle_follows_the_grabbers_stamps() {
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
    wait_for(&record, 70, Duration::from_secs(30)).await;
    camera.stop().await.unwrap();
    drop(camera);
    put_back(&saved);

    let frames = record.0.lock().unwrap();
    let apart = |a: &FrameMeta, b: &FrameMeta| {
        let host = (b.host_ts_mid_exposure.0 as f64 - a.host_ts_mid_exposure.0 as f64) / 1e6;
        let stamps = (b.device_ts.unwrap().0 as f64 - a.device_ts.unwrap().0 as f64) / 1e3;
        (host - stamps).abs()
    };
    let worst = frames[10..]
        .windows(2)
        .map(|pair| apart(&pair[0], &pair[1]))
        .fold(0.0, f64::max);
    eprintln!(
        "{} frames: the host's times and the stamps apart by {worst:.4} ms at worst",
        frames.len()
    );
    assert!(
        worst < 0.25,
        "the frames' times drift from the stamps by {worst:.3} ms"
    );
}

/// What the strobe changes on the grabber, to put back after: its camera controller as `RC`
/// shows it, the line's source, and the line input tool that reads the line back. In the order
/// they are put back, a selector before what it selects.
fn saved_strobe(line: &str) -> Vec<(String, String)> {
    let mut g = grabber().unwrap();
    let mut kept: Vec<(String, String)> = Vec::new();
    let mut read = |g: &mut EGrabber, feature: &str| {
        let (module, name) = module_of(feature);
        let value = g.get(module, name).unwrap();
        kept.push((feature.to_owned(), value.clone()));
        value
    };
    let set = |g: &mut EGrabber, feature: &str, value: &str| {
        let (module, name) = module_of(feature);
        g.set(module, name, value).unwrap();
    };
    let method = g.get(Module::Device, "CameraControlMethod").unwrap();
    set(&mut g, "device:CameraControlMethod", "RC");
    read(&mut g, "device:CameraControlMethod");
    for feature in [
        "CycleTriggerSource",
        "ExposureTime",
        "StrobeDelay",
        "StrobeDuration",
    ] {
        read(&mut g, &format!("device:{feature}"));
    }
    read(&mut g, "device:EventSelector");
    set(&mut g, "device:CameraControlMethod", &method);
    read(&mut g, "device:CameraControlMethod");
    let selected = read(&mut g, "interface:LineSelector");
    set(&mut g, "interface:LineSelector", line);
    read(&mut g, "interface:LineSelector");
    read(&mut g, "interface:LineSource");
    let tool = read(&mut g, "interface:LineInputToolSelector");
    set(&mut g, "interface:LineInputToolSelector", "LIN1");
    read(&mut g, "interface:LineInputToolSelector");
    read(&mut g, "interface:LineInputToolSource");
    read(&mut g, "interface:LineInputToolActivation");
    read(&mut g, "interface:EventSelector");
    set(&mut g, "interface:LineSelector", &selected);
    set(&mut g, "interface:LineInputToolSelector", &tool);
    kept.extend([
        ("interface:LineSelector".to_owned(), selected),
        ("interface:LineInputToolSelector".to_owned(), tool),
    ]);
    kept
}

/// The grabber's counts of the camera's triggers, the strobes, and the line tool's rising edges.
fn strobe_counts(g: &mut EGrabber) -> [u64; 3] {
    let count = |g: &mut EGrabber, module, event| {
        g.set(module, "EventSelector", event).unwrap();
        g.get(module, "EventCount").unwrap().parse::<u64>().unwrap()
    };
    [
        count(g, Module::Device, "CameraTriggerRisingEdge"),
        count(g, Module::Device, "StrobeRisingEdge"),
        count(g, Module::Interface, "LIN1"),
    ]
}

// P10.4 (ADR-0072): each software trigger strobes the line once, read back through a line input
// tool with no wiring; and its exposure starts within 2 ms of the call, after the first frames
// have settled the clocks' offset.
#[tokio::test]
#[ignore = "hardware"]
async fn each_trigger_strobes_its_line_and_exposes_soon_after() {
    let Some(saved) = saved() else { return };
    let line = "IOUT11";
    let strobed = saved_strobe(line);
    let strobe = siq_euresys::Strobe {
        line: line.to_owned(),
        delay: Millis(0.0),
        duration: None,
    };
    let clock = Arc::new(SystemClock::new());
    let camera = open_on(clock.clone()).with_strobe(strobe).unwrap();
    let settings = CameraSettings {
        roi: Roi {
            offset_x: 4608,
            offset_y: 3584,
            width: 2048,
            height: 2048,
        },
        exposure: Millis(5.0),
        trigger: TriggerMode::Software,
        ..camera.settings()
    };
    camera.configure(settings).await.unwrap();
    let before = camera.with_grabber(|g| {
        g.set(Module::Interface, "LineInputToolSelector", "LIN1")
            .unwrap();
        g.set(Module::Interface, "LineInputToolSource", line)
            .unwrap();
        g.set(Module::Interface, "LineInputToolActivation", "RisingEdge")
            .unwrap();
        strobe_counts(g)
    });
    let record = Arc::new(Record::default());
    camera.start(pool(&settings), record.clone()).await.unwrap();
    let mut called = Vec::new();
    for n in 1..=15 {
        called.push(siq_types::Clock::now(&*clock));
        camera.trigger().await.unwrap();
        wait_for(&record, n, Duration::from_secs(2)).await;
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    camera.stop().await.unwrap();
    let after = camera.with_grabber(strobe_counts);
    drop(camera);
    put_back(&strobed);
    put_back(&saved);

    let counted = [0, 1, 2].map(|k| after[k] - before[k]);
    assert_eq!(counted, [15; 3], "triggers, strobes, and the line's edges");
    let frames = record.0.lock().unwrap().clone();
    let latencies: Vec<f64> = called
        .iter()
        .zip(&frames)
        .skip(5)
        .map(|(call, meta)| {
            let started = meta.host_ts_mid_exposure.0 as i64 - 2_500_000;
            (started - call.0 as i64) as f64 / 1e6
        })
        .collect();
    assert!(
        latencies.iter().all(|ms| (0.0..2.0).contains(ms)),
        "{latencies:?}"
    );
}
