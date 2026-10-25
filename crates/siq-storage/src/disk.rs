//! The free space on a disk, from Windows: a small Windows-API module, allowed unsafe code by
//! ADR-0007.

#![allow(unsafe_code)]

use std::io;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::ptr::null_mut;

use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;

/// The bytes this process may still write on the disk that holds `path`, an existing folder or
/// file: what its quota leaves.
pub fn free_space(path: &Path) -> io::Result<u64> {
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
    let mut free = 0u64;
    // SAFETY: `wide` is a NUL-terminated UTF-16 path that outlives the call, `free` is a valid
    // place for a u64, and the call accepts null for the two totals not asked for.
    let done = unsafe { GetDiskFreeSpaceExW(wide.as_ptr(), &mut free, null_mut(), null_mut()) };
    if done == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(free)
    }
}
