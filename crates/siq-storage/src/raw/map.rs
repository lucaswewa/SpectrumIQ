//! A recorded frame mapped read-only, and its bytes as 16-bit pixels: the crate's one use of
//! memory mapping, allowed unsafe code by ADR-0041.

#![allow(unsafe_code)]

use std::fs::{File, OpenOptions};
use std::io;
use std::os::windows::fs::OpenOptionsExt;
use std::path::Path;

use memmap2::Mmap;
use windows_sys::Win32::Storage::FileSystem::FILE_SHARE_READ;

/// `path`, opened sharing reading only, and mapped.
pub(super) fn read_only(path: &Path) -> io::Result<(File, Mmap)> {
    let file = OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ)
        .open(path)?;
    // SAFETY: a mapping is sound only if nothing changes or truncates the file while it lives.
    // The file is open sharing reading only, and the caller keeps it open as long as the mapping:
    // no other handle can be opened to write it, truncate it or delete it meanwhile. And no
    // handle was open to write it, or this open would have failed. The writer never opens a
    // frame again once written.
    let map = unsafe { Mmap::map(&file)? };
    Ok((file, map))
}

/// `bytes` as 16-bit pixels, if they are aligned for them and an even number.
pub(super) fn as_u16(bytes: &[u8]) -> Option<&[u16]> {
    // SAFETY: every pair of bytes is a valid `u16`, and `align_to` puts only the aligned middle
    // in the slice of `u16`, which is all that is used, and only when nothing is left over.
    let (head, pixels, tail) = unsafe { bytes.align_to::<u16>() };
    (head.is_empty() && tail.is_empty()).then_some(pixels)
}
