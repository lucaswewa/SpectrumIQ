//! Euresys eGrabber, through SpectrumIQ's C++ shim (ADR-0004): the raw bindings of
//! `shim/siq_egrabber.h`, which documents each function.
//!
//! - **Without the `sdk` feature** (the default), the types and constants build without the SDK.
//!   The functions are declared but not linked: a call fails to link.
//! - **With `sdk`,** `build.rs` compiles the shim against the SDK at `EGRABBER_SDK_DIR` (default
//!   `C:\Program Files\Euresys\eGrabber`). At run time, the SDK's header loads `egrabber.dll`
//!   through `EURESYS_EGRABBER_LIB64`, which its installer sets.
//!
//! Each function returns a [`SiqEgStatus`], except the closers, `siq_eg_abi_version` and
//! `siq_eg_last_error`. With `sdk`, `last_error` gives a failure's message, on the thread that
//! failed. The safe wrapper is `siq-euresys` (P10.1).
//!
//! `bindings.rs` is generated from the header by `cargo xtask regen-bindings`. No SDK file is in
//! this crate.

#![allow(unsafe_code)]

#[allow(
    non_camel_case_types,
    non_upper_case_globals,
    missing_docs,
    clippy::all
)]
mod bindings;

pub use bindings::*;

/// A status's name, as in the header.
pub fn status_name(status: SiqEgStatus) -> &'static str {
    match status {
        SIQ_EG_OK => "SIQ_EG_OK",
        SIQ_EG_TIMEOUT => "SIQ_EG_TIMEOUT",
        SIQ_EG_ABORTED => "SIQ_EG_ABORTED",
        SIQ_EG_TRUNCATED => "SIQ_EG_TRUNCATED",
        SIQ_EG_INVALID_ARGUMENT => "SIQ_EG_INVALID_ARGUMENT",
        SIQ_EG_GENTL_ERROR => "SIQ_EG_GENTL_ERROR",
        SIQ_EG_ERROR => "SIQ_EG_ERROR",
        _ => "an unknown status",
    }
}

/// The last failure's message on this thread, empty if there was none.
#[cfg(feature = "sdk")]
pub fn last_error() -> String {
    let mut buf = vec![0u8; 256];
    loop {
        // SAFETY: `buf` is writable for `buf.len()` bytes, and the shim writes at most that.
        let needed = unsafe { siq_eg_last_error(buf.as_mut_ptr().cast(), buf.len()) };
        if needed <= buf.len() {
            break;
        }
        buf.resize(needed, 0);
    }
    let message = std::ffi::CStr::from_bytes_until_nul(&buf).unwrap_or_default();
    message.to_string_lossy().into_owned()
}
