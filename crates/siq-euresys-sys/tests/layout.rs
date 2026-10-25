//! The bindings against the header, without the SDK: the structs' layout, which the shim also
//! asserts in C++, and the constants and functions, by name and value.

use std::collections::BTreeSet;
use std::mem::{align_of, offset_of, size_of};
use std::path::Path;

use siq_euresys_sys::*;

fn read(path: &str) -> String {
    std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(path)).unwrap()
}

#[test]
fn the_structs_have_the_layout_the_shim_asserts() {
    assert_eq!(
        (size_of::<SiqEgCounts>(), align_of::<SiqEgCounts>()),
        (12, 4)
    );
    assert_eq!(
        (size_of::<SiqEgCounters>(), align_of::<SiqEgCounters>()),
        (48, 8)
    );
    assert_eq!(
        (size_of::<SiqBufferInfo>(), align_of::<SiqBufferInfo>()),
        (64, 8)
    );
    let offsets = [
        offset_of!(SiqBufferInfo, index),
        offset_of!(SiqBufferInfo, ptr),
        offset_of!(SiqBufferInfo, size),
        offset_of!(SiqBufferInfo, timestamp),
        offset_of!(SiqBufferInfo, frame_id),
        offset_of!(SiqBufferInfo, width),
        offset_of!(SiqBufferInfo, height),
        offset_of!(SiqBufferInfo, pixfmt),
        offset_of!(SiqBufferInfo, flags),
        offset_of!(SiqBufferInfo, pixfmt_namespace),
    ];
    assert_eq!(offsets, [0, 8, 16, 24, 32, 40, 44, 48, 56, 60]);
    // The handles are opaque.
    assert_eq!(size_of::<SiqEgSystem>() + size_of::<SiqEgGrabber>(), 0);
}

#[test]
fn every_constant_of_the_header_is_bound_with_its_value() {
    let bound: BTreeSet<(&str, i64)> = [
        ("SIQ_EG_ABI_VERSION", i64::from(SIQ_EG_ABI_VERSION)),
        (
            "SIQ_EG_BUFFER_INCOMPLETE",
            i64::from(SIQ_EG_BUFFER_INCOMPLETE),
        ),
        ("SIQ_EG_OK", i64::from(SIQ_EG_OK)),
        ("SIQ_EG_TIMEOUT", i64::from(SIQ_EG_TIMEOUT)),
        ("SIQ_EG_ABORTED", i64::from(SIQ_EG_ABORTED)),
        ("SIQ_EG_TRUNCATED", i64::from(SIQ_EG_TRUNCATED)),
        (
            "SIQ_EG_INVALID_ARGUMENT",
            i64::from(SIQ_EG_INVALID_ARGUMENT),
        ),
        ("SIQ_EG_GENTL_ERROR", i64::from(SIQ_EG_GENTL_ERROR)),
        ("SIQ_EG_ERROR", i64::from(SIQ_EG_ERROR)),
        ("SIQ_EG_INTERFACE", i64::from(SIQ_EG_INTERFACE)),
        ("SIQ_EG_DEVICE", i64::from(SIQ_EG_DEVICE)),
        ("SIQ_EG_STREAM", i64::from(SIQ_EG_STREAM)),
        ("SIQ_EG_REMOTE", i64::from(SIQ_EG_REMOTE)),
        ("SIQ_EG_FOUND_INTERFACE", i64::from(SIQ_EG_FOUND_INTERFACE)),
        ("SIQ_EG_FOUND_GRABBER", i64::from(SIQ_EG_FOUND_GRABBER)),
        ("SIQ_EG_FOUND_CAMERA", i64::from(SIQ_EG_FOUND_CAMERA)),
    ]
    .into();
    // `NAME = 1,` in an enum, or `#define NAME 1u`.
    let header = read("shim/siq_egrabber.h");
    let declared: BTreeSet<(&str, i64)> = header
        .lines()
        .filter_map(|line| {
            let line = line.trim().trim_start_matches("#define ");
            let (name, value) = line.split_once(" = ").or_else(|| line.split_once(' '))?;
            let value = value.trim_end_matches([',', 'u']).parse().ok()?;
            name.starts_with("SIQ_EG_").then_some((name, value))
        })
        .collect();
    assert_eq!(declared, bound);
}

#[test]
fn every_function_of_the_header_is_bound() {
    let names = |text: &str, before: &str| -> BTreeSet<String> {
        text.split(before)
            .skip(1)
            .filter_map(|rest| rest.split_once('(').map(|(name, _)| name.trim().to_owned()))
            .filter(|name| name.starts_with("siq_eg_") && !name.contains(' '))
            .collect()
    };
    let header = read("shim/siq_egrabber.h");
    let declared: BTreeSet<String> = ["SiqEgStatus ", "void ", "uint32_t ", "size_t "]
        .into_iter()
        .flat_map(|returns| names(&header, returns))
        .collect();
    let bound = names(&read("src/bindings.rs"), "pub fn ");
    assert_eq!(declared.len(), 22, "{declared:?}");
    assert_eq!(declared, bound);
}

#[test]
fn statuses_have_their_names() {
    assert_eq!(status_name(SIQ_EG_TIMEOUT), "SIQ_EG_TIMEOUT");
    assert_eq!(status_name(SIQ_EG_GENTL_ERROR), "SIQ_EG_GENTL_ERROR");
    assert_eq!(status_name(99), "an unknown status");
}
