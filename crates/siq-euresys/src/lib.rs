//! SpectrumIQ's Euresys camera (ADR-0070): a `siq-devices` [`Camera`](siq_devices::Camera) that
//! fills the frame pool's slots by DMA from a Coaxlink grabber.
//!
//! - [`EuresysCamera`] works over any [`Grabber`]: eGrabber's (`EGrabber`, with the `sdk`
//!   feature), or a fake in tests.
//! - [`FeatureMap`] names the camera's features where they differ from GenICam SFNC's, from a
//!   TOML file for the camera model (`config/cameras/`).

// ADR-0007 allows `unsafe` here: the shim's FFI, and DMA into the pool's slots.
#![allow(unsafe_code)]

mod camera;
#[cfg(feature = "sdk")]
mod egrabber;
mod grabber;
mod map;

pub use camera::{EuresysCamera, Strobe};
#[cfg(feature = "sdk")]
pub use egrabber::EGrabber;
pub use grabber::{Counters, ErrorKind, Grabber, GrabberError, Module, Popped};
pub use map::{FeatureMap, MapError, Setting, Triggers, module_of};
