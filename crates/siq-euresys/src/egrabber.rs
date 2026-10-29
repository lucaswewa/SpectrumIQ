//! eGrabber, through `siq-euresys-sys`'s shim: the [`Grabber`] of a real Coaxlink. Only with the
//! `sdk` feature.

use std::ffi::{CStr, CString, c_char, c_void};
use std::path::Path;
use std::ptr;
use std::time::Duration;

use siq_euresys_sys::*;

use crate::grabber::{Counters, ErrorKind, Grabber, GrabberError, Module, Popped};

/// A camera opened through eGrabber: its GenTL system, and the grabber of the camera.
pub struct EGrabber {
    system: *mut SiqEgSystem,
    grabber: *mut SiqEgGrabber,
}

// SAFETY: the shim's grabber may move between threads; it is used from one at a time, which
// `&mut self` ensures. The system is used only to open and close it.
unsafe impl Send for EGrabber {}

/// `status` as a result, saying what failed: the shim's message, from this thread.
fn check(status: SiqEgStatus, what: &str) -> Result<(), GrabberError> {
    let kind = match status {
        SIQ_EG_OK => return Ok(()),
        SIQ_EG_INVALID_ARGUMENT => ErrorKind::InvalidArgument,
        _ => ErrorKind::Fault,
    };
    let why = format!("{what}: {} ({})", last_error(), status_name(status));
    Err(GrabberError::new(kind, why))
}

/// A NUL-terminated copy of `text`.
fn c(text: &str) -> Result<CString, GrabberError> {
    CString::new(text).map_err(|_| GrabberError::new(ErrorKind::InvalidArgument, "a NUL in a name"))
}

/// A string the shim writes into a buffer: it asks for the size first.
fn read(
    what: &str,
    call: impl Fn(*mut c_char, usize, *mut usize) -> SiqEgStatus,
) -> Result<String, GrabberError> {
    let mut needed = 0;
    match call(ptr::null_mut(), 0, &mut needed) {
        SIQ_EG_OK | SIQ_EG_TRUNCATED => {}
        status => check(status, what)?,
    }
    let mut buf = vec![0u8; needed.max(1)];
    check(
        call(buf.as_mut_ptr().cast(), buf.len(), ptr::null_mut()),
        what,
    )?;
    let text = CStr::from_bytes_until_nul(&buf).unwrap_or_default();
    Ok(text.to_string_lossy().into_owned())
}

const fn module(module: Module) -> SiqEgModule {
    match module {
        Module::Interface => SIQ_EG_INTERFACE,
        Module::Device => SIQ_EG_DEVICE,
        Module::Stream => SIQ_EG_STREAM,
        Module::Remote => SIQ_EG_REMOTE,
    }
}

impl EGrabber {
    /// Opens the camera at `device` of grabber card `interface`, through `producer`, or the
    /// default GenTL producer (Coaxlink's).
    pub fn open(
        producer: Option<&Path>,
        interface: u32,
        device: u32,
    ) -> Result<Self, GrabberError> {
        let producer = producer
            .map(|path| c(&path.to_string_lossy()))
            .transpose()?;
        let mut system = ptr::null_mut();
        let path = producer.as_ref().map_or(ptr::null(), |path| path.as_ptr());
        // SAFETY: `path` is null or NUL-terminated, and `system` is writable.
        check(
            unsafe { siq_eg_system_open(path, &mut system) },
            "the GenTL producer",
        )?;
        let mut opened = Self {
            system,
            grabber: ptr::null_mut(),
        };
        let mut counts = SiqEgCounts {
            interfaces: 0,
            grabbers: 0,
            cameras: 0,
        };
        // SAFETY: the system is open, and `counts` is writable.
        check(
            unsafe { siq_eg_discover(system, &mut counts) },
            "the discovery",
        )?;
        let wanted = [("interface_index", interface), ("device_index", device)];
        for camera in 0..counts.cameras {
            let info = read("the discovery", |buf, len, needed| {
                // SAFETY: the system is discovered; the buffer is as `read` gives it.
                unsafe {
                    siq_eg_discovery_info(system, SIQ_EG_FOUND_CAMERA, camera, buf, len, needed)
                }
            })?;
            let field = |key: &str| {
                let line = info
                    .lines()
                    .find_map(|line| line.strip_prefix(key)?.strip_prefix('='));
                line.and_then(|value| value.trim().parse::<u32>().ok())
            };
            if wanted.iter().all(|(key, value)| field(key) == Some(*value)) {
                // SAFETY: the system is discovered, and `grabber` is writable.
                let status = unsafe { siq_eg_open(system, camera, &mut opened.grabber) };
                check(status, "opening the camera")?;
                return Ok(opened);
            }
        }
        let why = format!("no camera at device {device} of grabber {interface}");
        Err(GrabberError::new(ErrorKind::NotFound, why))
    }
}

impl EGrabber {
    /// The features `at` offers now, by name: for writing a camera's feature map.
    pub fn features(&mut self, at: Module) -> Result<Vec<String>, GrabberError> {
        let what = SIQ_EG_LIST_FEATURES;
        let list = read("the features", |buf, len, needed| {
            // SAFETY: the grabber is open; the buffer is as `read` gives it. The enumeration's name
            // is ignored when listing features, and empty.
            unsafe {
                siq_eg_list(
                    self.grabber,
                    module(at),
                    what,
                    c"".as_ptr(),
                    buf,
                    len,
                    needed,
                )
            }
        })?;
        Ok(list
            .lines()
            .filter(|line| !line.is_empty())
            .map(str::to_owned)
            .collect())
    }
}

impl Drop for EGrabber {
    /// Closes the grabber, which stops it and revokes its buffers, then the system.
    fn drop(&mut self) {
        // SAFETY: each handle is open or null, and closed once; the grabber before its system.
        unsafe {
            if !self.grabber.is_null() {
                siq_eg_close(self.grabber);
            }
            siq_eg_system_close(self.system);
        }
    }
}

impl Grabber for EGrabber {
    fn get(&mut self, at: Module, feature: &str) -> Result<String, GrabberError> {
        let name = c(feature)?;
        read(feature, |buf, len, needed| {
            // SAFETY: the grabber is open; the buffer is as `read` gives it.
            unsafe { siq_eg_get_string(self.grabber, module(at), name.as_ptr(), buf, len, needed) }
        })
    }

    fn set(&mut self, at: Module, feature: &str, value: &str) -> Result<(), GrabberError> {
        let (name, text) = (c(feature)?, c(value)?);
        // SAFETY: the grabber is open, and both strings are NUL-terminated.
        let status =
            unsafe { siq_eg_set_string(self.grabber, module(at), name.as_ptr(), text.as_ptr()) };
        check(status, &format!("setting {feature} to {value}"))
    }

    fn execute(&mut self, at: Module, command: &str) -> Result<(), GrabberError> {
        let name = c(command)?;
        // SAFETY: the grabber is open, and the name NUL-terminated.
        let status = unsafe { siq_eg_execute(self.grabber, module(at), name.as_ptr()) };
        check(status, &format!("executing {command}"))
    }

    fn entries(&mut self, at: Module, feature: &str) -> Result<Vec<String>, GrabberError> {
        let name = c(feature)?;
        let what = SIQ_EG_LIST_ENUM_ENTRIES;
        let list = read(feature, |buf, len, needed| {
            // SAFETY: as in `get`.
            unsafe {
                siq_eg_list(
                    self.grabber,
                    module(at),
                    what,
                    name.as_ptr(),
                    buf,
                    len,
                    needed,
                )
            }
        })?;
        Ok(list
            .lines()
            .filter(|line| !line.is_empty())
            .map(str::to_owned)
            .collect())
    }

    unsafe fn announce(&mut self, buffers: &[*mut u8], size: usize) -> Result<(), GrabberError> {
        let pointers: Vec<*mut c_void> = buffers.iter().map(|buffer| buffer.cast()).collect();
        let list = if pointers.is_empty() {
            ptr::null()
        } else {
            pointers.as_ptr()
        };
        // SAFETY: the grabber is open; the caller keeps the buffers valid, as the trait says.
        let status =
            unsafe { siq_eg_announce_user_buffers(self.grabber, list, size, pointers.len()) };
        check(status, "announcing the buffers")
    }

    fn start(&mut self) -> Result<(), GrabberError> {
        // SAFETY: the grabber is open.
        check(unsafe { siq_eg_start(self.grabber, 0) }, "starting")
    }

    fn stop(&mut self) -> Result<(), GrabberError> {
        // SAFETY: the grabber is open.
        check(unsafe { siq_eg_stop(self.grabber) }, "stopping")
    }

    fn pop(&mut self, timeout: Duration) -> Result<Option<Popped>, GrabberError> {
        let mut info = SiqBufferInfo {
            index: 0,
            ptr: ptr::null_mut(),
            size: 0,
            timestamp: 0,
            frame_id: 0,
            width: 0,
            height: 0,
            pixfmt: 0,
            flags: 0,
            pixfmt_namespace: 0,
        };
        let ms = u64::try_from(timeout.as_millis()).unwrap_or(u64::MAX);
        // SAFETY: the grabber is open, and `info` is writable.
        match unsafe { siq_eg_pop(self.grabber, ms, &mut info) } {
            SIQ_EG_TIMEOUT | SIQ_EG_ABORTED => Ok(None),
            status => check(status, "waiting for a frame").map(|()| {
                Some(Popped {
                    index: info.index as usize,
                    bytes: info.size as usize,
                    timestamp_ns: info.timestamp,
                    frame_id: info.frame_id,
                    incomplete: info.flags & SIQ_EG_BUFFER_INCOMPLETE != 0,
                })
            }),
        }
    }

    fn requeue(&mut self, index: usize) -> Result<(), GrabberError> {
        // SAFETY: the grabber is open; the shim refuses an index it didn't pop.
        check(
            unsafe { siq_eg_requeue(self.grabber, index as u64) },
            "requeuing a buffer",
        )
    }

    fn counters(&mut self) -> Result<Counters, GrabberError> {
        let mut counters = SiqEgCounters {
            announced: 0,
            queued: 0,
            awaiting_delivery: 0,
            delivered: 0,
            underrun: 0,
            started: 0,
        };
        // SAFETY: the grabber is open, and `counters` is writable.
        check(
            unsafe { siq_eg_counters(self.grabber, &mut counters) },
            "the counters",
        )?;
        Ok(Counters {
            delivered: counters.delivered,
            underrun: counters.underrun,
        })
    }
}
