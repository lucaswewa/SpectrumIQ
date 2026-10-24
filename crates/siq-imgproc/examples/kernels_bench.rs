//! Times the kernels on a 100 MP frame, on a pool of half the cores as the frame path has
//! (architecture §4.3), against the per-frame budget of architecture §4.2.
//!
//! ```text
//! cargo run --release -p siq-imgproc --example kernels_bench
//! ```
//!
//! Smoke mode (`SIQ_SMOKE=1`) uses a 1 MP frame. Each line is the best of 5 runs, and GB/s is
//! what the kernel reads.

use std::time::{Duration, Instant};

use siq_imgproc::*;
use siq_types::{Binning, OwnedImage, RectPx};

fn main() {
    let smoke = std::env::var_os("SIQ_SMOKE").is_some();
    let (width, height) = if smoke { (1155, 866) } else { (11_648, 8_742) };
    let threads = (std::thread::available_parallelism().map_or(2, |n| n.get()) / 2).max(1);
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build()
        .expect("a pool");
    println!(
        "{width} × {height} ({:.0} MP), {threads} threads",
        (width * height) as f64 / 1e6
    );

    let raw = OwnedImage::from_fn(width, height, |x, y| {
        ((x * 31 + y * 17 + (x * y) % 251) % 16384) as u16
    });
    let dark = OwnedImage::from_fn(width, height, |x, _| (100 + x % 40) as u16);
    let gain = OwnedImage::from_fn(width, height, |x, y| 0.9 + ((x + y) % 5) as f32 * 0.05);
    let mut corrected = OwnedImage::new(width, height);
    // The overview fits 1920 × 1080 with the frame's aspect; the ROI is 1920 × 1080 at 1:1.
    let fit = (1920.0 / width as f64).min(1080.0 / height as f64).min(1.0);
    let (ow, oh) = (
        (width as f64 * fit) as usize,
        (height as f64 * fit) as usize,
    );
    let (mut overview, mut overview_grey) =
        (OwnedImage::<u16>::new(ow, oh), OwnedImage::new(ow, oh));
    let (rw, rh) = (1920.min(width), 1080.min(height));
    let roi = RectPx::new(
        ((width - rw) / 2) as u32,
        ((height - rh) / 2) as u32,
        rw as u32,
        rh as u32,
    );
    let (mut roi_copy, mut roi_rgb) = (OwnedImage::<u16>::new(rw, rh), OwnedImage::new(rw, rh));
    let mut binned = OwnedImage::new(width / 2, height / 2);
    let float = OwnedImage::from_fn(width / 2, height / 2, |x, y| ((x ^ y) % 256) as f32 / 255.0);
    let (mut scratch, mut blurred) = (
        OwnedImage::new(width / 2, height / 2),
        OwnedImage::new(width / 2, height / 2),
    );
    let quarter = OwnedImage::from_fn(width / 4, height / 4, |x, y| ((x + y) % 256) as f32 / 255.0);
    let mut enlarged = OwnedImage::new(width / 2, height / 2);
    let mapping = DisplayMapping {
        min: 200,
        max: 15000,
        gamma: 1.0,
        colormap: Colormap::Colour([0, 255, 0]),
    };
    let lut = Lut::new(&mapping).expect("a valid mapping");
    let calibration = Calibration {
        dark: Some(dark.view()),
        gain: Some(gain.view()),
    };
    let frame_mb = (width * height * 2) as f64 / 1e6;

    let best = |work: &mut (dyn FnMut() + Send)| {
        pool.install(|| (0..5).map(|_| timed(&mut *work)).min().expect("five runs"))
    };
    let report = |name: &str, time: Duration, read_mb: f64, budget: &str| {
        let ms = time.as_secs_f64() * 1e3;
        println!(
            "{name:<34} {ms:>8.2} ms {:>7.1} GB/s   {budget}",
            read_mb / ms
        );
    };

    let t =
        best(&mut || correct(&raw.view(), &calibration, 16383, &mut corrected.view_mut()).unwrap());
    report("flat field (dark + gain)", t, frame_mb * 4.0, "");
    let t = best(&mut || downsample_area(&corrected.view(), &mut overview.view_mut()).unwrap());
    let lut_t = best(&mut || {
        lut.apply(&overview.view(), &mut overview_grey.view_mut())
            .unwrap()
    });
    report(
        &format!("overview: area average to {ow}×{oh}"),
        t,
        frame_mb,
        "",
    );
    report("overview: LUT", lut_t, (ow * oh * 2) as f64 / 1e6, "");
    report(
        "overview total",
        t + lut_t,
        frame_mb,
        "budget ≤ 35 ms with JPEG",
    );
    let crop = corrected.view().sub_view(roi).expect("the ROI is inside");
    let t = best(&mut || copy(&crop, &mut roi_copy.view_mut()).unwrap());
    let rgb_t = best(&mut || {
        lut.apply_rgb(&roi_copy.view(), &mut roi_rgb.view_mut())
            .unwrap()
    });
    report(
        &format!("ROI: crop {rw}×{rh} + colour LUT"),
        t + rgb_t,
        (rw * rh * 2) as f64 / 1e6,
        "budget ≤ 15 ms with JPEG",
    );
    let mut counts = Histogram::new(14);
    let t = best(&mut || histogram(&corrected.view(), 4, &mut counts).unwrap());
    let focus_t = best(&mut || {
        std::hint::black_box(focus(&roi_copy.view(), FocusMeasure::Tenengrad));
    });
    report("stats: histogram, step 4", t, frame_mb / 16.0, "");
    report(
        "stats: Tenengrad on the ROI",
        focus_t,
        (rw * rh * 2) as f64 / 1e6,
        "",
    );
    report(
        "stats total",
        t + focus_t,
        frame_mb / 16.0,
        "budget ≤ 10 ms",
    );
    let t = best(&mut || {
        bin(
            &corrected.view(),
            Binning::new(2, 2).unwrap(),
            BinMode::Mean,
            &mut binned.view_mut(),
        )
        .unwrap()
    });
    report(
        "live stitch: 2 × 2 bin",
        t,
        frame_mb,
        "with flat field: budget ≤ 60 ms",
    );
    let t = best(&mut || {
        gaussian_blur(
            &float.view(),
            2.0,
            &mut scratch.view_mut(),
            &mut blurred.view_mut(),
        )
        .unwrap()
    });
    report(
        &format!("blur σ 2, f32 {}×{}", width / 2, height / 2),
        t,
        (width * height) as f64 / 1e6 * 2.0,
        "simulator",
    );
    let t = best(&mut || upsample_bilinear(&quarter.view(), &mut enlarged.view_mut()).unwrap());
    report(
        "bilinear 2× to the binned size",
        t,
        (width * height) as f64 / 4e6,
        "simulator",
    );
    println!(
        "window from the histogram: {:?}",
        counts.auto_window(0.001, 0.999)
    );
}

fn timed(work: &mut (dyn FnMut() + Send)) -> Duration {
    let started = Instant::now();
    work();
    started.elapsed()
}
