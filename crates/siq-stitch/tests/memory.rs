//! Registering pairs holds at most `threads` × `bytes_per_pair` (ADR-0061): a counting global
//! allocator measures the peak of live heap bytes while a row of flat-corrected tiles is
//! registered, one pair at a time, then two. REQ-STI-002, REQ-NFR-002.
//!
//! It is the only test in its binary, so the allocator counts nothing but the registration.

#![allow(unsafe_code)] // a global allocator can only be written with `unsafe`

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicIsize, Ordering};

use siq_calib::shading::{DEGREE, FlatField};
use siq_stitch::pairs::{self, PairOptions};
use siq_stitch::{Mosaic, Tile};
use siq_storage::FrameKey;
use siq_types::OwnedImage;

static LIVE: AtomicIsize = AtomicIsize::new(0);
static PEAK: AtomicIsize = AtomicIsize::new(0);

struct Counting;

fn grew(by: usize) {
    let live = LIVE.fetch_add(by as isize, Ordering::Relaxed) + by as isize;
    PEAK.fetch_max(live, Ordering::Relaxed);
}

fn shrank(by: usize) {
    LIVE.fetch_sub(by as isize, Ordering::Relaxed);
}

// SAFETY: every call is forwarded to the system allocator unchanged; only sizes are counted.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        grew(layout.size());
        // SAFETY: forwarded with the caller's guarantees.
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        grew(layout.size());
        // SAFETY: forwarded with the caller's guarantees.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        grew(new_size);
        shrank(layout.size());
        // SAFETY: forwarded with the caller's guarantees.
        unsafe { System.realloc(ptr, layout, new_size) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        shrank(layout.size());
        // SAFETY: forwarded with the caller's guarantees.
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

#[test]
fn registering_holds_at_most_threads_times_a_pair_s_bytes() {
    // Five 1024 × 768 tiles in a row, 30 % over each other, of one texture.
    let texture = |x: usize, y: usize| {
        let (fx, fy) = (x as f64, y as f64);
        (20_000.0 + 8_000.0 * ((fx * 0.11).sin() * (fy * 0.07).cos())) as u16
    };
    let step = 717;
    let images: Vec<OwnedImage<u16>> = (0..5)
        .map(|k| OwnedImage::from_fn(1024, 768, |x, y| texture(x + k * step, y)))
        .collect();
    let tiles = (0..5)
        .map(|k| Tile {
            key: FrameKey {
                t: 0,
                c: 0,
                z: 0,
                p: k,
            },
            size: [1024, 768],
            nominal: [f64::from(k) * step as f64, 0.0],
            sensor: [0, 0],
            binning: 1,
            cfa: None,
        })
        .collect();
    let mosaic = Mosaic {
        tiles,
        px_per_um: 2.66,
    };
    let options = PairOptions::default();
    let pairs = mosaic.overlaps(options.min_overlap_px, options.min_overlap_share);
    assert_eq!(pairs.len(), 4);
    let mut coefficients = vec![0.0; (DEGREE + 1) * (DEGREE + 2) / 2];
    coefficients[0] = 1.0;
    let flat = FlatField {
        sensor: [1024, 768],
        coefficients,
    };
    let each = pairs::bytes_per_pair(&mosaic, &pairs, &options) as isize;
    for threads in [1, 2] {
        let before = LIVE.load(Ordering::Relaxed);
        PEAK.store(before, Ordering::Relaxed);
        let edges = pairs::register(
            &images[..],
            &mosaic,
            &pairs,
            Some((0.0, &flat)),
            &options,
            threads,
        )
        .unwrap();
        let held = PEAK.load(Ordering::Relaxed) - before;
        eprintln!(
            "{threads} at a time: {:.1} MB held at most, against {:.1} MB a pair",
            held as f64 / 1e6,
            each as f64 / 1e6
        );
        assert!(edges.iter().all(|edge| edge.confident), "{edges:?}");
        assert!(
            held <= threads as isize * each,
            "{held} against {each} a pair"
        );
    }
}
