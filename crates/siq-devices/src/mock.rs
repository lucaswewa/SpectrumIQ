//! Mocks that complete every operation at once, for engine tests.
//!
//! Each mock checks its arguments as a driver does, and fails its next operation with the error
//! given to `fail_next`. Motion mocks arrive at once, timestamp their status on the [`Clock`]
//! they are given, and have their home at zero. [`MockCamera`] fills each frame's bytes with the
//! frame's number, mod 256.

use std::future::ready;
use std::ops::Add;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use siq_buffer::FramePool;
use siq_types::{
    Binning, Clock, Micrometers, MicrometersPerSecond, Millis, ModalityMode, PixelFormat,
};

use crate::camera::RoiRules;
use crate::{
    AxisStatus, BoxFuture, Camera, CameraCapabilities, CameraSettings, CameraStats, Changer,
    ChangerStatus, Device, DeviceError, DeviceInfo, DeviceResult, FilterWheel, FocusDrive,
    FocusStatus, FrameRateModel, FrameSink, Illumination, IlluminationStatus, Modality,
    ModalityStatus, ModalityUnit, MotionState, MoveOpts, ObjectiveTurret, PatternSet, Stage,
    StageStatus, Travel, TriggerMode, Xyz, watch,
};

fn done<T: Send + 'static>(result: DeviceResult<T>) -> BoxFuture<'static, DeviceResult<T>> {
    Box::pin(ready(result))
}

fn invalid<T>(why: impl Into<String>) -> DeviceResult<T> {
    Err(DeviceError::InvalidArgument(why.into()))
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn info(model: &str) -> DeviceInfo {
    DeviceInfo::new("SpectrumIQ", model)
}

/// The error the next operation fails with.
#[derive(Default)]
struct Fault(Mutex<Option<DeviceError>>);

impl Fault {
    fn check(&self) -> DeviceResult<()> {
        lock(&self.0).take().map_or(Ok(()), Err)
    }
}

/// `Device`, and `fail_next`, for a mock whose `info` and `fault` are in `self`, or in `self.0`.
macro_rules! mock_device {
    ($mock:ident $(. $inner:tt)?) => {
        impl Device for $mock {
            fn info(&self) -> &DeviceInfo {
                &self$(.$inner)?.info
            }
        }

        impl $mock {
            /// Fails the next operation with `error`.
            pub fn fail_next(&self, error: DeviceError) {
                *lock(&self$(.$inner)?.fault.0) = Some(error);
            }
        }
    };
}

mock_device!(MockStage.0);
mock_device!(MockFocusDrive.0);
mock_device!(MockCamera);
mock_device!(MockIllumination);
mock_device!(MockChanger);
mock_device!(MockModalityUnit);

/// Axes that arrive at once: the stage's three, or the focus drive's one.
struct Axes<P> {
    info: DeviceInfo,
    travel: Travel<P>,
    within: fn(&Travel<P>, P) -> bool,
    clock: Arc<dyn Clock>,
    status: watch::Sender<AxisStatus<P>>,
    fault: Fault,
}

impl<P: Copy + Default + Add<Output = P>> Axes<P> {
    fn new(
        model: &str,
        travel: Travel<P>,
        within: fn(&Travel<P>, P) -> bool,
        clock: Arc<dyn Clock>,
    ) -> Self {
        let status = AxisStatus {
            position: P::default(),
            state: MotionState::Idle,
            homed: false,
            ts: clock.now(),
        };
        Self {
            info: info(model),
            travel,
            within,
            clock,
            status: watch::Sender::new(status),
            fault: Fault::default(),
        }
    }

    fn set(&self, position: P, state: MotionState, homed: bool) -> AxisStatus<P> {
        let status = AxisStatus {
            position,
            state,
            homed,
            ts: self.clock.now(),
        };
        self.status.send_replace(status);
        status
    }

    fn move_to(&self, target: P) -> DeviceResult<AxisStatus<P>> {
        self.fault.check()?;
        if !(self.within)(&self.travel, target) {
            return invalid("the target is outside the travel");
        }
        let homed = self.status.borrow().homed;
        Ok(self.set(target, MotionState::Idle, homed))
    }

    fn move_by(&self, delta: P) -> DeviceResult<AxisStatus<P>> {
        let position = self.status.borrow().position;
        self.move_to(position + delta)
    }

    fn change_state(&self, state: MotionState) -> DeviceResult<()> {
        self.fault.check()?;
        let now = *self.status.borrow();
        self.set(now.position, state, now.homed);
        Ok(())
    }

    fn home(&self) -> DeviceResult<AxisStatus<P>> {
        self.fault.check()?;
        Ok(self.set(P::default(), MotionState::Idle, true))
    }
}

/// A stage that arrives at once. A jog only changes its state, to `Jogging` until stopped.
pub struct MockStage(Axes<Xyz>);

impl MockStage {
    /// A stage at the origin, not homed, that may go within `travel`.
    pub fn new(travel: Travel<Xyz>, clock: Arc<dyn Clock>) -> Self {
        Self(Axes::new(
            "mock stage",
            travel,
            Travel::<Xyz>::contains,
            clock,
        ))
    }
}

impl Stage for MockStage {
    fn move_to(&self, target: Xyz, _: MoveOpts) -> BoxFuture<'_, DeviceResult<StageStatus>> {
        done(self.0.move_to(target))
    }

    fn move_by(&self, delta: Xyz, _: MoveOpts) -> BoxFuture<'_, DeviceResult<StageStatus>> {
        done(self.0.move_by(delta))
    }

    fn jog(&self, _: Xyz<MicrometersPerSecond>) -> BoxFuture<'_, DeviceResult<()>> {
        done(self.0.change_state(MotionState::Jogging))
    }

    fn stop(&self) -> BoxFuture<'_, DeviceResult<()>> {
        done(self.0.change_state(MotionState::Idle))
    }

    fn home(&self) -> BoxFuture<'_, DeviceResult<StageStatus>> {
        done(self.0.home())
    }

    fn status(&self) -> watch::Receiver<StageStatus> {
        self.0.status.subscribe()
    }

    fn limits(&self) -> Travel<Xyz> {
        self.0.travel
    }
}

/// A focus drive that arrives at once, as [`MockStage`] does.
pub struct MockFocusDrive(Axes<Micrometers>);

impl MockFocusDrive {
    /// A focus drive at zero, not homed, that may go within `travel`.
    pub fn new(travel: Travel, clock: Arc<dyn Clock>) -> Self {
        Self(Axes::new(
            "mock focus drive",
            travel,
            Travel::<Micrometers>::contains,
            clock,
        ))
    }
}

impl FocusDrive for MockFocusDrive {
    fn move_to(
        &self,
        target: Micrometers,
        _: MoveOpts,
    ) -> BoxFuture<'_, DeviceResult<FocusStatus>> {
        done(self.0.move_to(target))
    }

    fn move_by(&self, delta: Micrometers, _: MoveOpts) -> BoxFuture<'_, DeviceResult<FocusStatus>> {
        done(self.0.move_by(delta))
    }

    fn jog(&self, _: MicrometersPerSecond) -> BoxFuture<'_, DeviceResult<()>> {
        done(self.0.change_state(MotionState::Jogging))
    }

    fn stop(&self) -> BoxFuture<'_, DeviceResult<()>> {
        done(self.0.change_state(MotionState::Idle))
    }

    fn home(&self) -> BoxFuture<'_, DeviceResult<FocusStatus>> {
        done(self.0.home())
    }

    fn status(&self) -> watch::Receiver<FocusStatus> {
        self.0.status.subscribe()
    }

    fn limits(&self) -> Travel {
        self.0.travel
    }
}

/// A 64 × 48 Mono16 sensor with 2 × 2 binning, ROI steps of 4, and exposures from 0.01 ms to
/// 10 s, at about 100 frames/s full frame: small enough for any test.
pub fn small_sensor() -> CameraCapabilities {
    CameraCapabilities {
        sensor_width: 64,
        sensor_height: 48,
        pixel_size: Micrometers(3.76),
        pixel_formats: vec![PixelFormat::Mono16, PixelFormat::Mono8],
        cfa: None,
        binnings: vec![Binning::NONE, Binning::new(2, 2).expect("2 isn't 0")],
        roi: RoiRules {
            step_x: 4,
            step_y: 4,
            min_width: 8,
            min_height: 8,
        },
        exposure_min: Millis(0.01),
        exposure_max: Millis(10_000.0),
        frame_rate: FrameRateModel {
            overhead: Millis(0.4),
            line_time: Millis(0.2),
        },
    }
}

/// A camera that delivers a frame on each [`trigger`](Camera::trigger), in software mode, and on
/// each [`emit`](Self::emit), in any mode.
pub struct MockCamera {
    info: DeviceInfo,
    capabilities: CameraCapabilities,
    clock: Arc<dyn Clock>,
    fault: Fault,
    state: Mutex<CameraState>,
}

struct CameraState {
    settings: CameraSettings,
    running: Option<(FramePool, Arc<dyn FrameSink>)>,
    seq: u64,
    stats: CameraStats,
}

impl MockCamera {
    /// A camera that can do what `capabilities` say, set up for its full frame at 10 ms.
    pub fn new(capabilities: CameraCapabilities, clock: Arc<dyn Clock>) -> Self {
        let settings = CameraSettings::full_frame(&capabilities, Millis(10.0));
        let state = CameraState {
            settings,
            running: None,
            seq: 0,
            stats: CameraStats::default(),
        };
        Self {
            info: info("mock camera"),
            capabilities,
            clock,
            fault: Fault::default(),
            state: Mutex::new(state),
        }
    }

    /// Takes a frame, as a free-running camera does at its rate. It is lost if no slot is free.
    pub fn emit(&self) -> DeviceResult<()> {
        let (mut slot, sink, meta, bytes) = {
            let mut state = lock(&self.state);
            let Some((pool, sink)) = state.running.clone() else {
                return invalid("the camera isn't acquiring");
            };
            let (seq, settings) = (state.seq, state.settings);
            state.seq += 1;
            let Some(slot) = pool.try_acquire_free() else {
                state.stats.lost_no_slot += 1;
                return Ok(());
            };
            state.stats.delivered += 1;
            let meta = settings.frame_meta(seq, self.clock.now());
            (slot, sink, meta, settings.frame_bytes())
        };
        slot.bytes_mut()[..bytes].fill(meta.seq.0 as u8);
        sink.frame(slot, meta);
        Ok(())
    }

    /// Loses a frame, as a grabber does when its queue overflows: its number is skipped.
    pub fn lose_frame(&self) {
        let mut state = lock(&self.state);
        state.seq += 1;
        state.stats.lost_device += 1;
    }
}

impl Camera for MockCamera {
    fn capabilities(&self) -> &CameraCapabilities {
        &self.capabilities
    }

    fn settings(&self) -> CameraSettings {
        lock(&self.state).settings
    }

    fn configure(&self, settings: CameraSettings) -> BoxFuture<'_, DeviceResult<()>> {
        let result = self
            .fault
            .check()
            .and_then(|()| self.capabilities.check(&settings));
        done(result.and_then(|()| {
            let mut state = lock(&self.state);
            let (exposure, gain) = (settings.exposure, settings.gain);
            if state.running.is_some()
                && settings
                    != (CameraSettings {
                        exposure,
                        gain,
                        ..state.settings
                    })
            {
                return Err(DeviceError::Busy(
                    "only the exposure and gain change while acquiring".into(),
                ));
            }
            state.settings = settings;
            Ok(())
        }))
    }

    fn start(
        &self,
        pool: FramePool,
        frames: Arc<dyn FrameSink>,
    ) -> BoxFuture<'_, DeviceResult<()>> {
        done(self.fault.check().and_then(|()| {
            let mut state = lock(&self.state);
            let bytes = state.settings.frame_bytes();
            if state.running.is_some() {
                return Err(DeviceError::Busy("the camera is acquiring already".into()));
            }
            if pool.slot_bytes() < bytes {
                return invalid(format!(
                    "{}-byte slots can't hold {bytes}-byte frames",
                    pool.slot_bytes()
                ));
            }
            state.running = Some((pool, frames));
            state.seq = 0;
            Ok(())
        }))
    }

    fn stop(&self) -> BoxFuture<'_, DeviceResult<()>> {
        done(
            self.fault
                .check()
                .map(|()| lock(&self.state).running = None),
        )
    }

    fn trigger(&self) -> BoxFuture<'_, DeviceResult<()>> {
        done(self.fault.check().and_then(|()| {
            if lock(&self.state).settings.trigger != TriggerMode::Software {
                return invalid("triggering needs the software trigger mode");
            }
            self.emit()
        }))
    }

    fn stats(&self) -> CameraStats {
        lock(&self.state).stats
    }
}

/// Light sources whose level changes at once. They start off.
pub struct MockIllumination {
    info: DeviceInfo,
    sources: Vec<String>,
    status: watch::Sender<IlluminationStatus>,
    fault: Fault,
}

impl MockIllumination {
    /// Sources named `sources`, all off.
    pub fn new(sources: Vec<String>) -> Self {
        let status = IlluminationStatus {
            levels: vec![0.0; sources.len()],
        };
        Self {
            info: info("mock illumination"),
            sources,
            status: watch::Sender::new(status),
            fault: Fault::default(),
        }
    }
}

impl Illumination for MockIllumination {
    fn sources(&self) -> &[String] {
        &self.sources
    }

    fn set_level(&self, source: usize, level: f64) -> BoxFuture<'_, DeviceResult<()>> {
        done(self.fault.check().and_then(|()| {
            if source >= self.sources.len() {
                return invalid(format!("there is no source {source}"));
            }
            if !(0.0..=1.0).contains(&level) {
                return invalid(format!("the level {level} isn't within 0 to 1"));
            }
            self.status
                .send_modify(|status| status.levels[source] = level);
            Ok(())
        }))
    }

    fn status(&self) -> watch::Receiver<IlluminationStatus> {
        self.status.subscribe()
    }
}

/// A filter wheel or objective turret that changes position at once. No position is in the path
/// until the first selection.
pub struct MockChanger {
    info: DeviceInfo,
    positions: Vec<String>,
    status: watch::Sender<ChangerStatus>,
    fault: Fault,
}

impl MockChanger {
    /// A changer with `positions`, by name.
    pub fn new(positions: Vec<String>) -> Self {
        let status = watch::Sender::new(ChangerStatus::default());
        Self {
            info: info("mock changer"),
            positions,
            status,
            fault: Fault::default(),
        }
    }
}

impl Changer for MockChanger {
    fn positions(&self) -> &[String] {
        &self.positions
    }

    fn select(&self, position: u8) -> BoxFuture<'_, DeviceResult<()>> {
        done(self.fault.check().and_then(|()| {
            if usize::from(position) >= self.positions.len() {
                return invalid(format!("there is no position {position}"));
            }
            self.status.send_replace(ChangerStatus {
                position: Some(position),
                moving: false,
            });
            Ok(())
        }))
    }

    fn status(&self) -> watch::Receiver<ChangerStatus> {
        self.status.subscribe()
    }
}

impl FilterWheel for MockChanger {}

impl ObjectiveTurret for MockChanger {}

/// A modality unit that changes mode at once. It starts in its first mode.
pub struct MockModalityUnit {
    info: DeviceInfo,
    modes: Vec<ModalityMode>,
    patterns: Option<PatternSet>,
    status: watch::Sender<ModalityStatus>,
    fault: Fault,
}

impl MockModalityUnit {
    /// A unit offering `modes`, and `patterns` for structured illumination.
    pub fn new(modes: Vec<ModalityMode>, patterns: Option<PatternSet>) -> Self {
        let mode = modes.first().copied().unwrap_or(ModalityMode::Widefield);
        let modality = Modality {
            mode,
            pattern: None,
        };
        let status = watch::Sender::new(ModalityStatus {
            modality,
            moving: false,
        });
        Self {
            info: info("mock modality unit"),
            modes,
            patterns,
            status,
            fault: Fault::default(),
        }
    }
}

impl ModalityUnit for MockModalityUnit {
    fn modes(&self) -> &[ModalityMode] {
        &self.modes
    }

    fn patterns(&self) -> Option<PatternSet> {
        self.patterns
    }

    fn set_modality(&self, modality: Modality) -> BoxFuture<'_, DeviceResult<()>> {
        done(self.fault.check().and_then(|()| {
            modality.check(&self.modes, self.patterns)?;
            self.status.send_replace(ModalityStatus {
                modality,
                moving: false,
            });
            Ok(())
        }))
    }

    fn status(&self) -> watch::Receiver<ModalityStatus> {
        self.status.subscribe()
    }
}
