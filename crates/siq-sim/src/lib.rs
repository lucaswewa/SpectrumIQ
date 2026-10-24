//! SpectrumIQ's simulators (ADR-0012).
//!
//! - [`optics`]: image formation, which renders a realistic frame of a `siq-synth` world for a
//!   stage pose, through an optics and a sensor model (ADR-0026).
//! - [`state`]: the true state of the simulated instrument, which the emulators write and the
//!   camera reads.
//! - [`camera`]: the simulated camera, a `Camera` that renders into pool slots on a camera's
//!   schedule.
//!
//! The GRBL emulator (P1.8) and the peripheral simulators (P1.10) follow.

pub mod camera;
pub mod optics;
pub mod state;
