//! SpectrumIQ's development tasks, run as `cargo xtask <task>`.
//!
//! - [`ci`]: the quality gate every phase passes (conventions §1.3);
//! - [`loc`]: the code budget (conventions §2);
//! - [`layers`]: the crate layering (architecture §3.1).

pub mod ci;
pub mod layers;
pub mod loc;
pub mod metadata;
mod process;
