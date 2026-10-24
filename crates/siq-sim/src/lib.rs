//! SpectrumIQ's simulators (ADR-0012).
//!
//! - [`optics`]: image formation, which renders a realistic frame of a `siq-synth` world for a
//!   stage pose, through an optics and a sensor model (ADR-0026).
//!
//! The simulated camera (P1.7), the GRBL emulator (P1.8) and the peripheral simulators (P1.10)
//! follow.

pub mod optics;
