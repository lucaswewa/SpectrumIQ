//! The camera (ADR-0070): a [`Camera`] over any [`Grabber`]. Every pool slot is a grabber
//! buffer, filled by DMA. One thread pops the filled buffers and hands them on as frames, and
//! requeues each slot once its last lease drops: a slot is queued only while the camera holds it,
//! so the grabber never writes a frame that is being read.

use std::collections::VecDeque;
use std::future::ready;
use std::iter;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use siq_buffer::{FillGuard, FramePool, ReleaseSink};
use siq_devices::{
    BoxFuture, Camera, CameraCapabilities, CameraSettings, CameraStats, Device, DeviceError,
    DeviceInfo, DeviceResult, FrameRateModel, FrameSink, RoiRules, TriggerMode,
};
use siq_types::{
    Binning, Clock, DeviceMicros, FrameMeta, Micrometers, Millis, PixelFormat, Timestamp,
};
use tracing::warn;

use crate::grabber::{ErrorKind, Grabber, GrabberError, Module, Popped};
use crate::map::{FeatureMap, module_of};

/// How long a pop waits before the camera's thread looks for a stop and for released slots.
const TICK: Duration = Duration::from_millis(20);
/// The frames whose arrival times the clocks' offset: the grabber's clock drifts from the host's
/// by tens of ppm, so it is taken again over the last of them.
const OFFSETS: usize = 64;

/// A camera on a frame grabber, named as its [`FeatureMap`] says.
pub struct EuresysCamera<G: Grabber> {
    info: DeviceInfo,
    capabilities: CameraCapabilities,
    shared: Arc<Shared<G>>,
}

struct Shared<G> {
    map: FeatureMap,
    clock: Arc<dyn Clock>,
    control: Mutex<Control>,
    grabber: Mutex<G>,
    // Dropped after the grabber, which may hold the slots announced until it is closed.
    announced: Mutex<Option<FramePool>>,
    stopping: AtomicBool,
    /// Callers waiting for the grabber: the camera's thread steps aside for them.
    waiting: AtomicU64,
    delivered: AtomicU64,
    lost_no_slot: AtomicU64,
    lost_device: AtomicU64,
}

struct Control {
    settings: CameraSettings,
    thread: Option<JoinHandle<()>>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn remote<G: Grabber>(grabber: &mut G, feature: &str) -> DeviceResult<String> {
    Ok(grabber.get(Module::Remote, feature)?)
}

fn number<G: Grabber>(grabber: &mut G, feature: &str) -> DeviceResult<f64> {
    let text = remote(grabber, feature)?;
    let number = text.trim().parse().ok().filter(|n: &f64| n.is_finite());
    number.ok_or_else(|| DeviceError::Fault(format!("{feature} is `{text}`, not a number")))
}

fn whole<G: Grabber>(grabber: &mut G, feature: &str) -> DeviceResult<u32> {
    let value = number(grabber, feature)?;
    let whole = (value >= 0.0 && value <= f64::from(u32::MAX)).then_some(value as u32);
    whole.ok_or_else(|| DeviceError::Fault(format!("{feature} is {value}, not a count")))
}

fn set<G: Grabber>(grabber: &mut G, feature: &str, value: impl ToString) -> DeviceResult<()> {
    Ok(grabber.set(Module::Remote, feature, &value.to_string())?)
}

/// What the camera can do, read from it: GenICam's `Feature.Min`, `.Max` and `.Inc`, with what
/// the map adds.
fn capabilities<G: Grabber>(g: &mut G, map: &FeatureMap) -> DeviceResult<CameraCapabilities> {
    let sensor_width = whole(g, "SensorWidth").or_else(|_| whole(g, "WidthMax"))?;
    let sensor_height = whole(g, "SensorHeight").or_else(|_| whole(g, "HeightMax"))?;
    let offered = g.entries(Module::Remote, "PixelFormat")?;
    let mut pixel_formats = Vec::new();
    for format in offered
        .iter()
        .filter_map(|entry| map.pixel_formats.get(entry))
    {
        if !pixel_formats.contains(format) {
            pixel_formats.push(*format);
        }
    }
    if pixel_formats.is_empty() {
        let offered = offered.join(", ");
        let why = format!("none of the camera's pixel formats ({offered}) is in the feature map");
        return Err(DeviceError::Unsupported(why));
    }
    // A camera without binning has no feature for it.
    let most = |g: &mut G, f: &str| whole(g, &format!("{f}.Max")).map_or(1, |n| n.clamp(1, 255));
    let (across, down) = (most(g, "BinningHorizontal"), most(g, "BinningVertical"));
    let binnings = (1..=down as u8)
        .flat_map(|v| (1..=across as u8).filter_map(move |h| Binning::new(h, v)))
        .collect();
    let step = |g: &mut G, f: &str| whole(g, &format!("{f}.Inc")).map_or(1, |n| n.max(1));
    let roi = RoiRules {
        step_x: step(g, "Width").max(step(g, "OffsetX")),
        step_y: step(g, "Height").max(step(g, "OffsetY")),
        min_width: whole(g, "Width.Min").unwrap_or(1).max(1),
        min_height: whole(g, "Height.Min").unwrap_or(1).max(1),
    };
    let exposure = |g: &mut G, limit: &str| {
        number(g, &format!("{}.{limit}", map.exposure)).map(|us| Millis(us / 1e3))
    };
    Ok(CameraCapabilities {
        sensor_width,
        sensor_height,
        pixel_size: Micrometers(map.pixel_size_um),
        pixel_formats,
        cfa: map.cfa,
        binnings,
        roi,
        exposure_min: exposure(g, "Min")?,
        exposure_max: exposure(g, "Max")?,
        frame_rate: map.readout(),
    })
}

/// Sets the exposure and the gain, and, free-running, the frame period: the readout, or the
/// exposure and a margin if that is longer. The Adimec skips every other frame unless its period
/// is more than 2 ms longer than its exposure (P10.2): the readout's fixed part and 2 ms clear it. These change while acquiring. A camera may shorten an
/// exposure to fit its period, so the period is set before the exposure, and again after it.
fn timing<G: Grabber>(
    g: &mut G,
    map: &FeatureMap,
    settings: &CameraSettings,
    model: &FrameRateModel,
) -> DeviceResult<()> {
    let longest = model
        .readout(settings)
        .0
        .max(settings.exposure.0 + model.overhead.0 + 2.0);
    let free = map
        .frame_period
        .as_ref()
        .filter(|_| settings.trigger == TriggerMode::FreeRun);
    let period = |g: &mut G| free.map_or(Ok(()), |feature| set(g, feature, longest * 1e3));
    period(g)?;
    set(g, &map.exposure, settings.exposure.0 * 1e3)?;
    period(g)?;
    let gain = settings.gain;
    set(
        g,
        &map.gain,
        if map.gain_db {
            20.0 * gain.log10()
        } else {
            gain
        },
    )
}

/// Applies every setting: the format, the binning, then the region, its offsets cleared first
/// so that any size fits, then the timing and the trigger.
fn apply<G: Grabber>(
    g: &mut G,
    map: &FeatureMap,
    capabilities: &CameraCapabilities,
    settings: &CameraSettings,
) -> DeviceResult<()> {
    let format = settings.pixel_format;
    let entry = map.entry(format).ok_or_else(|| {
        DeviceError::InvalidArgument(format!("the feature map has no entry for {format:?}"))
    })?;
    set(g, "PixelFormat", entry)?;
    let (across, down) = settings.binning.factors();
    if capabilities.binnings.len() > 1 {
        set(g, "BinningHorizontal", across)?;
        set(g, "BinningVertical", down)?;
    }
    let (across, down) = (u32::from(across), u32::from(down));
    let roi = settings.roi;
    set(g, "OffsetX", 0)?;
    set(g, "OffsetY", 0)?;
    set(g, "Width", roi.width / across)?;
    set(g, "Height", roi.height / down)?;
    set(g, "OffsetX", roi.offset_x / across)?;
    set(g, "OffsetY", roi.offset_y / down)?;
    for [feature, value] in map.trigger(settings.trigger) {
        let (module, name) = module_of(feature);
        g.set(module, name, value)?;
    }
    timing(g, map, settings, &capabilities.frame_rate)
}

impl<G: Grabber> EuresysCamera<G> {
    /// The camera `grabber` has open, named as `map` says, timed by `clock`: set up for its full
    /// frame, in its deepest pixel format, at 10 ms and unity gain, free-running.
    pub fn new(mut grabber: G, map: FeatureMap, clock: Arc<dyn Clock>) -> DeviceResult<Self> {
        let model = remote(&mut grabber, "DeviceModelName")?;
        if let Some(expected) = map.model.as_ref().filter(|expected| **expected != model) {
            return Err(DeviceError::InvalidArgument(format!(
                "the feature map is for a {expected}, and the camera is a {model}"
            )));
        }
        if map.pixel_size_um.is_nan() || map.pixel_size_um <= 0.0 {
            let why = "the feature map must give the sensor's pixel_size_um";
            return Err(DeviceError::InvalidArgument(why.into()));
        }
        let info = DeviceInfo {
            vendor: remote(&mut grabber, "DeviceVendorName")?,
            model,
            serial: remote(&mut grabber, "DeviceSerialNumber").ok(),
            firmware: remote(&mut grabber, "DeviceFirmwareVersion").ok(),
        };
        let capabilities = capabilities(&mut grabber, &map)?;
        // As the simulated camera starts: the full frame at 10 ms, whatever the camera was left in,
        // in the deepest format, since datasets take 16-bit frames.
        let deepest = capabilities
            .pixel_formats
            .iter()
            .max_by_key(|f| f.bits())
            .copied();
        let settings = CameraSettings {
            pixel_format: deepest.unwrap_or(PixelFormat::Mono16),
            ..CameraSettings::full_frame(&capabilities, Millis(10.0))
        };
        apply(&mut grabber, &map, &capabilities, &settings)?;
        let shared = Shared {
            map,
            clock,
            control: Mutex::new(Control {
                settings,
                thread: None,
            }),
            grabber: Mutex::new(grabber),
            announced: Mutex::new(None),
            stopping: AtomicBool::new(false),
            waiting: AtomicU64::new(0),
            delivered: AtomicU64::new(0),
            lost_no_slot: AtomicU64::new(0),
            lost_device: AtomicU64::new(0),
        };
        Ok(Self {
            info,
            capabilities,
            shared: Arc::new(shared),
        })
    }

    /// Announces every slot of `pool`, and starts the camera's thread.
    fn begin(&self, pool: FramePool, sink: Arc<dyn FrameSink>) -> DeviceResult<()> {
        let mut control = lock(&self.shared.control);
        let needed = control.settings.frame_bytes();
        if control.thread.is_some() {
            return Err(DeviceError::Busy("the camera is acquiring already".into()));
        }
        if pool.slot_bytes() < needed {
            let size = pool.slot_bytes();
            let why = format!("{size}-byte slots can't hold {needed}-byte frames");
            return Err(DeviceError::InvalidArgument(why));
        }
        let mut slots = pool.take_all_free();
        if slots.len() < pool.slots() {
            let (held, all) = (pool.slots() - slots.len(), pool.slots());
            let why = format!("{held} of the pool's {all} slots are still held");
            return Err(DeviceError::Busy(why));
        }
        // Each slot is announced at its own index.
        slots.sort_by_key(FillGuard::slot);
        let buffers: Vec<*mut u8> = slots.iter_mut().map(FillGuard::as_mut_ptr).collect();
        let mut grabber = lock(&self.shared.grabber);
        // SAFETY: each slot is `slot_bytes` long. `announced` keeps the pool's memory alive until
        // the grabber is dropped or announces again: it is dropped after the grabber, and replaced
        // only once the grabber has announced these. A slot is queued only while its guard is the
        // camera's thread's: no lease reads it, and nothing else fills it.
        unsafe { grabber.announce(&buffers, pool.slot_bytes())? };
        *lock(&self.shared.announced) = Some(pool.clone());
        let (released, returned) = mpsc::channel();
        pool.set_release_sink(Arc::new(Requeue(released)));
        if let Err(error) = grabber.start() {
            pool.reset_release_sink();
            return Err(error.into());
        }
        drop(grabber);
        self.shared.stopping.store(false, Ordering::Release);
        let shared = Arc::clone(&self.shared);
        let thread = thread::Builder::new()
            .name("euresys-camera".into())
            .spawn(move || {
                time_critical();
                let mut running = Running {
                    shared: &shared,
                    pool,
                    returned,
                    queued: slots.into_iter().map(Some).collect(),
                    seq: 0,
                    offsets: VecDeque::new(),
                };
                running.run(sink.as_ref());
            });
        let thread = thread.map_err(|error| DeviceError::Fault(error.to_string()))?;
        control.thread = Some(thread);
        Ok(())
    }

    fn trigger_now(&self) -> DeviceResult<()> {
        let control = lock(&self.shared.control);
        let why = match (control.settings.trigger, &control.thread) {
            (TriggerMode::Software, Some(_)) => None,
            (TriggerMode::Software, None) => Some("the camera isn't acquiring"),
            _ => Some("the camera isn't in software trigger"),
        };
        if let Some(why) = why {
            return Err(DeviceError::InvalidArgument(why.into()));
        }
        drop(control);
        let map = &self.shared.map;
        let mut grabber = self.shared.grab();
        Ok(grabber.execute(map.trigger_module, &map.trigger_command)?)
    }
}

impl<G> Shared<G> {
    /// The grabber, for a caller beside the camera's thread. The thread takes it again and again,
    /// and a lock isn't fair, so it steps aside while a caller waits: within a tick.
    fn grab(&self) -> MutexGuard<'_, G> {
        self.waiting.fetch_add(1, Ordering::AcqRel);
        let grabber = lock(&self.grabber);
        self.waiting.fetch_sub(1, Ordering::AcqRel);
        grabber
    }
}

/// Raises the calling thread to time-critical priority (architecture §4.3): a frame popped late
/// holds its slot from the grabber. The thread waits in `pop` or sleeps, so it starves no one.
fn time_critical() {
    use windows_sys::Win32::System::Threading::{
        GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_TIME_CRITICAL,
    };
    // SAFETY: the pseudo-handle of the calling thread, valid in it, which the call only reads.
    let raised = unsafe { SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_TIME_CRITICAL) };
    if raised == 0 {
        warn!("the camera's thread couldn't be raised to time-critical priority");
    }
}

/// Where a slot goes when its last lease drops: back to the camera's thread, to be filled again.
/// Once the thread has ended, it goes back to the pool's free list.
struct Requeue(Sender<FillGuard>);

impl ReleaseSink for Requeue {
    fn release(&self, slot: FillGuard) {
        drop(self.0.send(slot));
    }
}

/// The camera's thread, while acquiring.
struct Running<'a, G: Grabber> {
    shared: &'a Shared<G>,
    pool: FramePool,
    returned: Receiver<FillGuard>,
    /// The slots queued to the grabber, by index.
    queued: Vec<Option<FillGuard>>,
    seq: u64,
    /// The host's clock less the grabber's, as the last frames arrived.
    offsets: VecDeque<i64>,
}

impl<G: Grabber> Running<'_, G> {
    fn run(&mut self, sink: &dyn FrameSink) {
        while !self.shared.stopping.load(Ordering::Acquire) {
            if let Err(error) = self.step(sink) {
                warn!(%error, "the Euresys camera stopped acquiring");
                break;
            }
        }
    }

    /// Requeues the slots given back, then waits a tick for a frame and hands it on.
    fn step(&mut self, sink: &dyn FrameSink) -> Result<(), GrabberError> {
        while self.shared.waiting.load(Ordering::Acquire) > 0 {
            thread::sleep(Duration::from_millis(1));
        }
        let popped = {
            let mut grabber = lock(&self.shared.grabber);
            // Slots back from their leases, or dropped unpublished, are filled again.
            let pool = &self.pool;
            let free = iter::from_fn(|| pool.try_acquire_free());
            for guard in self.returned.try_iter().chain(free) {
                let index = guard.slot();
                grabber.requeue(index)?;
                self.queued[index] = Some(guard);
            }
            let popped = grabber.pop(TICK)?;
            let counters = grabber.counters()?;
            self.shared
                .lost_no_slot
                .store(counters.underrun, Ordering::Relaxed);
            popped
        };
        let Some(popped) = popped else {
            return Ok(());
        };
        let Some(guard) = self.queued.get_mut(popped.index).and_then(Option::take) else {
            let why = format!("the grabber filled buffer {}, not queued", popped.index);
            return Err(GrabberError::new(ErrorKind::Fault, why));
        };
        let settings = lock(&self.shared.control).settings;
        // A frame of another size than the settings say isn't the frame they describe.
        let short = popped.bytes != settings.frame_bytes();
        if popped.incomplete || short {
            if short {
                let (bytes, needed) = (popped.bytes, settings.frame_bytes());
                warn!(bytes, needed, "a frame of the wrong size: lost");
            }
            self.shared.lost_device.fetch_add(1, Ordering::Relaxed);
            lock(&self.shared.grabber).requeue(popped.index)?;
            self.queued[popped.index] = Some(guard);
            return Ok(());
        }
        let meta = self.meta(&settings, &popped);
        self.seq += 1;
        self.shared.delivered.fetch_add(1, Ordering::Relaxed);
        sink.frame(guard, meta);
        Ok(())
    }

    /// A frame's metadata. The grabber stamps a frame as its readout starts, as its exposure ends,
    /// and it arrives its readout later (P10.2): the stamp, on the host's clock, less half the
    /// exposure, is the middle of the exposure. The clocks' offset is the least of the last frames'
    /// arrival, less their readout, less their stamp: the quickest arrival is the least delayed.
    fn meta(&mut self, settings: &CameraSettings, popped: &Popped) -> FrameMeta {
        let ns = |millis: Millis| (millis.0 * 1e6) as i64;
        let stamp = popped.timestamp_ns as i64;
        let readout = ns(self.shared.map.readout().readout(settings));
        let arrived = self.shared.clock.now().0 as i64;
        self.offsets.push_back(arrived - readout - stamp);
        if self.offsets.len() > OFFSETS {
            self.offsets.pop_front();
        }
        let offset = self.offsets.iter().copied().min().unwrap_or_default();
        let mid = stamp + offset - ns(settings.exposure) / 2;
        let mut meta = settings.frame_meta(self.seq, Timestamp(mid.max(0) as u64));
        meta.device_ts = Some(DeviceMicros(popped.timestamp_ns / 1000));
        let (x, y) = (settings.roi.offset_x, settings.roi.offset_y);
        meta.cfa = self.shared.map.cfa.map(|cfa| cfa.shifted(x, y));
        meta
    }
}

impl<G: Grabber> Drop for Running<'_, G> {
    /// Stops the grabber and lets go of the slots before they go back to the pool, also when the
    /// thread panics: none is written once a lease could read it.
    fn drop(&mut self) {
        let mut grabber = lock(&self.shared.grabber);
        if let Err(error) = grabber.stop() {
            warn!(%error, "stopping the grabber");
        }
        // SAFETY: announcing nothing gives the grabber no buffer: it revokes the slots.
        if let Err(error) = unsafe { grabber.announce(&[], 0) } {
            warn!(%error, "revoking the grabber's buffers");
        }
        drop(grabber);
        self.pool.reset_release_sink();
        self.returned.try_iter().for_each(drop);
        self.queued.clear();
    }
}

impl<G: Grabber> Device for EuresysCamera<G> {
    fn info(&self) -> &DeviceInfo {
        &self.info
    }
}

impl<G: Grabber> Camera for EuresysCamera<G> {
    fn capabilities(&self) -> &CameraCapabilities {
        &self.capabilities
    }

    fn settings(&self) -> CameraSettings {
        lock(&self.shared.control).settings
    }

    fn configure(&self, settings: CameraSettings) -> BoxFuture<'_, DeviceResult<()>> {
        let result = self.capabilities.check(&settings).and_then(|()| {
            let mut control = lock(&self.shared.control);
            let mut grabber = self.shared.grab();
            let (map, capabilities) = (&self.shared.map, &self.capabilities);
            if control.thread.is_some() {
                let live = CameraSettings {
                    exposure: settings.exposure,
                    gain: settings.gain,
                    ..control.settings
                };
                if settings != live {
                    let why = "only the exposure and gain change while acquiring";
                    return Err(DeviceError::Busy(why.into()));
                }
                timing(&mut *grabber, map, &settings, &capabilities.frame_rate)?;
            } else {
                apply(&mut *grabber, map, capabilities, &settings)?;
            }
            control.settings = settings;
            Ok(())
        });
        Box::pin(ready(result))
    }

    fn start(
        &self,
        pool: FramePool,
        frames: Arc<dyn FrameSink>,
    ) -> BoxFuture<'_, DeviceResult<()>> {
        Box::pin(ready(self.begin(pool, frames)))
    }

    fn stop(&self) -> BoxFuture<'_, DeviceResult<()>> {
        self.shared.stopping.store(true, Ordering::Release);
        Box::pin(async move {
            // The thread notices within a tick.
            let finished = || {
                let control = lock(&self.shared.control);
                control.thread.as_ref().is_none_or(JoinHandle::is_finished)
            };
            while !finished() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            let Some(thread) = lock(&self.shared.control).thread.take() else {
                return Ok(());
            };
            let panicked = || DeviceError::Fault("the camera's thread panicked".into());
            thread.join().map_err(|_| panicked())
        })
    }

    fn trigger(&self) -> BoxFuture<'_, DeviceResult<()>> {
        Box::pin(ready(self.trigger_now()))
    }

    fn stats(&self) -> CameraStats {
        let count = |counter: &AtomicU64| counter.load(Ordering::Relaxed);
        CameraStats {
            delivered: count(&self.shared.delivered),
            lost_no_slot: count(&self.shared.lost_no_slot),
            lost_device: count(&self.shared.lost_device),
        }
    }
}

impl<G: Grabber> Drop for EuresysCamera<G> {
    fn drop(&mut self) {
        self.shared.stopping.store(true, Ordering::Release);
        if let Some(thread) = lock(&self.shared.control).thread.take() {
            drop(thread.join());
        }
    }
}
