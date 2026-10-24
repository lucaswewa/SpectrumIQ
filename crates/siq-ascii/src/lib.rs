//! Data-driven ASCII dialects (REQ-STG-002, ADR-0030): a device's serial protocol, described by a
//! TOML file instead of code.
//!
//! - **A [`Dialect`]** gives the terminators, the step size, the timeouts, how the device moves and
//!   how the end of a move shows, the replies every command may get, and the commands.
//! - **A [`Template`]** is a line with fields, such as `MA {target:steps}`. It renders a line from
//!   values, and matches a line back into values through the anchored regex it compiles. So one
//!   description serves the driver and the emulator.
//! - **[`Dialect::parse`]** checks the whole file, and reports every problem with its path.
//!
//! The focus-drive driver over a dialect follows in P1.10b. `siq-sim`'s `DialectEmulator` is the
//! device side.

mod dialect;
mod template;

pub use dialect::{
    Command, Commands, Dialect, DialectError, Done, Motion, Replies, Terminators, Timeouts, lengths,
};
pub use template::{Kind, Template, Value, Values};
