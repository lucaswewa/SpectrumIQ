//! Below-normal thread priority, from Windows: a small Windows-API module, allowed unsafe code by
//! ADR-0007. Jobs yield to acquisition, the previews and the service (architecture §4.3).

#![allow(unsafe_code)]

use windows_sys::Win32::System::Threading::{
    GetCurrentThread, GetThreadPriority, SetThreadPriority, THREAD_PRIORITY_BELOW_NORMAL,
};

/// Lowers the calling thread to below-normal priority.
pub(crate) fn lower_this_thread() {
    // SAFETY: `GetCurrentThread` returns a pseudo-handle to the calling thread, always valid in it,
    // which `SetThreadPriority` only reads.
    let lowered = unsafe { SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_BELOW_NORMAL) };
    if lowered == 0 {
        tracing::warn!("a job thread's priority couldn't be lowered");
    }
}

/// The calling thread's priority, as Windows numbers it: −1 is below normal, 0 normal.
pub fn this_thread_priority() -> i32 {
    // SAFETY: as above; `GetThreadPriority` only reads the pseudo-handle.
    unsafe { GetThreadPriority(GetCurrentThread()) }
}
