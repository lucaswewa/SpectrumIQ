//! The peripheral simulators (REQ-SIM-003, REQ-DEV-001): illumination, the filter wheel, the
//! objective turret and the modality unit, behind their `siq-devices` traits.
//!
//! Each change takes a realistic time on the clock, then sets the true state that the camera
//! sees:
//!
//! | Simulator | Time |
//! |---|---|
//! | [`SimIllumination`] | 5 ms for the light to settle |
//! | [`SimChanger::filter_wheel`] | 60 ms per position, the shorter way round |
//! | [`SimChanger::objective_turret`] | 400 ms per position, the shorter way round |
//! | [`SimModalityUnit`] | 300 ms to change mode, 20 ms to change pattern |
//!
//! While a changer moves, no position is in the light path. Changes to one device wait their
//! turn.

use std::sync::Arc;
use std::time::Duration;

use siq_devices::{
    BoxFuture, Changer, ChangerStatus, Device, DeviceError, DeviceInfo, DeviceResult, FilterWheel,
    Illumination, IlluminationStatus, Modality, ModalityStatus, ModalityUnit, ObjectiveTurret,
    PatternSet, watch,
};
use siq_types::{Clock, ModalityMode, Timestamp};
use tokio::sync::Mutex;

use crate::state::TrueState;

const LIGHT: Duration = Duration::from_millis(5);
const WHEEL: Duration = Duration::from_millis(60);
const TURRET: Duration = Duration::from_millis(400);
const MODE: Duration = Duration::from_millis(300);
const PATTERN: Duration = Duration::from_millis(20);

/// Waits until `deadline` on `clock`, in short steps of real time, so that a manual clock may be
/// moved meanwhile.
async fn until(clock: &dyn Clock, deadline: Timestamp) {
    loop {
        let now = clock.now();
        if now >= deadline {
            return;
        }
        let left = deadline.saturating_since(now);
        tokio::time::sleep(left.min(Duration::from_millis(5))).await;
    }
}

fn info(model: &str) -> DeviceInfo {
    DeviceInfo::new("SpectrumIQ", model)
}

fn invalid(message: String) -> DeviceResult<()> {
    Err(DeviceError::InvalidArgument(message))
}

/// Simulated light sources, which light the true state's sources by index.
pub struct SimIllumination {
    info: DeviceInfo,
    sources: Vec<String>,
    state: Arc<TrueState>,
    clock: Arc<dyn Clock>,
    status: watch::Sender<IlluminationStatus>,
}

impl SimIllumination {
    /// Sources named `sources`, at `state`'s levels.
    pub fn new(sources: Vec<String>, state: Arc<TrueState>, clock: Arc<dyn Clock>) -> Self {
        let levels = (0..sources.len())
            .map(|source| state.level(source))
            .collect();
        Self {
            info: info("simulated illumination"),
            sources,
            state,
            clock,
            status: watch::Sender::new(IlluminationStatus { levels }),
        }
    }
}

impl Device for SimIllumination {
    fn info(&self) -> &DeviceInfo {
        &self.info
    }
}

impl Illumination for SimIllumination {
    fn sources(&self) -> &[String] {
        &self.sources
    }

    fn set_level(&self, source: usize, level: f64) -> BoxFuture<'_, DeviceResult<()>> {
        Box::pin(async move {
            if source >= self.sources.len() {
                return invalid(format!("there is no source {source}"));
            }
            if !(0.0..=1.0).contains(&level) {
                return invalid(format!("the level {level} isn't within 0 to 1"));
            }
            until(&*self.clock, self.clock.now() + LIGHT).await;
            self.state.set_level(source, level);
            self.status
                .send_modify(|status| status.levels[source] = level);
            Ok(())
        })
    }

    fn status(&self) -> watch::Receiver<IlluminationStatus> {
        self.status.subscribe()
    }
}

/// Which of the true state's positions a changer moves.
#[derive(Debug, Clone, Copy)]
enum Slot {
    Filter,
    Objective,
}

/// A simulated filter wheel or objective turret.
pub struct SimChanger {
    info: DeviceInfo,
    positions: Vec<String>,
    slot: Slot,
    per_position: Duration,
    state: Arc<TrueState>,
    clock: Arc<dyn Clock>,
    status: watch::Sender<ChangerStatus>,
    turn: Mutex<()>,
}

impl SimChanger {
    /// A filter wheel with `filters`, by position, which moves `state`'s filter.
    pub fn filter_wheel(
        filters: Vec<String>,
        state: Arc<TrueState>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self::new(
            "simulated filter wheel",
            filters,
            Slot::Filter,
            WHEEL,
            state,
            clock,
        )
    }

    /// An objective turret of `positions`, named `"position 0"` and on, which moves `state`'s
    /// objective.
    pub fn objective_turret(positions: u8, state: Arc<TrueState>, clock: Arc<dyn Clock>) -> Self {
        let names = (0..positions).map(|i| format!("position {i}")).collect();
        Self::new(
            "simulated objective turret",
            names,
            Slot::Objective,
            TURRET,
            state,
            clock,
        )
    }

    fn new(
        model: &str,
        positions: Vec<String>,
        slot: Slot,
        per_position: Duration,
        state: Arc<TrueState>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        let position = match slot {
            Slot::Filter => state.filter(),
            Slot::Objective => state.objective(),
        };
        let status = ChangerStatus {
            position,
            moving: false,
        };
        Self {
            info: info(model),
            positions,
            slot,
            per_position,
            state,
            clock,
            status: watch::Sender::new(status),
            turn: Mutex::new(()),
        }
    }

    fn place(&self, position: Option<u8>) {
        match self.slot {
            Slot::Filter => self.state.set_filter(position),
            Slot::Objective => self.state.set_objective(position),
        }
        let moving = position.is_none();
        self.status.send_replace(ChangerStatus { position, moving });
    }
}

impl Device for SimChanger {
    fn info(&self) -> &DeviceInfo {
        &self.info
    }
}

impl Changer for SimChanger {
    fn positions(&self) -> &[String] {
        &self.positions
    }

    fn select(&self, position: u8) -> BoxFuture<'_, DeviceResult<()>> {
        Box::pin(async move {
            let count = self.positions.len();
            if usize::from(position) >= count {
                return invalid(format!("there is no position {position}"));
            }
            let _turn = self.turn.lock().await;
            // The shorter way round; from an unknown position, half of it.
            let steps = match self.status.borrow().position {
                Some(from) => {
                    let apart = usize::from(position.abs_diff(from));
                    apart.min(count - apart)
                }
                None => count / 2,
            };
            if steps > 0 {
                self.place(None);
                let steps = u32::try_from(steps).unwrap_or(u32::MAX);
                until(&*self.clock, self.clock.now() + self.per_position * steps).await;
            }
            self.place(Some(position));
            Ok(())
        })
    }

    fn status(&self) -> watch::Receiver<ChangerStatus> {
        self.status.subscribe()
    }
}

impl FilterWheel for SimChanger {}

impl ObjectiveTurret for SimChanger {}

/// A simulated modality unit.
pub struct SimModalityUnit {
    info: DeviceInfo,
    modes: Vec<ModalityMode>,
    patterns: Option<PatternSet>,
    state: Arc<TrueState>,
    clock: Arc<dyn Clock>,
    status: watch::Sender<ModalityStatus>,
    turn: Mutex<()>,
}

impl SimModalityUnit {
    /// A unit offering `modes`, and `patterns` for structured illumination, which sets `state`'s
    /// modality. It starts in its first mode.
    pub fn new(
        modes: Vec<ModalityMode>,
        patterns: Option<PatternSet>,
        state: Arc<TrueState>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        let mode = modes.first().copied().unwrap_or(ModalityMode::Widefield);
        let modality = Modality {
            mode,
            pattern: None,
        };
        state.set_modality(modality);
        let status = ModalityStatus {
            modality,
            moving: false,
        };
        Self {
            info: info("simulated modality unit"),
            modes,
            patterns,
            state,
            clock,
            status: watch::Sender::new(status),
            turn: Mutex::new(()),
        }
    }
}

impl Device for SimModalityUnit {
    fn info(&self) -> &DeviceInfo {
        &self.info
    }
}

impl ModalityUnit for SimModalityUnit {
    fn modes(&self) -> &[ModalityMode] {
        &self.modes
    }

    fn patterns(&self) -> Option<PatternSet> {
        self.patterns
    }

    fn set_modality(&self, modality: Modality) -> BoxFuture<'_, DeviceResult<()>> {
        Box::pin(async move {
            modality.check(&self.modes, self.patterns)?;
            let _turn = self.turn.lock().await;
            let now = self.status.borrow().modality;
            let wait = match (now.mode == modality.mode, now.pattern == modality.pattern) {
                (false, _) => MODE,
                (true, false) => PATTERN,
                (true, true) => Duration::ZERO,
            };
            let moving = true;
            self.status
                .send_replace(ModalityStatus { modality, moving });
            until(&*self.clock, self.clock.now() + wait).await;
            self.state.set_modality(modality);
            let moving = false;
            self.status
                .send_replace(ModalityStatus { modality, moving });
            Ok(())
        })
    }

    fn status(&self) -> watch::Receiver<ModalityStatus> {
        self.status.subscribe()
    }
}
