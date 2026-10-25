//! Lists what an eGrabber GenTL producer sees: the interfaces (frame grabber cards), the grabbers,
//! and the cameras, with what each says about itself.
//!
//! ```text
//! cargo run -p siq-euresys-sys --features sdk --example eg_discover [--open] [producer.cti]
//! ```
//!
//! With no producer, it loads the default one (`EURESYS_DEFAULT_GENTL_PRODUCER`, normally
//! Coaxlink's). `%EURESYS_PLAYLINK_GENTL64_CTI%` loads Playlink, Euresys's virtual producer.
//! With `--open`, it also opens each camera and reads its size and pixel format, changing
//! nothing.

#![allow(unsafe_code)]

use std::ffi::{CStr, CString};
use std::process::ExitCode;
use std::ptr;

use siq_euresys_sys::*;

fn check(status: SiqEgStatus, what: &str) -> Result<(), String> {
    match status {
        SIQ_EG_OK => Ok(()),
        _ => Err(format!("{what}: {}, {}", status_name(status), last_error())),
    }
}

/// The features `--open` reads from each camera, by module.
const FEATURES: [(SiqEgModule, &CStr); 7] = [
    (SIQ_EG_REMOTE, c"DeviceModelName"),
    (SIQ_EG_REMOTE, c"DeviceFirmwareVersion"),
    (SIQ_EG_REMOTE, c"Width"),
    (SIQ_EG_REMOTE, c"Height"),
    (SIQ_EG_REMOTE, c"PixelFormat"),
    (SIQ_EG_STREAM, c"PayloadSize"),
    (SIQ_EG_INTERFACE, c"InterfaceID"),
];

/// A feature's value, as a string: GenICam converts any type.
fn feature(grabber: *mut SiqEgGrabber, module: SiqEgModule, name: &CStr) -> String {
    let mut buf = vec![0u8; 256];
    // SAFETY: `grabber` is open, and `buf` is writable for its length.
    let status = unsafe {
        let into = buf.as_mut_ptr().cast();
        siq_eg_get_string(
            grabber,
            module,
            name.as_ptr(),
            into,
            buf.len(),
            ptr::null_mut(),
        )
    };
    match status {
        SIQ_EG_OK | SIQ_EG_TRUNCATED => CStr::from_bytes_until_nul(&buf)
            .map(|value| value.to_string_lossy().into_owned())
            .unwrap_or_default(),
        _ => format!("({})", status_name(status)),
    }
}

/// Opens camera `index`, and prints its features.
fn open(system: *mut SiqEgSystem, index: i32) -> Result<(), String> {
    let mut grabber = ptr::null_mut();
    // SAFETY: `system` is open and discovered, and `grabber` is writable.
    check(
        unsafe { siq_eg_open(system, index, &mut grabber) },
        "opening the camera",
    )?;
    for (module, name) in FEATURES {
        let name_shown = name.to_string_lossy();
        println!("  {name_shown:<22} {}", feature(grabber, module, name));
    }
    // SAFETY: `grabber` is open; nothing was announced or started.
    unsafe { siq_eg_close(grabber) };
    Ok(())
}

/// What the discovery found at `index` of `found`, as its `key=value` lines.
fn info(system: *mut SiqEgSystem, found: SiqEgFound, index: i32) -> Result<String, String> {
    let mut needed = 0;
    // SAFETY: a null buffer of length 0 asks only for the size, written to `needed`.
    let status =
        unsafe { siq_eg_discovery_info(system, found, index, ptr::null_mut(), 0, &mut needed) };
    if status != SIQ_EG_TRUNCATED {
        check(status, "the discovery's information")?;
    }
    let mut buf = vec![0u8; needed];
    let into = buf.as_mut_ptr().cast();
    // SAFETY: `buf` is writable for `needed` bytes.
    let status =
        unsafe { siq_eg_discovery_info(system, found, index, into, needed, ptr::null_mut()) };
    check(status, "the discovery's information")?;
    let text = CStr::from_bytes_until_nul(&buf).map_err(|error| error.to_string())?;
    Ok(text.to_string_lossy().into_owned())
}

fn list(system: *mut SiqEgSystem, opening: bool) -> Result<(), String> {
    let mut counts = SiqEgCounts {
        interfaces: 0,
        grabbers: 0,
        cameras: 0,
    };
    // SAFETY: `system` is open, and `counts` is writable.
    check(unsafe { siq_eg_discover(system, &mut counts) }, "discovery")?;
    println!(
        "{} interfaces, {} grabbers, {} cameras",
        counts.interfaces, counts.grabbers, counts.cameras
    );
    let kinds = [
        (SIQ_EG_FOUND_INTERFACE, "interface", counts.interfaces),
        (SIQ_EG_FOUND_GRABBER, "grabber", counts.grabbers),
        (SIQ_EG_FOUND_CAMERA, "camera", counts.cameras),
    ];
    for (found, kind, count) in kinds {
        for index in 0..count {
            println!("{kind} {index}:");
            let info = info(system, found, index)?;
            let lines = info.lines().filter_map(|line| line.split_once('='));
            for (key, value) in lines.filter(|(_, value)| !value.is_empty()) {
                println!("  {key:<22} {value}");
            }
            if opening && found == SIQ_EG_FOUND_CAMERA {
                open(system, index)?;
            }
        }
    }
    Ok(())
}

fn run() -> Result<(), String> {
    // SAFETY: it takes nothing.
    println!("shim ABI {}", unsafe { siq_eg_abi_version() });
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let opening = args.iter().any(|arg| arg == "--open");
    args.retain(|arg| arg != "--open");
    let producer = args.into_iter().next().map(CString::new).transpose();
    let producer = producer.map_err(|error| error.to_string())?;
    let path = producer.as_ref().map_or(ptr::null(), |path| path.as_ptr());
    let mut system = ptr::null_mut();
    // SAFETY: `path` is null or a NUL-terminated string, and `system` is writable.
    let status = unsafe { siq_eg_system_open(path, &mut system) };
    check(status, "loading the GenTL producer")?;
    let listed = list(system, opening);
    // SAFETY: `system` is open, with no grabber open on it.
    unsafe { siq_eg_system_close(system) };
    listed
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("eg_discover: {error}");
            ExitCode::FAILURE
        }
    }
}
