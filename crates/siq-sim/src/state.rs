//! The true physical state of the simulated instrument (ADR-0012, ADR-0026): what the stage,
//! focus and peripherals really are, as the emulators set it and the camera sees it.
//!
//! Reads take no lock. The pose is behind a sequence lock, so a reader never sees half of a
//! move; the peripherals are single atomics.

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering, fence};
use std::sync::{Mutex, PoisonError};

use siq_devices::{Modality, Pattern};
use siq_types::{Micrometers, ModalityMode, StagePose};

/// `N` numbers that one writer at a time changes, and any number of readers read without a lock.
#[derive(Debug)]
struct SeqLock<const N: usize> {
    sequence: AtomicU64,
    values: [AtomicU64; N],
    writer: Mutex<()>,
}

impl<const N: usize> SeqLock<N> {
    fn new(values: [f64; N]) -> Self {
        Self {
            sequence: AtomicU64::new(0),
            values: values.map(|value| AtomicU64::new(value.to_bits())),
            writer: Mutex::new(()),
        }
    }

    fn load(&self) -> [f64; N] {
        self.values
            .each_ref()
            .map(|value| f64::from_bits(value.load(Ordering::Relaxed)))
    }

    /// The values, as a writer left them: retried while a write is under way.
    fn read(&self) -> [f64; N] {
        loop {
            let before = self.sequence.load(Ordering::Acquire);
            if before.is_multiple_of(2) {
                let values = self.load();
                fence(Ordering::Acquire);
                if self.sequence.load(Ordering::Relaxed) == before {
                    return values;
                }
            }
            std::hint::spin_loop();
        }
    }

    /// Changes the values. Writers take turns; readers retry while one writes.
    fn update(&self, change: impl FnOnce(&mut [f64; N])) {
        let _writer = self.writer.lock().unwrap_or_else(PoisonError::into_inner);
        let mut values = self.load();
        change(&mut values);
        let sequence = self.sequence.load(Ordering::Relaxed);
        self.sequence.store(sequence + 1, Ordering::Relaxed);
        fence(Ordering::Release);
        for (slot, value) in self.values.iter().zip(values) {
            slot.store(value.to_bits(), Ordering::Relaxed);
        }
        self.sequence.store(sequence + 2, Ordering::Release);
    }
}

/// No position: stored in an atomic position.
const NONE: u32 = u32::MAX;

/// The true state: written by the emulators (P1.8, P1.10) and the peripheral simulators, read by
/// the simulated camera.
#[derive(Debug)]
pub struct TrueState {
    pose: SeqLock<4>,
    levels: Box<[AtomicU64]>,
    filter: AtomicU32,
    objective: AtomicU32,
    modality: AtomicU32,
}

impl TrueState {
    /// The stage at the origin, in focus, with `sources` light sources all at full level, the
    /// objective at turret position 0, no filter selected, and widefield.
    pub fn new(sources: usize) -> Self {
        Self {
            pose: SeqLock::new([0.0; 4]),
            levels: (0..sources)
                .map(|_| AtomicU64::new(1.0f64.to_bits()))
                .collect(),
            filter: AtomicU32::new(NONE),
            objective: AtomicU32::new(0),
            modality: AtomicU32::new(pack(Modality::WIDEFIELD)),
        }
    }

    /// The true pose of the stage and focus.
    pub fn pose(&self) -> StagePose {
        let [x, y, z_coarse, focus] = self.pose.read().map(Micrometers);
        StagePose {
            x,
            y,
            z_coarse,
            focus,
        }
    }

    /// Sets the whole pose.
    pub fn set_pose(&self, pose: StagePose) {
        self.pose
            .update(|values| *values = [pose.x, pose.y, pose.z_coarse, pose.focus].map(|v| v.0));
    }

    /// Sets the stage's axes, keeping the focus: what the stage emulator moves.
    pub fn set_stage(&self, x: Micrometers, y: Micrometers, z_coarse: Micrometers) {
        self.pose
            .update(|values| (values[0], values[1], values[2]) = (x.0, y.0, z_coarse.0));
    }

    /// Sets the focus drive, keeping the stage: what the focus emulator moves.
    pub fn set_focus(&self, focus: Micrometers) {
        self.pose.update(|values| values[3] = focus.0);
    }

    /// The level of light source `source`, from 0 to 1; 0 for a source that isn't there.
    pub fn level(&self, source: usize) -> f64 {
        self.levels
            .get(source)
            .map_or(0.0, |level| f64::from_bits(level.load(Ordering::Relaxed)))
    }

    /// Sets the level of light source `source`. A source that isn't there is ignored.
    pub fn set_level(&self, source: usize, level: f64) {
        if let Some(slot) = self.levels.get(source) {
            slot.store(level.to_bits(), Ordering::Relaxed);
        }
    }

    /// How much light reaches the sample, relative to one source at full level: the sum of the
    /// levels, or 1 when no sources are simulated.
    pub fn light(&self) -> f64 {
        match self.levels.len() {
            0 => 1.0,
            sources => (0..sources).map(|source| self.level(source)).sum(),
        }
    }

    /// The filter in the light path.
    pub fn filter(&self) -> Option<u8> {
        unpack_position(self.filter.load(Ordering::Relaxed))
    }

    /// Sets the filter in the light path.
    pub fn set_filter(&self, position: Option<u8>) {
        self.filter
            .store(position.map_or(NONE, u32::from), Ordering::Relaxed);
    }

    /// The objective in the light path, by turret position.
    pub fn objective(&self) -> Option<u8> {
        unpack_position(self.objective.load(Ordering::Relaxed))
    }

    /// Sets the objective in the light path.
    pub fn set_objective(&self, position: Option<u8>) {
        self.objective
            .store(position.map_or(NONE, u32::from), Ordering::Relaxed);
    }

    /// The modality unit's mode and pattern.
    pub fn modality(&self) -> Modality {
        unpack(self.modality.load(Ordering::Relaxed))
    }

    /// Sets the modality unit's mode and pattern.
    pub fn set_modality(&self, modality: Modality) {
        self.modality.store(pack(modality), Ordering::Relaxed);
    }
}

fn unpack_position(value: u32) -> Option<u8> {
    u8::try_from(value).ok()
}

const MODES: [ModalityMode; 4] = [
    ModalityMode::Widefield,
    ModalityMode::LightField,
    ModalityMode::Confocal,
    ModalityMode::StructuredIllumination,
];

/// A modality in one word: the mode's index, a pattern flag, the orientation and the phase.
fn pack(modality: Modality) -> u32 {
    let mode = MODES.iter().position(|&m| m == modality.mode).unwrap_or(0) as u32;
    match modality.pattern {
        Some(Pattern { orientation, phase }) => {
            mode | 1 << 8 | u32::from(orientation) << 16 | u32::from(phase) << 24
        }
        None => mode,
    }
}

fn unpack(value: u32) -> Modality {
    let pattern = (value >> 8 & 1 == 1).then_some(Pattern {
        orientation: (value >> 16) as u8,
        phase: (value >> 24) as u8,
    });
    Modality {
        mode: MODES[(value & 0xff) as usize % MODES.len()],
        pattern,
    }
}
