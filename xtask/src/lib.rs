//! SpectrumIQ's development tasks, run as `cargo xtask <task>`.
//!
//! - [`ci`]: the quality gate every phase passes (conventions §1.3);
//! - [`loc`]: the code budget (conventions §2);
//! - [`layers`]: the crate layering (architecture §3.1);
//! - [`regen`]: the Euresys shim's bindings, regenerated (ADR-0004);
//! - [`soak`]: viewers and an operator against a service, for a while (ADR-0069).

pub mod ci;
pub mod layers;
pub mod loc;
pub mod metadata;
mod process;
pub mod regen;
pub mod soak;
