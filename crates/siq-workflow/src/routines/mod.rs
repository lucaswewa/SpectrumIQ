//! Routines: short sequences of moves and snaps that measure the instrument, on the device traits
//! and the acquisition engine, for any instrument, simulated or real.
//!
//! - [`stage_camera`]: the moves and image shifts that `siq-calib` fits the stage–camera affine
//!   from (ADR-0049).
//! - [`shading`]: the frames of the dark and of the flat (ADR-0050).

pub mod shading;
pub mod stage_camera;
