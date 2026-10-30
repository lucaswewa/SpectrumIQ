//! A pyramid's build holds a few chunks per thread, not the image (ADR-0044): a counting global
//! allocator measures the peak of live heap bytes while a 4096 × 4096 image's levels are built on
//! two threads. REQ-STI-003.
//!
//! It is the only test in its binary, so the allocator counts nothing but the build.

#![allow(unsafe_code)] // a global allocator can only be written with `unsafe`

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicIsize, Ordering};

use siq_pyramid::build;
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
fn building_holds_a_few_chunks_per_thread_not_the_image() {
    let folder = tempfile::tempdir().unwrap();
    let size = Size5 {
        t: 1,
        c: 1,
        z: 1,
        y: 4096,
        x: 4096,
    };
    let info = ImageInfo {
        name: "memory".to_owned(),
        sample: SampleType::U16,
        size,
        levels: 1,
        chunk: 512,
        pixel_size: [1.0; 3],
        origin: [0.0; 3],
        channels: vec![ChannelInfo {
            label: "c".to_owned(),
            colour: [255; 3],
            window: (0.0, 1.0),
        }],
        cfa: None,
    };
    let mut image = ZarrImage::create(&folder.path().join("big.ome.zarr"), info).unwrap();
    for (row, col) in (0..8).flat_map(|row| (0..8).map(move |col| (row, col))) {
        let chunk = OwnedImage::from_fn(512, 512, |x, y| (x * 7 + y * 13 + row as usize) as u16);
        image
            .write_chunk(0, (0, 0, 0), (row, col), chunk.view())
            .unwrap();
    }
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(2)
        .build()
        .unwrap();

    let before = LIVE.load(Ordering::Relaxed);
    PEAK.store(before, Ordering::Relaxed);
    pool.install(|| build(&mut image, 256, &())).unwrap();
    let peak = PEAK.load(Ordering::Relaxed) - before;

    // The image is 32 MB; a chunk row of it is 4 MB. Two threads each hold 2.5 MB of pixels.
    let chunk_row = 4096 * 512 * 2;
    println!("peak: {:.1} MB above the start", peak as f64 / 1e6);
    assert!(peak < 3 * chunk_row, "{peak} bytes at the peak");
    assert_eq!(image.info().levels, 5);
}
