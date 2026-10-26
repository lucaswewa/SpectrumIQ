//! Workflows: what the instrument does on its own, step by step.
//!
//! - [`engine`] runs acquisition plans (ADR-0052): their dataset, their steps, their progress,
//!   pausing, cancelling and error policies.
//! - [`routines`] move the instrument and measure it: the stage–camera calibration (ADR-0049),
//!   the shading's captures (ADR-0050), and a channel put in place.
//! - [`DeviceSet`] is the instrument as they see it.

mod devices;
pub mod engine;
pub mod routines;

pub use devices::DeviceSet;
