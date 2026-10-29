//! The simulated camera (REQ-SIM-001, ADR-0026): a [`Camera`] that renders frames of a world into
//! pool slots on its own thread, on a camera's schedule.
//!
//! - **Free-run:** an exposure starts every period, `max(exposure, readout)`. A frame is ready
//!   its exposure plus its readout after it starts. If a render runs late, the schedule slips:
//!   frame numbers stay contiguous, and the rate drops.
//! - **Software and external trigger:** an exposure starts `trigger_latency` after `trigger()`,
//!   and not before the last one allows. The simulator has no trigger line, so `trigger()` also
//!   stands in for an external trigger.
//! - **No free slot when a frame is ready:** the frame is lost and counted, and its number
//!   skipped.

use std::future::{Future, ready};
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::task::{Context, Poll, Waker};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use siq_buffer::FramePool;
use siq_config::model::{ObjectiveConfig, SimCameraConfig, SimConfig, SimProfile};
use siq_devices::{
    BoxFuture, Camera, CameraCapabilities, CameraSettings, CameraStats, Device, DeviceError,
    DeviceInfo, DeviceResult, FrameRateModel, FrameSink, RoiRules, TriggerMode,
};
use siq_synth::World;
use siq_types::{
    Affine2, AffineParams, Binning, Clock, DeviceMicros, FrameMeta, Hertz, Micrometers, Millis,
    PixelFormat, Timestamp,
};

use crate::optics::{OpticsModel, Renderer, SensorModel, Shot};
use crate::state::TrueState;

/// One objective of the simulated microscope.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Lens {
    /// Its turret position.
    pub position: u8,
    /// The magnification.
    pub magnification: f64,
    /// The numerical aperture.
    pub numerical_aperture: f64,
    /// The immersion medium's refractive index.
    pub immersion_index: f64,
}

/// What the simulated camera is.
#[derive(Debug, Clone, PartialEq)]
pub struct SimCameraSetup {
    /// The sensor's columns and rows.
    pub sensor: (u32, u32),
    /// The pixel format it starts in.
    pub pixel_format: PixelFormat,
    /// The full-frame rate.
    pub frame_rate: Hertz,
    /// The sensor's pixel pitch.
    pub pixel_size: Micrometers,
    /// The objectives. The one at the true state's turret position is in place, or the first.
    pub lenses: Vec<Lens>,
    /// The optics, whose aperture and index the lens in place sets.
    pub optics: OpticsModel,
    /// The sensor's noise and fixed pattern.
    pub sensor_model: SensorModel,
    /// From `trigger()` to the start of the exposure.
    pub trigger_latency: Millis,
    /// How the camera sits: the image's turn, scale and shear against the stage, a linear
    /// transform of the sensor's pixels. The identity for a perfect mount.
    pub mount: Affine2,
}

impl SimCameraSetup {
    /// The setup a configuration describes: its `[camera]` of kind `sim`, its `[[objectives]]`,
    /// and `[sim]`'s seed and profile. The `fast` profile halves each side of the sensor and
    /// doubles its pixel, so the field of view stays the same.
    pub fn from_config(
        camera: &SimCameraConfig,
        objectives: &[ObjectiveConfig],
        sim: &SimConfig,
    ) -> Self {
        let scale = match sim.profile {
            SimProfile::Fast => 2,
            SimProfile::Full => 1,
        };
        let lens = |(index, objective): (usize, &ObjectiveConfig)| Lens {
            position: objective.turret_position.unwrap_or(index as u8),
            magnification: objective.magnification,
            numerical_aperture: objective.numerical_aperture,
            immersion_index: objective.immersion.refractive_index(),
        };
        let mut lenses: Vec<Lens> = objectives.iter().enumerate().map(lens).collect();
        if lenses.is_empty() {
            lenses.push(Lens {
                position: 0,
                magnification: 10.0,
                numerical_aperture: 0.30,
                immersion_index: 1.0,
            });
        }
        Self {
            sensor: (camera.width / scale, camera.height / scale),
            pixel_format: camera.pixel_format,
            frame_rate: camera.frame_rate,
            pixel_size: camera.pixel_size * f64::from(scale),
            lenses,
            optics: OpticsModel::default(),
            sensor_model: SensorModel {
                seed: sim.seed,
                ..SensorModel::default()
            },
            trigger_latency: Millis(0.1),
            mount: Affine2::from_params(&AffineParams {
                scale_x: 1.0 + sim.mount.scale_error,
                scale_y: 1.0 + sim.mount.scale_error,
                rotation: sim.mount.rotation.to_radians(),
                shear: sim.mount.shear,
                translation: [0.0; 2],
            }),
        }
    }

    /// The lens at turret `position`, or the first.
    fn lens(&self, position: Option<u8>) -> Lens {
        let placed = self
            .lenses
            .iter()
            .find(|lens| Some(lens.position) == position);
        *placed.or(self.lenses.first()).expect("a setup has a lens")
    }

    /// What the camera can do: every Mono format, 1, 2 and 4 binning, ROI steps of up to 8 that
    /// divide the sensor, so its full frame is always one, and a readout of 1 ms plus a line
    /// time that gives the full-frame rate.
    pub fn capabilities(&self) -> CameraCapabilities {
        let (width, height) = self.sensor;
        let step = |size: u32| {
            [8, 4, 2, 1]
                .into_iter()
                .find(|&step| size.is_multiple_of(step))
                .unwrap_or(1)
        };
        let full = 1e3 / self.frame_rate.0.max(1e-3);
        let mut formats = vec![self.pixel_format];
        formats.extend(
            [
                PixelFormat::Mono8,
                PixelFormat::Mono10,
                PixelFormat::Mono12,
                PixelFormat::Mono14,
                PixelFormat::Mono16,
            ]
            .into_iter()
            .filter(|&f| f != self.pixel_format),
        );
        CameraCapabilities {
            sensor_width: width,
            sensor_height: height,
            pixel_size: self.pixel_size,
            pixel_formats: formats,
            cfa: None,
            binnings: [1, 2, 4]
                .map(|b| Binning::new(b, b).expect("not 0"))
                .to_vec(),
            roi: RoiRules {
                step_x: step(width),
                step_y: step(height),
                min_width: 64.min(width),
                min_height: 64.min(height),
            },
            exposure_min: Millis(0.01),
            exposure_max: Millis(10_000.0),
            frame_rate: FrameRateModel {
                overhead: Millis(1.0),
                line_time: Millis((full - 1.0).max(0.0) / f64::from(height.max(1))),
            },
        }
    }
}

/// The simulated camera.
pub struct SimCamera {
    info: DeviceInfo,
    capabilities: CameraCapabilities,
    shared: Arc<Shared>,
}

struct Shared {
    setup: SimCameraSetup,
    readout: FrameRateModel,
    world: Arc<dyn World>,
    state: Arc<TrueState>,
    clock: Arc<dyn Clock>,
    control: Mutex<Control>,
    wake: Condvar,
    delivered: AtomicU64,
    lost: AtomicU64,
}

struct Control {
    settings: CameraSettings,
    thread: Option<JoinHandle<()>>,
    stopping: bool,
    finished: bool,
    trigger: Option<Timestamp>,
    waker: Option<Waker>,
}

/// How long the camera's thread waits at a time, so that it notices a stop.
const STEP: Duration = Duration::from_millis(20);

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn millis(duration: Millis) -> Duration {
    Duration::from_secs_f64(duration.0.max(0.0) / 1e3)
}

impl SimCamera {
    /// A camera of `world`, seeing it as `state` says, timed by `clock`: set up for its full
    /// frame at 10 ms, free-running.
    pub fn new(
        setup: SimCameraSetup,
        world: Arc<dyn World>,
        state: Arc<TrueState>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        let capabilities = setup.capabilities();
        let settings = CameraSettings::full_frame(&capabilities, Millis(10.0));
        let info = DeviceInfo {
            serial: Some(format!("sim-{}", setup.sensor_model.seed)),
            ..DeviceInfo::new("SpectrumIQ", "simulated camera")
        };
        let control = Control {
            settings,
            thread: None,
            stopping: false,
            finished: false,
            trigger: None,
            waker: None,
        };
        let shared = Shared {
            readout: capabilities.frame_rate,
            setup,
            world,
            state,
            clock,
            control: Mutex::new(control),
            wake: Condvar::new(),
            delivered: AtomicU64::new(0),
            lost: AtomicU64::new(0),
        };
        Self {
            info,
            capabilities,
            shared: Arc::new(shared),
        }
    }
}

impl Device for SimCamera {
    fn info(&self) -> &DeviceInfo {
        &self.info
    }
}

impl Camera for SimCamera {
    fn capabilities(&self) -> &CameraCapabilities {
        &self.capabilities
    }

    fn settings(&self) -> CameraSettings {
        lock(&self.shared.control).settings
    }

    fn configure(&self, settings: CameraSettings) -> BoxFuture<'_, DeviceResult<()>> {
        let result = self.capabilities.check(&settings).and_then(|()| {
            let mut control = lock(&self.shared.control);
            let live = CameraSettings {
                exposure: settings.exposure,
                gain: settings.gain,
                ..control.settings
            };
            if control.thread.is_some() && settings != live {
                return Err(DeviceError::Busy(
                    "only the exposure and gain change while acquiring".into(),
                ));
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
        let mut control = lock(&self.shared.control);
        let result = match control.thread {
            Some(_) => Err(DeviceError::Busy("the camera is acquiring already".into())),
            None if pool.slot_bytes() < control.settings.frame_bytes() => {
                Err(DeviceError::InvalidArgument(format!(
                    "{}-byte slots can't hold {}-byte frames",
                    pool.slot_bytes(),
                    control.settings.frame_bytes()
                )))
            }
            None => {
                (control.stopping, control.finished, control.trigger) = (false, false, None);
                let (shared, epoch) = (Arc::clone(&self.shared), self.shared.clock.now());
                let thread = thread::Builder::new()
                    .name("sim-camera".into())
                    .spawn(move || run(&shared, epoch, &pool, frames.as_ref()));
                thread
                    .map(|thread| control.thread = Some(thread))
                    .map_err(|error| DeviceError::Fault(error.to_string()))
            }
        };
        Box::pin(ready(result))
    }

    fn stop(&self) -> BoxFuture<'_, DeviceResult<()>> {
        lock(&self.shared.control).stopping = true;
        self.shared.wake.notify_all();
        Box::pin(Stopped {
            shared: &self.shared,
        })
    }

    fn trigger(&self) -> BoxFuture<'_, DeviceResult<()>> {
        let mut control = lock(&self.shared.control);
        let result = match (control.settings.trigger, &control.thread, control.trigger) {
            (TriggerMode::FreeRun, _, _) => Err(DeviceError::InvalidArgument(
                "the camera is free-running".into(),
            )),
            (_, None, _) => Err(DeviceError::InvalidArgument(
                "the camera isn't acquiring".into(),
            )),
            (_, _, Some(_)) => Err(DeviceError::Busy(
                "the last trigger hasn't started its exposure".into(),
            )),
            _ => {
                control.trigger = Some(self.shared.clock.now());
                self.shared.wake.notify_all();
                Ok(())
            }
        };
        Box::pin(ready(result))
    }

    fn stats(&self) -> CameraStats {
        CameraStats {
            delivered: self.shared.delivered.load(Ordering::Relaxed),
            lost_no_slot: self.shared.lost.load(Ordering::Relaxed),
            lost_device: 0,
        }
    }
}

/// Resolves when the camera's thread has finished.
struct Stopped<'a> {
    shared: &'a Shared,
}

impl Future for Stopped<'_> {
    type Output = DeviceResult<()>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut control = lock(&self.shared.control);
        match control.thread.take() {
            Some(thread) if control.finished => {
                drop(control);
                let joined = thread
                    .join()
                    .map_err(|_| DeviceError::Fault("the camera's thread panicked".into()));
                Poll::Ready(joined)
            }
            Some(thread) => {
                control.thread = Some(thread);
                control.waker = Some(cx.waker().clone());
                Poll::Pending
            }
            None => Poll::Ready(Ok(())),
        }
    }
}

/// The camera's thread: exposes, waits for each frame's readout, renders, and hands it over.
/// `epoch` is when `start` was called: the first exposure starts then.
fn run(shared: &Shared, epoch: Timestamp, pool: &FramePool, sink: &dyn FrameSink) {
    let setup = &shared.setup;
    let mut renderer = Renderer::new(Arc::clone(&shared.world), setup.optics, setup.sensor_model);
    let (mut seq, mut next) = (0u64, epoch);
    while let Some((settings, start)) = next_exposure(shared, next) {
        let exposure = millis(settings.exposure);
        let rows = f64::from(settings.image_size().1);
        let readout = millis(shared.readout.overhead + shared.readout.line_time * rows);
        let ready = start + exposure + readout;
        while !shared.clock.wait_until(ready, STEP) {
            if lock(&shared.control).stopping {
                return finish(shared);
            }
        }
        // Render into a free slot, if there is one.
        let frame = pool.try_acquire_free().and_then(|mut slot| {
            let shot = shot(shared, &mut renderer, settings);
            let (width, height) = shot.image_size();
            let mut image = slot.image_mut::<u16>(width, height).ok()?;
            renderer.render(&shot, seq, &mut image).ok()?;
            Some(slot)
        });
        // Whether the render ran late is judged before the frame is handed over, which may
        // take its consumers any time. Late, the next frame is ready now rather than overdue:
        // the schedule slips.
        let late = shared.clock.now() - (exposure + readout);
        let mid = start + exposure / 2;
        match frame {
            Some(slot) => {
                let device = DeviceMicros(mid.saturating_since(epoch).as_micros() as u64);
                let meta = FrameMeta {
                    device_ts: Some(device),
                    ..settings.frame_meta(seq, mid)
                };
                sink.frame(slot, meta);
                shared.delivered.fetch_add(1, Ordering::Relaxed);
            }
            None => {
                shared.lost.fetch_add(1, Ordering::Relaxed);
            }
        }
        seq += 1;
        next = (start + exposure.max(readout)).max(late);
    }
    finish(shared);
}

/// The settings of the next exposure, and when it starts: at `next` when free-running, or after
/// the next trigger. `None` once the camera is stopping.
fn next_exposure(shared: &Shared, next: Timestamp) -> Option<(CameraSettings, Timestamp)> {
    let mut control = lock(&shared.control);
    loop {
        if control.stopping {
            return None;
        }
        let settings = control.settings;
        match (settings.trigger, control.trigger.take()) {
            (TriggerMode::FreeRun, _) => return Some((settings, next)),
            (_, Some(at)) => {
                return Some((
                    settings,
                    (at + millis(shared.setup.trigger_latency)).max(next),
                ));
            }
            (_, None) => {
                control = shared
                    .wake
                    .wait_timeout(control, STEP)
                    .unwrap_or_else(PoisonError::into_inner)
                    .0
            }
        }
    }
}

/// The shot of the next frame: the true pose, the lens in place, and the light.
fn shot(shared: &Shared, renderer: &mut Renderer, settings: CameraSettings) -> Shot {
    let (setup, state) = (&shared.setup, &shared.state);
    let lens = setup.lens(state.objective());
    let optics = renderer.optics();
    if (optics.numerical_aperture, optics.immersion_index)
        != (lens.numerical_aperture, lens.immersion_index)
    {
        let optics = renderer.optics_mut();
        (optics.numerical_aperture, optics.immersion_index) =
            (lens.numerical_aperture, lens.immersion_index);
    }
    renderer.set_flux(setup.optics.flux * state.light());
    let scale = lens.magnification / setup.pixel_size.0;
    Shot {
        pose: state.pose(),
        calibration: setup.mount.compose(&Affine2::scale(scale, scale)),
        sensor: setup.sensor,
        settings,
    }
}

/// Marks the thread finished, and wakes whoever awaits the stop.
fn finish(shared: &Shared) {
    let waker = {
        let mut control = lock(&shared.control);
        control.finished = true;
        control.waker.take()
    };
    if let Some(waker) = waker {
        waker.wake();
    }
}
