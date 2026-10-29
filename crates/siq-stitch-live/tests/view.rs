//! The viewport and the frames' preparation (ADR-0059): a viewport's place on the canvas and its
//! checks, a frame binned to the canvas's scale and flat-corrected, which need no GPU; and on this
//! machine's GPU, each test skipped with its reason where there is none, the view rendered as the
//! canvas's pixels through the LUT say, a spot at the centre of a view about it, and a view across
//! pages, an evicted one at its shrunk detail. REQ-STI-001, REQ-PRV-001.

use siq_calib::shading::{DEGREE, FlatField};
use siq_gpu::lut::Table;
use siq_gpu::{GpuHandle, GpuOptions};
use siq_stitch_live::paint::Frame;
use siq_stitch_live::view::Params;
use siq_stitch_live::{Canvas, CanvasGeometry, CanvasOptions, PAGE, Preparer, View, Viewport};
use siq_types::{
    Affine2, Binning, FrameFlags, FrameMeta, FrameSeq, ImageView, Millis, PixelFormat, PoseQuality,
    Roi, Timestamp,
};

fn gpu() -> Option<GpuHandle> {
    match GpuHandle::open(&GpuOptions::default()) {
        Ok(handle) => Some(handle),
        Err(error) => {
            eprintln!("skipped: {error}");
            None
        }
    }
}

/// A window from 0 to 40 000, linear.
fn table() -> Table {
    std::array::from_fn(|value| (value as f64 / 40_000.0 * 255.0).min(255.0).round() as u8)
}

/// The level a canvas word shows at through `table`, as the kernel computes it.
fn shown(word: u32, table: &Table) -> u8 {
    let level = f32::from(table[(word & 0xffff) as usize]) * (word >> 16) as f32 / 65_535.0;
    level.round_ties_even() as u8
}

#[test]
fn a_viewport_is_a_rectangle_of_the_canvas() {
    let geometry = CanvasGeometry {
        origin: [-1000.0, 500.0],
        px_per_um: 2.0,
    };
    let viewport = Viewport {
        centre: [100.0, 550.0],
        zoom: 0.5,
        width: 400,
        height: 300,
    };
    assert_eq!(viewport.check(), Ok(()));
    let view = viewport.on_canvas(&geometry);
    // The centre at canvas (2200, 100), two canvas pixels a view pixel.
    assert_eq!(view.step, 2.0);
    assert_eq!(view.origin, [2200.0 - 400.0, 100.0 - 300.0]);
    assert_eq!(view.to_canvas([200, 150]), [2201.0, 101.0]);
    assert_eq!(view.stride(), 100);
    let back = geometry.to_stage(view.to_canvas([0, 0]));
    assert_eq!(back, [100.0 - 199.5, 550.0 - 149.5]);
    // The pages it sees, and those it doesn't.
    assert!(Params::for_page(&view, [0, 0], PAGE, 1).is_some());
    assert!(Params::for_page(&view, [i64::from(PAGE), 0], PAGE, 1).is_none());
    assert!(Params::for_page(&view, [0, -i64::from(PAGE)], PAGE, 1).is_some());
    assert!(Params::for_page(&view, [0, -2 * i64::from(PAGE)], PAGE, 1).is_none());

    for wrong in [
        Viewport {
            width: 0,
            ..viewport
        },
        Viewport {
            width: 1921,
            ..viewport
        },
        Viewport {
            height: 1081,
            ..viewport
        },
        Viewport {
            zoom: 0.0,
            ..viewport
        },
        Viewport {
            zoom: 65.0,
            ..viewport
        },
        Viewport {
            centre: [f64::NAN, 0.0],
            ..viewport
        },
    ] {
        assert!(wrong.check().is_err(), "{wrong:?}");
    }
    assert_eq!(Viewport::default().check(), Ok(()));
}

fn meta(width: u32, height: u32, binning: u8) -> FrameMeta {
    FrameMeta {
        seq: FrameSeq(0),
        device_ts: None,
        host_ts_mid_exposure: Timestamp(0),
        exposure: Millis(10.0),
        gain: 1.0,
        roi: Roi::full(width, height),
        binning: Binning::new(binning, binning).unwrap(),
        pixel_format: PixelFormat::Mono14,
        cfa: None,
        pose: None,
        pose_quality: PoseQuality::Settled,
        channel: None,
        objective: None,
        calibration: None,
        flags: FrameFlags::default(),
    }
}

#[test]
fn a_frame_is_binned_to_the_canvas_scale_and_flat_corrected() {
    // A 256 × 192 sensor, its light from a quarter of the mean on the left to three quarters on
    // the right, and a frame of 1100 everywhere, over a dark of 100.
    let mut coefficients = vec![0.0; (DEGREE + 1) * (DEGREE + 2) / 2];
    (coefficients[0], coefficients[1]) = (0.5, 0.25);
    let flat = FlatField {
        sensor: [256, 192],
        coefficients,
    };
    let samples = vec![1100u16; 256 * 192];
    let image = ImageView::new(&samples, 256, 192, 256).unwrap();
    let mut preparer = Preparer::default();
    let plain = preparer
        .prepare(&image, &meta(256, 192, 1), 0.5, None)
        .unwrap();
    assert_eq!((plain.width, plain.height, plain.binning), (128, 96, 2));
    assert!(plain.samples.iter().all(|&value| value == 1100));
    let corrected = preparer
        .prepare(&image, &meta(256, 192, 1), 0.5, Some((100.0, &flat)))
        .unwrap();
    for column in [0, 40, 127] {
        let x = (column as f64 + 0.5) * 2.0 * 2.0 / 256.0 - 1.0;
        let expected = (1000.0 / (0.5 + 0.25 * x)).round() as i32;
        let got = i32::from(corrected.samples[50 * 128 + column]);
        assert!(
            (got - expected).abs() <= 1,
            "{column}: {got} for {expected}"
        );
    }
    // Binned 2 × 2 by the camera already: a quarter needs only 2 more.
    let quarter = preparer
        .prepare(&image, &meta(512, 384, 2), 0.25, None)
        .unwrap();
    assert_eq!((quarter.width, quarter.binning), (128, 4));
}

/// Texture that varies smoothly, the same each run.
fn texture(width: u32, height: u32, seed: u32) -> Vec<u16> {
    (0..height)
        .flat_map(|y| {
            (0..width).map(move |x| {
                let wave =
                    (f64::from(x) * 0.05 + f64::from(seed)).sin() * (f64::from(y) * 0.04).cos();
                (20_000.0 + 15_000.0 * wave) as u16
            })
        })
        .collect()
}

#[test]
fn the_view_shows_the_canvas_through_the_lut() {
    let Some(handle) = gpu() else { return };
    // Two frames across a page's edge, seen at half their detail from a corner between pixels.
    let edge = f64::from(PAGE);
    let frames = [
        (texture(300, 200, 1), [edge - 200.0, 50.0]),
        (texture(300, 200, 2), [edge - 20.0, 120.0]),
    ];
    let view = View {
        origin: [edge - 300.3, 10.7],
        step: 2.0,
        size: [301, 190],
    };
    let table = table();
    let (pixels, region) = handle
        .run_blocking(move |gpu| {
            let mut canvas = Canvas::new(CanvasOptions::default());
            for (samples, at) in &frames {
                let frame = Frame {
                    samples,
                    width: 300,
                    height: 200,
                };
                canvas.paint(gpu, frame, &Affine2::translation(*at))?;
            }
            let pixels = canvas.render(gpu, &view, &table)?;
            let corner = [(edge - 301.0) as i64, 10];
            Ok((pixels, (corner, canvas.region(gpu, corner, [604, 384])?)))
        })
        .unwrap();
    assert_eq!(pixels.len(), 301 * 190);
    let ((cx, cy), words) = ((region.0[0], region.0[1]), region.1);
    let (mut wrong, mut lit) = (0, 0);
    for y in 0..190 {
        for x in 0..301 {
            let [px, py] = view.to_canvas([x, y]).map(f64::floor);
            let word = words[((py as i64 - cy) * 604 + (px as i64 - cx)) as usize];
            let (got, expected) = (pixels[(y * 301 + x) as usize], shown(word, &table));
            wrong += usize::from(got.abs_diff(expected) > 1);
            lit += usize::from(got > 0);
        }
    }
    eprintln!("{lit} pixels lit, {wrong} not as the canvas says");
    assert!(lit > 20_000 && wrong == 0, "{lit} lit, {wrong} wrong");
}

#[test]
fn a_spot_shows_at_the_centre_of_a_view_about_it() {
    let Some(handle) = gpu() else { return };
    // A bright spot on a dark frame, at stage point (250.5, 120.5) µm, two canvas pixels a µm.
    let geometry = CanvasGeometry {
        origin: [0.0, 0.0],
        px_per_um: 2.0,
    };
    let mut samples = vec![1000u16; 200 * 200];
    for y in 97..103 {
        for x in 97..103 {
            samples[y * 200 + x] = 40_000;
        }
    }
    // The frame's pixel (100, 100) at canvas (501, 241).
    let m = Affine2::translation([401.0, 141.0]);
    let viewport = Viewport {
        centre: [250.5, 120.5],
        zoom: 1.0,
        width: 64,
        height: 48,
    };
    let view = viewport.on_canvas(&geometry);
    let table = table();
    let pixels = handle
        .run_blocking(move |gpu| {
            let mut canvas = Canvas::new(CanvasOptions::default());
            let frame = Frame {
                samples: &samples,
                width: 200,
                height: 200,
            };
            canvas.paint(gpu, frame, &m)?;
            canvas.render(gpu, &view, &table)
        })
        .unwrap();
    // The bright block's centre at the view's.
    let (mut sum, mut sx, mut sy) = (0.0, 0.0, 0.0);
    for (i, &level) in pixels.iter().enumerate() {
        let excess = f64::from(level.saturating_sub(10));
        let (x, y) = ((i % 64) as f64 + 0.5, (i / 64) as f64 + 0.5);
        (sum, sx, sy) = (sum + excess, sx + excess * x, sy + excess * y);
    }
    let found = [sx / sum, sy / sum];
    assert!(
        (found[0] - 32.0).abs() < 0.01 && (found[1] - 24.0).abs() < 0.01,
        "{found:?}"
    );
}

#[test]
fn a_view_across_pages_shows_an_evicted_one_shrunk() {
    let Some(handle) = gpu() else { return };
    // Room for two pages; frames on three, so the first is in host RAM.
    let options = CanvasOptions {
        budget_bytes: 2 * u64::from(PAGE) * u64::from(PAGE) * 4,
        ..CanvasOptions::default()
    };
    let flat = vec![20_000u16; 400 * 300];
    let page = f64::from(PAGE);
    let places = [[100.0, 100.0], [page + 100.0, 100.0], [100.0, page + 100.0]];
    // The two pages side by side, an eighth of their detail.
    let view = View {
        origin: [0.0, 0.0],
        step: 8.0,
        size: [1024, 512],
    };
    let table = table();
    let (pixels, on_host) = handle
        .run_blocking(move |gpu| {
            let mut canvas = Canvas::new(options);
            for at in places {
                let frame = Frame {
                    samples: &flat,
                    width: 400,
                    height: 300,
                };
                canvas.paint(gpu, frame, &Affine2::translation(at))?;
            }
            Ok((canvas.render(gpu, &view, &table)?, canvas.on_host()))
        })
        .unwrap();
    assert_eq!(on_host, 1);
    let level = |x: u32, y: u32| pixels[(y * 1024 + x) as usize];
    let expected = table[20_000];
    // Each frame's middle, the first from host RAM; and nothing between them.
    assert_eq!(level(37, 31), expected);
    assert_eq!(level(512 + 37, 31), expected);
    assert_eq!(level(300, 31), 0);
    assert_eq!(level(37, 400), 0);
}
