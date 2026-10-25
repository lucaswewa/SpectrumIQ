//! The shim, built against the installed SDK (`--features sdk`): its ABI, and refusals. Ignored
//! unless asked for with `--ignored`: discovery with this PC's producer, and a grab from
//! Playlink's virtual camera into our own buffers.

#![cfg(feature = "sdk")]
#![allow(unsafe_code)]

use std::alloc::{Layout, alloc_zeroed, dealloc};
use std::ffi::{CStr, CString};
use std::ptr;

use siq_euresys_sys::*;

fn ok(status: SiqEgStatus) {
    assert_eq!(
        status,
        SIQ_EG_OK,
        "{}: {}",
        status_name(status),
        last_error()
    );
}

fn counts() -> SiqEgCounts {
    SiqEgCounts {
        interfaces: 0,
        grabbers: 0,
        cameras: 0,
    }
}

/// A system on `producer`, or the default one, with its discovery's counts.
fn discover(producer: Option<&str>) -> (*mut SiqEgSystem, SiqEgCounts) {
    let producer = producer.map(|path| CString::new(path).unwrap());
    let path = producer.as_ref().map_or(ptr::null(), |path| path.as_ptr());
    let (mut system, mut counts) = (ptr::null_mut(), counts());
    // SAFETY: `path` is null or NUL-terminated, and `system` and `counts` are writable.
    unsafe {
        ok(siq_eg_system_open(path, &mut system));
        ok(siq_eg_discover(system, &mut counts));
    }
    (system, counts)
}

#[test]
fn the_shim_is_built_for_these_bindings() {
    // SAFETY: it takes nothing.
    assert_eq!(unsafe { siq_eg_abi_version() }, SIQ_EG_ABI_VERSION);
}

#[test]
fn bad_arguments_are_refused_with_a_message_and_nothing_throws() {
    let mut counts = counts();
    // SAFETY: the shim checks for null handles before using them.
    unsafe {
        assert_eq!(
            siq_eg_discover(ptr::null_mut(), &mut counts),
            SIQ_EG_INVALID_ARGUMENT
        );
        assert_eq!(last_error(), "no system or counts");
        assert_eq!(siq_eg_requeue(ptr::null_mut(), 0), SIQ_EG_INVALID_ARGUMENT);
        assert_eq!(last_error(), "no grabber");
        let feature = c"Width".as_ptr();
        let mut value = 0;
        let status = siq_eg_get_int(ptr::null_mut(), SIQ_EG_REMOTE, feature, &mut value);
        assert_eq!(status, SIQ_EG_INVALID_ARGUMENT);
        // Closing nothing is nothing.
        siq_eg_close(ptr::null_mut());
        siq_eg_system_close(ptr::null_mut());
    }
    // The message is per thread.
    let other = std::thread::spawn(last_error).join().unwrap();
    assert_eq!(other, "");
}

#[test]
#[ignore = "hardware: the installed GenTL producer"]
fn discovery_returns_without_error() {
    let (system, counts) = discover(None);
    println!("{counts:?}");
    assert!(counts.cameras <= counts.grabbers, "{counts:?}");
    // SAFETY: `system` is open, with no grabber.
    unsafe { siq_eg_system_close(system) };
}

/// A string feature, read as the header says: a short buffer is truncated, with the size needed.
///
/// # Safety
///
/// `grabber` is open.
unsafe fn string(grabber: *mut SiqEgGrabber, module: SiqEgModule, feature: &CStr) -> String {
    let (mut short, mut needed) = ([0u8; 2], 0);
    // SAFETY: `short` is writable for its length, and `needed` is writable.
    let status = unsafe {
        let into = short.as_mut_ptr().cast();
        siq_eg_get_string(grabber, module, feature.as_ptr(), into, 2, &mut needed)
    };
    assert_eq!(status, SIQ_EG_TRUNCATED, "{}", last_error());
    let mut buf = vec![0u8; needed];
    // SAFETY: as above, with `needed` bytes.
    ok(unsafe {
        let into = buf.as_mut_ptr().cast();
        siq_eg_get_string(
            grabber,
            module,
            feature.as_ptr(),
            into,
            needed,
            ptr::null_mut(),
        )
    });
    let value = CStr::from_bytes_until_nul(&buf).unwrap().to_str().unwrap();
    assert_eq!(&short[..1], &value.as_bytes()[..1], "the truncated start");
    value.to_owned()
}

/// Page-aligned memory for a grabber to fill.
struct Memory {
    ptr: *mut u8,
    layout: Layout,
}

impl Memory {
    fn new(size: usize) -> Self {
        let layout = Layout::from_size_align(size, 4096).unwrap();
        // SAFETY: the size isn't zero.
        let ptr = unsafe { alloc_zeroed(layout) };
        assert!(!ptr.is_null());
        Self { ptr, layout }
    }

    fn contains(&self, ptr: *mut std::ffi::c_void) -> bool {
        ptr.cast() == self.ptr
    }
}

impl Drop for Memory {
    fn drop(&mut self) {
        // SAFETY: allocated in `new` with this layout.
        unsafe { dealloc(self.ptr, self.layout) };
    }
}

#[test]
#[ignore = "hardware: Playlink, from the SDK's installer"]
fn playlink_fills_our_buffers_and_takes_them_back() {
    let producer = std::env::var("EURESYS_PLAYLINK_GENTL64_CTI").expect("Playlink installed");
    let (system, counts) = discover(Some(&producer));
    assert!(counts.cameras >= 1, "{counts:?}");
    let mut grabber = ptr::null_mut();
    let (mut width, mut height, mut payload) = (0, 0, 0);
    // SAFETY: `system` is open and discovered, and every out-pointer is writable.
    unsafe {
        ok(siq_eg_open(system, 0, &mut grabber));
        ok(siq_eg_get_int(
            grabber,
            SIQ_EG_REMOTE,
            c"Width".as_ptr(),
            &mut width,
        ));
        ok(siq_eg_get_int(
            grabber,
            SIQ_EG_REMOTE,
            c"Height".as_ptr(),
            &mut height,
        ));
        let size = c"PayloadSize".as_ptr();
        ok(siq_eg_get_int(grabber, SIQ_EG_STREAM, size, &mut payload));
    }
    // SAFETY: `grabber` is open.
    let format = unsafe { string(grabber, SIQ_EG_REMOTE, c"PixelFormat") };
    println!("{width} × {height} {format}, {payload} bytes a frame");
    let memory: Vec<Memory> = (0..4).map(|_| Memory::new(payload as usize)).collect();
    let ptrs: Vec<*mut std::ffi::c_void> = memory.iter().map(|m| m.ptr.cast()).collect();

    let mut frames = Vec::new();
    // SAFETY: the buffers outlive the grabber, which is closed below; each popped `info` is
    // writable, and each index requeued was popped.
    unsafe {
        let count = ptrs.len();
        ok(siq_eg_announce_user_buffers(
            grabber,
            ptrs.as_ptr(),
            payload as usize,
            count,
        ));
        ok(siq_eg_start(grabber, 0));
        for _ in 0..8 {
            let mut info: SiqBufferInfo = std::mem::zeroed();
            ok(siq_eg_pop(grabber, 5000, &mut info));
            frames.push(info);
            ok(siq_eg_requeue(grabber, info.index));
        }
        // Requeued, it can't be requeued again until popped.
        let last = frames.last().unwrap().index;
        assert_eq!(siq_eg_requeue(grabber, last), SIQ_EG_INVALID_ARGUMENT);
        let mut counters: SiqEgCounters = std::mem::zeroed();
        ok(siq_eg_counters(grabber, &mut counters));
        println!("{counters:?}");
        assert!(
            counters.delivered >= 8 && counters.announced == 4,
            "{counters:?}"
        );
        ok(siq_eg_stop(grabber));
        siq_eg_close(grabber);
        siq_eg_system_close(system);
    }
    for info in &frames {
        assert!(memory[info.index as usize].contains(info.ptr), "{info:?}");
        assert_eq!((info.width, info.height), (width as u32, height as u32));
        assert!(info.size > 0 && info.size <= payload as u64, "{info:?}");
    }
    // Playlink numbers no frame (every `frame_id` is 0), but times each one, in ns.
    let times: Vec<u64> = frames.iter().map(|info| info.timestamp).collect();
    assert!(times.windows(2).all(|pair| pair[0] < pair[1]), "{times:?}");
    assert!(frames.iter().all(|info| info.flags == 0), "{frames:?}");
    let (first, ids) = (&frames[0], frames.iter().map(|info| info.frame_id));
    println!(
        "frame ids {:?}, times {times:?} ns, format {:#x} in namespace {}",
        ids.collect::<Vec<_>>(),
        first.pixfmt,
        first.pixfmt_namespace
    );
}
