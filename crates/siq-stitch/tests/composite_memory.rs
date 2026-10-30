//! Compositing holds at most `composite::bytes` (ADR-0063): a counting global allocator measures
//! the peak of live heap bytes while a 4 × 4 mosaic of flat-corrected tiles is composited into
//! OME-Zarr on two threads. REQ-STI-003, REQ-NFR-002.
//!
//! It is the only test in its binary, so the allocator counts nothing but the composite.

#![allow(unsafe_code)] // a global allocator can only be written with `unsafe`

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicIsize, Ordering};

use siq_calib::shading::{DEGREE, FlatField};
use siq_stitch::Tile;
use siq_stitch::composite::{self, Bounds, CompositeOptions, Placed, Plane};
use siq_storage::FrameKey;
use siq_storage::zarr::{ChannelInfo, ImageInfo, SampleType, Size5, ZarrImage};
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
fn compositing_holds_its_blocks_and_chunks_not_the_mosaic() {
    // Sixteen 1024 × 768 tiles of one texture, 20 % over each other: a mosaic of 3481 × 2610.
    let texture = |x: usize, y: usize| {
        let (fx, fy) = (x as f64, y as f64);
        (20_000.0 + 8_000.0 * ((fx * 0.11).sin() * (fy * 0.07).cos())) as u16
    };
    let step = [819usize, 614];
    let corners: Vec<[f64; 2]> = (0..16)
        .map(|k| {
            [
                (k % 4 * step[0]) as f64 + 0.25,
                (k / 4 * step[1]) as f64 - 0.5,
            ]
        })
        .collect();
    let images: Vec<OwnedImage<u16>> = (0..16)
        .map(|k| {
            let at = [k % 4 * step[0], k / 4 * step[1]];
            OwnedImage::from_fn(1024, 768, |x, y| texture(x + at[0], y + at[1]))
        })
        .collect();
    let tiles = (0..16)
        .map(|k| Placed {
            source: k,
            tile: Tile {
                key: FrameKey {
                    t: 0,
                    c: 0,
                    z: 0,
                    p: k as u32,
                },
                size: [1024, 768],
                nominal: corners[k],
                sensor: [0, 0],
                binning: 1,
            },
            corner: corners[k],
        })
        .collect();
    let bounds = Bounds::of(&corners, &[[1024, 768]; 16]);
    let mut coefficients = vec![0.0; (DEGREE + 1) * (DEGREE + 2) / 2];
    coefficients[0] = 1.0;
    let flat = FlatField {
        sensor: [1024, 768],
        coefficients,
    };
    let folder = tempfile::tempdir().unwrap();
    let info = ImageInfo {
        name: "mosaic".to_owned(),
        sample: SampleType::U16,
        size: Size5 {
            t: 1,
            c: 1,
            z: 1,
            y: bounds.size[1],
            x: bounds.size[0],
        },
        levels: 1,
        chunk: 256,
        pixel_size: [1.0, 0.376, 0.376],
        origin: [0.0; 3],
        channels: vec![ChannelInfo {
            label: "mosaic".to_owned(),
            colour: [255; 3],
            window: (0.0, 65_535.0),
        }],
        cfa: None,
    };
    let image = ZarrImage::create(&folder.path().join("mosaic.ome.zarr"), info).unwrap();
    let plane = Plane {
        at: (0, 0, 0),
        tiles,
    };
    let options = CompositeOptions::default();
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(2)
        .build()
        .unwrap();
    let mosaic = (bounds.size[0] * bounds.size[1] * 2) as isize;
    let bound = composite::bytes(256, &options, 2) as isize;
    let before = LIVE.load(Ordering::Relaxed);
    PEAK.store(before, Ordering::Relaxed);
    pool.install(|| {
        let shading = Some((0.0, &flat));
        composite::composite(
            &images[..],
            &[plane],
            &bounds,
            &image,
            shading,
            &options,
            &(),
        )
    })
    .unwrap();
    let held = PEAK.load(Ordering::Relaxed) - before;
    eprintln!(
        "{:.1} MB held at most, against {:.1} MB estimated and a {:.1} MB mosaic",
        held as f64 / 1e6,
        bound as f64 / 1e6,
        mosaic as f64 / 1e6
    );
    assert!(held <= bound && held < mosaic / 2, "{held} against {bound}");
}
