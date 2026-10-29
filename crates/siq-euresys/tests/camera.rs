//! The camera over a fake grabber (ADR-0070), as the Adimec's feature map names it: its
//! capabilities and settings read from the camera, the region set in binned pixels, the trigger
//! modes, every pool slot filled by "DMA" and requeued only once its last lease drops, frames lost
//! for want of a slot, an incomplete buffer lost and filled again, and every slot free after a stop.
//! REQ-ACQ-001, REQ-ACQ-002, REQ-ACQ-003, REQ-ACQ-005.

// The fake grabber writes into the pool's slots, as DMA does: ADR-0007 allows `unsafe` in
// siq-euresys.
#![allow(unsafe_code)]

use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use siq_buffer::{Backend, FillGuard, FrameLease, FramePool, PoolConfig};
use siq_devices::{Camera, CameraSettings, DeviceError, FrameSink, TriggerMode};
use siq_euresys::{
    Counters, ErrorKind, EuresysCamera, FeatureMap, Grabber, GrabberError, Module, Popped,
};
use siq_types::{Binning, Cfa, FrameMeta, Millis, PixelFormat, Roi, SystemClock};

/// What the test feeds the fake grabber.
#[derive(Default)]
struct Feed {
    /// Frames the camera will expose.
    pending: u32,
    /// The next frame arrives incomplete.
    incomplete: bool,
}

/// A grabber that fills its queued buffers as frames are fed, writing each frame's number in.
struct Fake {
    features: HashMap<String, String>,
    entries: Vec<String>,
    set: Arc<Mutex<Vec<String>>>,
    executed: Arc<Mutex<Vec<String>>>,
    feed: Arc<Mutex<Feed>>,
    buffers: Vec<*mut u8>,
    queue: VecDeque<usize>,
    started: bool,
    counters: Counters,
    frame: u64,
    /// The bytes each frame fills: as the camera's settings say.
    bytes: usize,
}

// SAFETY: the buffers are the pool's; the fake writes them only while they are queued.
unsafe impl Send for Fake {}

impl Grabber for Fake {
    fn get(&mut self, _: Module, feature: &str) -> Result<String, GrabberError> {
        let missing = || GrabberError::new(ErrorKind::Fault, format!("no {feature}"));
        self.features.get(feature).cloned().ok_or_else(missing)
    }

    fn set(&mut self, _: Module, feature: &str, value: &str) -> Result<(), GrabberError> {
        self.set.lock().unwrap().push(format!("{feature}={value}"));
        self.features.insert(feature.into(), value.into());
        Ok(())
    }

    fn execute(&mut self, _: Module, command: &str) -> Result<(), GrabberError> {
        self.executed.lock().unwrap().push(command.into());
        Ok(())
    }

    fn entries(&mut self, _: Module, _: &str) -> Result<Vec<String>, GrabberError> {
        Ok(self.entries.clone())
    }

    unsafe fn announce(&mut self, buffers: &[*mut u8], _: usize) -> Result<(), GrabberError> {
        self.buffers = buffers.to_vec();
        let number = |f: &str| self.features[f].parse::<usize>().unwrap();
        self.bytes = number("Width") * number("Height") * 2;
        self.queue = (0..buffers.len()).collect();
        Ok(())
    }

    fn start(&mut self) -> Result<(), GrabberError> {
        (self.started, self.counters) = (true, Counters::default());
        Ok(())
    }

    fn stop(&mut self) -> Result<(), GrabberError> {
        self.started = false;
        self.queue.clear();
        Ok(())
    }

    fn pop(&mut self, _: Duration) -> Result<Option<Popped>, GrabberError> {
        let mut feed = self.feed.lock().unwrap();
        if !self.started || feed.pending == 0 {
            drop(feed);
            std::thread::sleep(Duration::from_millis(1));
            return Ok(None);
        }
        feed.pending -= 1;
        self.frame += 1;
        let Some(index) = self.queue.pop_front() else {
            self.counters.underrun += 1;
            return Ok(None);
        };
        // SAFETY: the buffer is queued, so the camera lent it to the grabber; it is a slot of at
        // least 8 bytes.
        unsafe { self.buffers[index].cast::<u64>().write(self.frame) };
        self.counters.delivered += 1;
        let incomplete = std::mem::take(&mut feed.incomplete);
        Ok(Some(Popped {
            index,
            bytes: self.bytes,
            timestamp_ns: self.frame * 100_000_000,
            frame_id: self.frame,
            incomplete,
        }))
    }

    fn requeue(&mut self, index: usize) -> Result<(), GrabberError> {
        assert!(!self.queue.contains(&index), "buffer {index} queued twice");
        self.queue.push_back(index);
        Ok(())
    }

    fn counters(&mut self) -> Result<Counters, GrabberError> {
        Ok(self.counters)
    }
}

/// What the fake was set, or executed, in order.
type Log = Arc<Mutex<Vec<String>>>;
/// The frames fed to the fake.
type Fed = Arc<Mutex<Feed>>;

/// The Adimec's map, from the repository.
fn adimec() -> FeatureMap {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config/cameras/adimec-d103a12c.toml");
    FeatureMap::load(&path).unwrap()
}

/// The Adimec, as the probe read it in P10.1, at a 10 ms exposure.
fn fake() -> (Fake, Log, Log, Fed) {
    let features = [
        ("DeviceModelName", "D-103A12c/CXP-T01"),
        ("DeviceVendorName", "Adimec"),
        ("DeviceSerialNumber", "336440"),
        ("SensorWidth", "11264"),
        ("SensorHeight", "9200"),
        ("PixelFormat", "BayerGB12"),
        ("BinningHorizontal", "1"),
        ("BinningVertical", "1"),
        ("BinningHorizontal.Max", "4"),
        ("BinningVertical.Max", "4"),
        ("Width.Inc", "1"),
        ("Height.Inc", "1"),
        ("OffsetX.Inc", "1"),
        ("OffsetY.Inc", "1"),
        ("Width.Min", "256"),
        ("Height.Min", "32"),
        ("ExposureTime.Min", "1.0"),
        ("ExposureTime.Max", "2.1e7"),
        ("OffsetX", "0"),
        ("OffsetY", "0"),
        ("Width", "11264"),
        ("Height", "9200"),
        ("ExposureTime", "10000.0"),
        ("Gain", "1.0"),
    ];
    let (set, executed, feed) = Default::default();
    let fake = Fake {
        features: features.map(|(k, v)| (k.to_owned(), v.to_owned())).into(),
        entries: ["BayerGB8", "BayerGB10", "BayerGB12"]
            .map(str::to_owned)
            .into(),
        set: Arc::clone(&set),
        executed: Arc::clone(&executed),
        feed: Arc::clone(&feed),
        buffers: Vec::new(),
        queue: VecDeque::new(),
        started: false,
        counters: Counters::default(),
        frame: 0,
        bytes: 0,
    };
    (fake, set, executed, feed)
}

fn camera() -> (EuresysCamera<Fake>, Log, Log, Fed) {
    let (fake, set, executed, feed) = fake();
    let camera = EuresysCamera::new(fake, adimec(), Arc::new(SystemClock::new())).unwrap();
    (camera, set, executed, feed)
}

/// A frame of 256 × 32 at (1, 1): small enough for small slots, and odd, for the pattern.
fn small(camera: &EuresysCamera<Fake>) -> CameraSettings {
    CameraSettings {
        roi: Roi {
            offset_x: 1,
            offset_y: 1,
            width: 256,
            height: 32,
        },
        ..camera.settings()
    }
}

/// Keeps every frame it is given.
#[derive(Default)]
struct Keep(Mutex<Vec<FrameLease>>);

impl FrameSink for Keep {
    fn frame(&self, slot: FillGuard, meta: FrameMeta) {
        self.0.lock().unwrap().push(slot.publish(meta).unwrap());
    }
}

fn eventually(what: &str, done: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !done() {
        assert!(Instant::now() < deadline, "never: {what}");
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn feed(feed: &Mutex<Feed>, frames: u32) {
    feed.lock().unwrap().pending += frames;
}

#[test]
fn the_capabilities_and_settings_come_from_the_camera_as_its_map_names_them() {
    let (camera, set, _, _) = camera();
    let capabilities = camera.capabilities();
    assert_eq!(
        (capabilities.sensor_width, capabilities.sensor_height),
        (11264, 9200)
    );
    let formats = [PixelFormat::Mono8, PixelFormat::Mono10, PixelFormat::Mono12];
    assert_eq!(capabilities.pixel_formats, formats);
    assert_eq!(capabilities.cfa, Some(Cfa::Gbrg));
    assert_eq!(capabilities.binnings.len(), 16);
    assert_eq!(capabilities.pixel_size.0, 3.2);
    assert_eq!(
        (capabilities.exposure_min, capabilities.exposure_max),
        (Millis(0.001), Millis(21_000.0))
    );
    assert_eq!(
        (capabilities.roi.min_width, capabilities.roi.min_height),
        (256, 32)
    );
    let settings = camera.settings();
    assert_eq!(
        (settings.pixel_format, settings.exposure),
        (PixelFormat::Mono12, Millis(10.0))
    );
    assert_eq!(settings.roi, Roi::full(11264, 9200));
    // Free-running, at the full frame's readout: 9200 rows of 8.85 µs, longer than 10 ms.
    let set = set.lock().unwrap();
    assert!(set.contains(&"ExposureMode=Timed".into()), "{set:?}");
    let period = set
        .iter()
        .rev()
        .find(|s| s.starts_with("AcquisitionFramePeriod="))
        .unwrap();
    let us: f64 = period.split('=').nth(1).unwrap().parse().unwrap();
    assert!((us - 81_420.0).abs() < 1.0, "{period}");
}

#[test]
fn a_map_for_another_model_or_without_the_pixel_pitch_is_refused() {
    let map = FeatureMap {
        model: Some("another camera".into()),
        ..adimec()
    };
    let refused = EuresysCamera::new(fake().0, map, Arc::new(SystemClock::new()));
    assert!(
        matches!(refused, Err(DeviceError::InvalidArgument(why)) if why.contains("another camera"))
    );
    let refused = EuresysCamera::new(
        fake().0,
        FeatureMap::default(),
        Arc::new(SystemClock::new()),
    );
    assert!(
        matches!(refused, Err(DeviceError::InvalidArgument(why)) if why.contains("pixel_size_um"))
    );
}

#[tokio::test]
async fn the_region_is_set_in_binned_pixels_and_only_the_timing_changes_while_acquiring() {
    let (camera, set, executed, _) = camera();
    set.lock().unwrap().clear();
    let settings = CameraSettings {
        roi: Roi {
            offset_x: 1024,
            offset_y: 512,
            width: 4096,
            height: 2048,
        },
        binning: Binning::new(2, 2).unwrap(),
        pixel_format: PixelFormat::Mono10,
        trigger: TriggerMode::Software,
        ..camera.settings()
    };
    camera.configure(settings).await.unwrap();
    let expected = [
        "PixelFormat=BayerGB10",
        "BinningHorizontal=2",
        "BinningVertical=2",
        "OffsetX=0",
        "OffsetY=0",
        "Width=2048",
        "Height=1024",
        "OffsetX=512",
        "OffsetY=256",
        "ExposureMode=TimedTriggerControl",
        "TriggerSource=Trigger",
        "CameraControlMethod=RC",
        "CycleTriggerSource=StartCycle",
        "ExposureTime=10000",
        "Gain=1",
    ];
    assert_eq!(*set.lock().unwrap(), expected);

    // Triggered while acquiring, and nothing but the timing changes then.
    assert!(camera.trigger().await.is_err(), "not acquiring yet");
    let pool = pool(2, settings.frame_bytes());
    camera.start(pool, Arc::new(Keep::default())).await.unwrap();
    // Again and again, while the camera's thread takes the grabber for each pop: it steps aside.
    for _ in 0..10 {
        camera.trigger().await.unwrap();
    }
    assert_eq!(*executed.lock().unwrap(), ["StartCycle"; 10]);
    let longer = CameraSettings {
        exposure: Millis(20.0),
        ..settings
    };
    camera.configure(longer).await.unwrap();
    let moved = CameraSettings {
        binning: Binning::NONE,
        ..longer
    };
    assert!(matches!(
        camera.configure(moved).await,
        Err(DeviceError::Busy(_))
    ));
    camera.stop().await.unwrap();
}

fn pool(slots: u16, bytes: usize) -> FramePool {
    let config = PoolConfig {
        name: "test".into(),
        slots,
        slot_bytes: bytes,
        backend: Backend::Heap,
    };
    FramePool::new(&config, Arc::new(SystemClock::new()), None).unwrap()
}

#[tokio::test]
async fn every_slot_is_filled_and_requeued_only_once_its_last_lease_drops() {
    let (camera, _, _, fed) = camera();
    let settings = small(&camera);
    camera.configure(settings).await.unwrap();
    let pool = pool(3, settings.frame_bytes());
    let kept = Arc::new(Keep::default());
    camera.start(pool.clone(), kept.clone()).await.unwrap();
    assert!(
        matches!(
            camera.start(pool.clone(), kept.clone()).await,
            Err(DeviceError::Busy(_))
        ),
        "started twice"
    );

    feed(&fed, 3);
    eventually("three frames", || kept.0.lock().unwrap().len() == 3);
    {
        let frames = kept.0.lock().unwrap();
        for (n, lease) in frames.iter().enumerate() {
            let meta = lease.meta();
            assert_eq!(meta.seq.0, n as u64);
            assert_eq!(meta.device_ts.unwrap().0, (n as u64 + 1) * 100_000);
            // The region starts an odd column and row in: green and red, then blue and green.
            assert_eq!(meta.cfa, Some(Cfa::Grbg));
            // The grabber's write, in the slot the frame is in.
            let written = u64::from_le_bytes(lease.as_bytes()[..8].try_into().unwrap());
            assert_eq!(written, n as u64 + 1);
        }
    }
    // Every slot is held: the next frame is lost for want of one.
    feed(&fed, 1);
    eventually("an underrun", || camera.stats().lost_no_slot == 1);
    assert_eq!(kept.0.lock().unwrap().len(), 3);

    // A slot let go is filled again, by the next frame.
    let released = kept.0.lock().unwrap().remove(1).slot();
    feed(&fed, 1);
    eventually("a fourth frame", || kept.0.lock().unwrap().len() == 3);
    assert_eq!(kept.0.lock().unwrap()[2].slot(), released);
    assert_eq!(camera.stats().delivered, 4);

    camera.stop().await.unwrap();
    kept.0.lock().unwrap().clear();
    assert_eq!(pool.stats().free, 3, "every slot back in the pool");
    // Started again, it announces them all again.
    camera.start(pool.clone(), kept.clone()).await.unwrap();
    camera.stop().await.unwrap();
}

#[tokio::test]
async fn an_incomplete_buffer_is_lost_and_filled_again_and_a_held_slot_stops_a_start() {
    let (camera, _, _, fed) = camera();
    let settings = small(&camera);
    camera.configure(settings).await.unwrap();
    let pool = pool(2, settings.frame_bytes());
    let held = pool.try_acquire_free().unwrap();
    let kept = Arc::new(Keep::default());
    let refused = camera.start(pool.clone(), kept.clone()).await;
    assert!(matches!(refused, Err(DeviceError::Busy(why)) if why.contains("1 of the pool's 2")));
    drop(held);

    camera.start(pool.clone(), kept.clone()).await.unwrap();
    fed.lock().unwrap().incomplete = true;
    feed(&fed, 3);
    eventually("two frames", || kept.0.lock().unwrap().len() == 2);
    assert_eq!(camera.stats().lost_device, 1);
    assert_eq!(
        camera.stats().lost_no_slot,
        0,
        "the incomplete buffer was filled again"
    );
    camera.stop().await.unwrap();
}
