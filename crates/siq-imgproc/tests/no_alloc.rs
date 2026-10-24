//! The kernels allocate nothing on a rayon pool's threads, as the frame path calls them
//! (REQ-NFR-003). A counting global allocator watches every thread while they run.
//!
//! It is the only test in its binary, so the allocator counts nothing but the kernels and the
//! pool that runs them.

#![allow(unsafe_code)] // a global allocator can only be written with `unsafe`

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use siq_imgproc::*;
use siq_types::{Binning, OwnedImage};

static COUNTING: AtomicBool = AtomicBool::new(false);
static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);

struct Counting;

fn count() {
    if COUNTING.load(Ordering::Relaxed) {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
    }
}

// SAFETY: every call is forwarded to the system allocator unchanged.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        count();
        // SAFETY: forwarded with the caller's guarantees.
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        count();
        // SAFETY: forwarded with the caller's guarantees.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        count();
        // SAFETY: forwarded with the caller's guarantees.
        unsafe { System.realloc(ptr, layout, new_size) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: forwarded with the caller's guarantees.
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

// REQ-NFR-003
#[test]
fn the_kernels_allocate_nothing_on_the_pool() {
    let (width, height) = (1024, 768);
    let raw = OwnedImage::from_fn(width, height, |x, y| ((x * 31 + y * 17) % 16384) as u16);
    let dark = OwnedImage::from_fn(width, height, |x, _| (x % 40) as u16);
    let gain = OwnedImage::from_fn(width, height, |_, y| 0.9 + (y % 3) as f32 * 0.1);
    let float = OwnedImage::from_fn(width, height, |x, y| ((x ^ y) % 256) as f32 / 255.0);
    let (mut corrected, mut copied) = (
        OwnedImage::new(width, height),
        OwnedImage::new(width, height),
    );
    let (mut overview, mut binned) = (
        OwnedImage::<u16>::new(300, 200),
        OwnedImage::new(width / 2, height / 2),
    );
    let (mut scratch, mut blurred) = (
        OwnedImage::new(width, height),
        OwnedImage::new(width, height),
    );
    let mut enlarged = OwnedImage::<f32>::new(1500, 1100);
    let (mut grey, mut rgb) = (
        OwnedImage::new(width, height),
        OwnedImage::new(width, height),
    );
    let mapping = DisplayMapping {
        min: 100,
        max: 12000,
        gamma: 1.5,
        colormap: Colormap::Colour([0, 255, 128]),
    };
    let lut = Lut::new(&mapping).unwrap();
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(4)
        .build()
        .unwrap();

    let mut run = || {
        let calibration = Calibration {
            dark: Some(dark.view()),
            gain: Some(gain.view()),
        };
        correct(&raw.view(), &calibration, 16383, &mut corrected.view_mut()).unwrap();
        correct_in_place(&mut corrected.view_mut(), &calibration, 16383).unwrap();
        downsample_area(&raw.view(), &mut overview.view_mut()).unwrap();
        bin(
            &raw.view(),
            Binning::new(2, 2).unwrap(),
            BinMode::Mean,
            &mut binned.view_mut(),
        )
        .unwrap();
        copy(&raw.view(), &mut copied.view_mut()).unwrap();
        gaussian_blur(
            &float.view(),
            2.0,
            &mut scratch.view_mut(),
            &mut blurred.view_mut(),
        )
        .unwrap();
        upsample_bilinear(&float.view(), &mut enlarged.view_mut()).unwrap();
        let mut counts = Histogram::new(14);
        histogram(&raw.view(), 4, &mut counts).unwrap();
        let window = counts.auto_window(0.01, 0.99);
        lut.apply(&raw.view(), &mut grey.view_mut()).unwrap();
        lut.apply_rgb(&raw.view(), &mut rgb.view_mut()).unwrap();
        let sharpness = focus(&raw.view(), FocusMeasure::Tenengrad)
            + focus(&raw.view(), FocusMeasure::LaplacianVariance);
        std::hint::black_box((
            window,
            sharpness,
            focus(&raw.view(), FocusMeasure::NormalizedVariance),
        ));
    };
    pool.install(|| {
        run(); // warm-up: the pool's queues reach their size

        // The counter must see an allocation, or a zero below would prove nothing.
        COUNTING.store(true, Ordering::Relaxed);
        drop(std::hint::black_box(vec![1u8]));
        COUNTING.store(false, Ordering::Relaxed);
        assert_eq!(
            ALLOCATIONS.swap(0, Ordering::Relaxed),
            1,
            "the counter works"
        );

        COUNTING.store(true, Ordering::Relaxed);
        for _ in 0..20 {
            run();
        }
        COUNTING.store(false, Ordering::Relaxed);
    });
    assert_eq!(ALLOCATIONS.load(Ordering::Relaxed), 0);
}
