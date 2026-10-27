//! Slot memory: page-aligned regions, from the heap or from `VirtualAlloc`, and the working-set
//! reservation that lets `VirtualLock` keep them in RAM.

#![allow(unsafe_code)]

use std::alloc::{Layout, alloc_zeroed, dealloc};
use std::io;
use std::ptr::{NonNull, null};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use windows_sys::Win32::Foundation::FILETIME;
use windows_sys::Win32::System::Memory::{
    MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_READWRITE, VirtualAlloc, VirtualFree, VirtualLock,
    VirtualUnlock,
};
use windows_sys::Win32::System::ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, GetProcessIoCounters, GetProcessTimes, GetProcessWorkingSetSize,
    IO_COUNTERS, SetProcessWorkingSetSize,
};

/// The boundary every slot starts on, and the unit its size rounds up to: a page. It also suits
/// unbuffered file writes, which need sector-aligned buffers.
pub const PAGE: usize = 4096;

/// What the working-set minimum is raised by, beyond the locked slots: room for the rest of the
/// process.
const MARGIN: usize = 64 << 20;

/// Where slot memory comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    /// The heap. Pages may be swapped out. For tests, tools and Miri.
    Heap,
    /// `VirtualAlloc`, locked in RAM with `VirtualLock`, as a grabber's DMA wants. The process's
    /// minimum working set is raised to hold the slots.
    WindowsLocked,
}

/// One slot's memory: `len` bytes, page-aligned, zero-filled when allocated, freed when dropped.
pub(crate) struct Region {
    ptr: NonNull<u8>,
    len: usize,
    backend: Backend,
}

// SAFETY: a region is plain bytes owned by the pool. Which thread may read or write them is
// decided by the pool's ownership protocol (slot.rs), not by the region.
unsafe impl Send for Region {}
// SAFETY: as above; a `&Region` gives no access to the bytes by itself.
unsafe impl Sync for Region {}

impl Region {
    /// Allocates `len` bytes, a non-zero multiple of [`PAGE`].
    pub(crate) fn allocate(len: usize, backend: Backend) -> io::Result<Self> {
        assert!(
            len > 0 && len.is_multiple_of(PAGE),
            "a region is whole pages"
        );
        let ptr = match backend {
            Backend::Heap => {
                let layout = Layout::from_size_align(len, PAGE).map_err(io::Error::other)?;
                // SAFETY: the layout's size is not zero.
                let ptr = unsafe { alloc_zeroed(layout) };
                NonNull::new(ptr).ok_or_else(|| io::Error::from(io::ErrorKind::OutOfMemory))?
            }
            Backend::WindowsLocked => {
                // SAFETY: a new allocation at an address the system chooses; it aliases nothing.
                // Committed pages are zero-filled.
                let ptr =
                    unsafe { VirtualAlloc(null(), len, MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE) };
                NonNull::new(ptr.cast::<u8>()).ok_or_else(io::Error::last_os_error)?
            }
        };
        Ok(Self { ptr, len, backend })
    }

    /// The first byte.
    pub(crate) fn ptr(&self) -> *mut u8 {
        self.ptr.as_ptr()
    }

    /// The size in bytes.
    pub(crate) fn len(&self) -> usize {
        self.len
    }

    /// Locks the region in RAM, so that it is never paged out.
    pub(crate) fn lock(&self) -> io::Result<()> {
        // SAFETY: the region is a live allocation of `len` bytes.
        let locked = unsafe { VirtualLock(self.ptr().cast(), self.len) };
        if locked == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
}

impl Drop for Region {
    fn drop(&mut self) {
        match self.backend {
            Backend::Heap => {
                // SAFETY: the pointer came from `alloc_zeroed` with this very layout, which was
                // valid then.
                unsafe {
                    dealloc(
                        self.ptr(),
                        Layout::from_size_align_unchecked(self.len, PAGE),
                    )
                }
            }
            Backend::WindowsLocked => {
                // SAFETY: the pointer came from `VirtualAlloc` and is released once, here. Unlocking
                // a region that was never locked fails harmlessly.
                unsafe {
                    VirtualUnlock(self.ptr().cast(), self.len);
                    VirtualFree(self.ptr().cast(), 0, MEM_RELEASE);
                }
            }
        }
    }
}

/// Serializes changes to the process's working set, which pools share.
static WORKING_SET: Mutex<()> = Mutex::new(());

/// The process's minimum working set, raised to hold some locked bytes until this is dropped.
///
/// `VirtualLock` can lock only as many pages as the minimum working set holds, less some the
/// process needs anyway, so the minimum is raised by the locked bytes plus a margin.
#[derive(Debug)]
pub(crate) struct WorkingSetReservation {
    raised_by: usize,
}

impl WorkingSetReservation {
    /// Raises the minimum to hold `locked` more bytes, and the maximum to stay above it.
    pub(crate) fn raise(locked: usize) -> io::Result<Self> {
        let raised_by = locked.saturating_add(MARGIN);
        let _serial = WORKING_SET.lock().unwrap_or_else(PoisonError::into_inner);
        let (min, max) = working_set()?;
        let min = min.saturating_add(raised_by);
        set_working_set(min, max.max(min.saturating_add(MARGIN)))?;
        Ok(Self { raised_by })
    }
}

impl Drop for WorkingSetReservation {
    fn drop(&mut self) {
        let _serial = WORKING_SET.lock().unwrap_or_else(PoisonError::into_inner);
        if let Ok((min, max)) = working_set() {
            // Failing to lower it only leaves the process more memory than it needs.
            let _ = set_working_set(min.saturating_sub(self.raised_by), max);
        }
    }
}

/// The process's minimum and maximum working set, in bytes.
pub fn working_set() -> io::Result<(usize, usize)> {
    let (mut min, mut max) = (0, 0);
    // SAFETY: the pseudo-handle of the current process, and two valid out-pointers.
    let ok = unsafe { GetProcessWorkingSetSize(GetCurrentProcess(), &mut min, &mut max) };
    if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok((min, max))
    }
}

/// The process's memory, in bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProcessMemory {
    /// The physical memory in use: the working set.
    pub working_set: usize,
    /// The largest the working set has been.
    pub peak_working_set: usize,
    /// The memory committed to the process alone: what it would need of the page file.
    pub private: usize,
}

/// The process's memory now.
pub fn process_memory() -> io::Result<ProcessMemory> {
    // SAFETY: an all-zero struct of integers is valid.
    let mut counters: PROCESS_MEMORY_COUNTERS = unsafe { std::mem::zeroed() };
    let size = size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
    // SAFETY: the pseudo-handle of the current process, and a struct of `size` bytes to fill.
    let ok = unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), &mut counters, size) };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(ProcessMemory {
        working_set: counters.WorkingSetSize,
        peak_working_set: counters.PeakWorkingSetSize,
        private: counters.PagefileUsage,
    })
}

/// The process's work so far: its processors' time, and what it read and wrote by calls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProcessUsage {
    /// The time its threads have run, in user and kernel mode.
    pub cpu: Duration,
    /// The bytes it has read by calls such as `ReadFile`: a mapped file's pages, faulted in, are
    /// not among them.
    pub read_bytes: u64,
    /// The bytes it has written by calls.
    pub written_bytes: u64,
}

/// The process's work so far.
pub fn process_usage() -> io::Result<ProcessUsage> {
    let mut times = [FILETIME::default(); 4];
    let [created, exited, kernel, user] = &mut times;
    // SAFETY: the pseudo-handle of the current process, and four valid out-pointers.
    let ok = unsafe { GetProcessTimes(GetCurrentProcess(), created, exited, kernel, user) };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    let mut counters = IO_COUNTERS::default();
    // SAFETY: the pseudo-handle of the current process, and a struct to fill.
    let ok = unsafe { GetProcessIoCounters(GetCurrentProcess(), &mut counters) };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    // A FILETIME counts 100 ns.
    let ticks =
        |time: &FILETIME| (u64::from(time.dwHighDateTime) << 32) | u64::from(time.dwLowDateTime);
    Ok(ProcessUsage {
        cpu: Duration::from_nanos((ticks(&times[2]) + ticks(&times[3])) * 100),
        read_bytes: counters.ReadTransferCount,
        written_bytes: counters.WriteTransferCount,
    })
}

fn set_working_set(min: usize, max: usize) -> io::Result<()> {
    // SAFETY: the pseudo-handle of the current process.
    let ok = unsafe { SetProcessWorkingSetSize(GetCurrentProcess(), min, max) };
    if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}
