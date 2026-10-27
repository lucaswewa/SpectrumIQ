//! The live canvas (ADR-0059) on this machine's GPU, each GPU test skipped with its reason where
//! there is none: the paint against its CPU reference, a spot placed where a known affine puts it,
//! a frame across four pages as one, pages evicted to host RAM and brought back, and clearing; and
//! the geometry, the policy and the WGSL, which need no GPU. REQ-STI-001, REQ-STI-003.

use siq_gpu::{Gpu, GpuError, GpuHandle, GpuOptions};
use siq_stitch_live::paint::{self, Frame, Params};
use siq_stitch_live::{Canvas, CanvasGeometry, CanvasOptions, PAGE, frame_to_canvas, paints};
use siq_types::{Affine2, AffineParams, Micrometers, PoseQuality};

fn gpu() -> Option<GpuHandle> {
    match GpuHandle::open(&GpuOptions::default()) {
        Ok(handle) => Some(handle),
        Err(error) => {
            eprintln!("skipped: {error}");
            None
        }
    }
}

/// Texture that varies smoothly and in every bit, the same each run.
fn texture(width: u32, height: u32, seed: u32) -> Vec<u16> {
    (0..height)
        .flat_map(|y| {
            (0..width).map(move |x| {
                let (fx, fy) = (f64::from(x), f64::from(y));
                let wave = (fx * 0.07 + f64::from(seed)).sin() * (fy * 0.05).cos();
                (20_000.0 + 15_000.0 * wave) as u16 ^ ((x * 7 + y * 13 + seed) % 16) as u16
            })
        })
        .collect()
}

/// Half a canvas pixel a frame pixel, turned 0.4°, then moved to `at`.
fn affine(at: [f64; 2]) -> Affine2 {
    let turned = Affine2::from_params(&AffineParams {
        scale_x: 0.5,
        scale_y: 0.5,
        rotation: 0.4_f64.to_radians(),
        shear: 0.0,
        translation: [0.0, 0.0],
    });
    Affine2::translation(at).compose(&turned)
}

/// The two halves of a canvas word: value and coverage.
fn halves(word: u32) -> (i32, i32) {
    ((word & 0xffff) as i32, (word >> 16) as i32)
}

#[test]
fn the_wgsl_validates() {
    for shader in siq_stitch_live::SHADERS {
        let module = naga::front::wgsl::parse_str(shader.source).unwrap_or_else(|error| {
            panic!("{}: {}", shader.name, error.emit_to_string(shader.source))
        });
        let flags = naga::valid::ValidationFlags::all();
        let mut validator = naga::valid::Validator::new(flags, naga::valid::Capabilities::all());
        validator.validate(&module).unwrap();
    }
}

#[test]
fn a_frame_lands_where_its_calibration_and_pose_put_it() {
    // 2.66 sensor pixels a µm, turned 0.4°: the simulator's mount.
    let l = Affine2::from_params(&AffineParams {
        scale_x: 10.0 / 3.76,
        scale_y: 10.0 / 3.76,
        rotation: 0.4_f64.to_radians(),
        shear: 0.0,
        translation: [0.0, 0.0],
    });
    let geometry = CanvasGeometry::for_calibration(&l, 0.5);
    assert!((geometry.px_per_um - 5.0 / 3.76).abs() < 1e-12);
    let pose = [-20_000.0, -15_000.0];
    let m = frame_to_canvas(&l, [512, 384], 2, pose, &geometry).unwrap();
    // Its centre where the stage is.
    let centre = m.apply([256.0, 192.0]);
    let expected = geometry.to_canvas(pose);
    assert!((centre[0] - expected[0]).abs() < 1e-9 && (centre[1] - expected[1]).abs() < 1e-9);
    // A corner a binned frame's half away, through L's inverse.
    let corner = m.apply([0.0, 0.0]);
    let offset = l.invert().unwrap().apply_vector([-512.0, -384.0]);
    let expected = geometry.to_canvas([pose[0] + offset[0], pose[1] + offset[1]]);
    assert!((corner[0] - expected[0]).abs() < 1e-9 && (corner[1] - expected[1]).abs() < 1e-9);
    // Back to the stage.
    let back = geometry.to_stage(geometry.to_canvas(pose));
    assert!((back[0] - pose[0]).abs() < 1e-9);
}

#[test]
fn only_frames_whose_pose_is_known_paint() {
    let threshold = Micrometers(2.0);
    assert!(paints(&PoseQuality::Settled, threshold));
    let moving = |um: f64| PoseQuality::Interpolated {
        uncertainty: Micrometers(um),
    };
    assert!(paints(&moving(1.5), threshold));
    assert!(!paints(&moving(2.5), threshold));
    assert!(!paints(&PoseQuality::Unknown, threshold));
}

/// `frames` painted by the CPU reference on one buffer `side` pixels a side, its corner at
/// canvas pixel `origin`.
fn reference(
    frames: &[(Vec<u16>, [u32; 2], Affine2)],
    origin: [i64; 2],
    side: u32,
    feather: f32,
) -> Vec<u32> {
    let mut canvas = vec![0u32; (side * side) as usize];
    for (samples, [width, height], m) in frames {
        let frame = Frame {
            samples,
            width: *width,
            height: *height,
        };
        if let Some(params) = Params::for_page(&frame, m, origin, side, feather) {
            paint::cpu(&mut canvas, samples, &params);
        }
    }
    canvas
}

/// `frames` painted on a canvas on `gpu`, and the rectangle from `origin`, `side` pixels a side.
fn painted(
    gpu: &mut Gpu,
    options: CanvasOptions,
    frames: &[(Vec<u16>, [u32; 2], Affine2)],
    origin: [i64; 2],
    side: u32,
) -> Result<Vec<u32>, GpuError> {
    let mut canvas = Canvas::new(options);
    for (samples, [width, height], m) in frames {
        let frame = Frame {
            samples,
            width: *width,
            height: *height,
        };
        canvas.paint(gpu, frame, m)?;
    }
    canvas.region(gpu, origin, [side, side])
}

/// The largest difference between two canvases' values and coverages, where either is painted.
fn worst(a: &[u32], b: &[u32]) -> (i32, i32) {
    a.iter().zip(b).fold((0, 0), |(v, c), (&x, &y)| {
        let ((xv, xc), (yv, yc)) = (halves(x), halves(y));
        (v.max((xv - yv).abs()), c.max((xc - yc).abs()))
    })
}

#[test]
fn the_gpu_paints_as_its_cpu_reference_does() {
    let Some(handle) = gpu() else { return };
    // Two frames overlapping, the second over the first: feathered where it covers it.
    let frames = vec![
        (texture(300, 200, 1), [300, 200], affine([100.0, 120.0])),
        (texture(300, 200, 2), [300, 200], affine([180.0, 150.0])),
    ];
    let expected = reference(&frames, [0, 0], 512, 32.0);
    let options = CanvasOptions {
        feather_px: 32.0,
        ..CanvasOptions::default()
    };
    let got = handle
        .run_blocking(move |gpu| painted(gpu, options, &frames, [0, 0], 512))
        .unwrap();
    let painted_pixels = got.iter().filter(|&&word| word >> 16 > 0).count();
    // Two frames of 150 × 100 canvas pixels, overlapping.
    assert!(painted_pixels > 20_000, "{painted_pixels}");
    let (value, cover) = worst(&got, &expected);
    eprintln!("the GPU's paint {value} and {cover} from the CPU's in value and coverage");
    assert!(
        value <= 1 && cover <= 1,
        "values {value} and coverages {cover} apart"
    );
}

#[test]
fn a_spot_lands_where_a_known_affine_puts_it() {
    let Some(handle) = gpu() else { return };
    // A spot on a dark frame, a Gaussian of 3 pixels.
    let (width, height, spot) = (256u32, 192u32, [100.3, 70.7]);
    let samples: Vec<u16> = (0..height)
        .flat_map(|y| {
            (0..width).map(move |x| {
                let d2 =
                    (f64::from(x) + 0.5 - spot[0]).powi(2) + (f64::from(y) + 0.5 - spot[1]).powi(2);
                (1000.0 + 40_000.0 * (-d2 / 18.0).exp()) as u16
            })
        })
        .collect();
    let m = affine([1234.5, 567.25]);
    let expected = m.apply(spot);
    let origin = [expected[0] as i64 - 16, expected[1] as i64 - 16];
    let frames = vec![(samples, [width, height], m)];
    let region = handle
        .run_blocking(move |gpu| painted(gpu, CanvasOptions::default(), &frames, origin, 32))
        .unwrap();
    // The centroid of what is above the background.
    let (mut sum, mut sx, mut sy) = (0.0, 0.0, 0.0);
    for (i, &word) in region.iter().enumerate() {
        let excess = (f64::from(word & 0xffff) - 1000.0).max(0.0);
        let (x, y) = ((i % 32) as f64 + 0.5, (i / 32) as f64 + 0.5);
        (sum, sx, sy) = (sum + excess, sx + excess * x, sy + excess * y);
    }
    let found = [origin[0] as f64 + sx / sum, origin[1] as f64 + sy / sum];
    let off = (found[0] - expected[0]).hypot(found[1] - expected[1]);
    eprintln!("the spot {off:.4} canvas pixels from where the affine puts it");
    assert!(
        off < 0.05,
        "{off} canvas pixels off: {found:?} against {expected:?}"
    );
}

#[test]
fn a_frame_across_four_pages_is_painted_as_one() {
    let Some(handle) = gpu() else { return };
    // A frame over the corner where four pages meet.
    let corner = f64::from(PAGE);
    let frames = vec![(
        texture(400, 300, 3),
        [400, 300],
        affine([corner - 80.0, corner - 60.0]),
    )];
    let origin = [i64::from(PAGE) - 128, i64::from(PAGE) - 128];
    let expected = reference(&frames, origin, 256, 64.0);
    let got = handle
        .run_blocking(move |gpu| {
            let mut canvas = Canvas::new(CanvasOptions::default());
            let (samples, [width, height], m) = &frames[0];
            let frame = Frame {
                samples,
                width: *width,
                height: *height,
            };
            let done = canvas.paint(gpu, frame, m)?;
            Ok((done, canvas.region(gpu, origin, [256, 256])?))
        })
        .unwrap();
    assert_eq!((got.0.pages, got.0.made), (4, 4));
    // Near a page's far corner the frame's coordinates are differences of numbers in the
    // thousands, which the GPU may round apart from the CPU by a thousandth of a pixel: through the
    // feather, 37.5 pixels here, up to 2 in coverage.
    let (value, cover) = worst(&got.1, &expected);
    assert!(
        value <= 1 && cover <= 2,
        "values {value} and coverages {cover} apart"
    );
}

#[test]
fn pages_past_the_budget_go_to_host_ram_and_come_back() {
    let Some(handle) = gpu() else { return };
    // Room for two pages; three far apart, then the first again.
    let options = CanvasOptions {
        budget_bytes: 2 * u64::from(PAGE) * u64::from(PAGE) * 4,
        ..CanvasOptions::default()
    };
    let flat = vec![1000u16; 200 * 150];
    let page = f64::from(PAGE);
    let places = [
        [100.0, 100.0],
        [page + 100.0, 100.0],
        [100.0, page + 100.0],
        [150.0, 120.0],
    ];
    let got = handle
        .run_blocking(move |gpu| {
            let mut canvas = Canvas::new(options);
            let mut done = Vec::new();
            let mut counts = Vec::new();
            let mut seen = Vec::new();
            for (i, at) in places.iter().enumerate() {
                let frame = Frame {
                    samples: &flat,
                    width: 200,
                    height: 150,
                };
                done.push(canvas.paint(gpu, frame, &Affine2::translation(*at))?);
                counts.push((canvas.on_gpu(), canvas.on_host()));
                if i == 2 {
                    // The first page, evicted: inside the first frame's feather, from host RAM.
                    seen = canvas.region(gpu, [192, 172], [8, 8])?;
                }
            }
            canvas.clear();
            counts.push((canvas.on_gpu(), canvas.on_host()));
            Ok((done, counts, seen, canvas.bounds()))
        })
        .unwrap();
    let (done, counts, seen, bounds) = got;
    assert_eq!(counts, [(1, 0), (2, 0), (2, 1), (2, 1), (0, 0)]);
    assert_eq!(done[2].evicted, 1);
    assert_eq!((done[3].restored, done[3].evicted), (1, 1));
    // A flat frame shrinks to itself, wholly covered.
    assert!(
        seen.iter().all(|&word| halves(word) == (1000, 65_535)),
        "{seen:?}"
    );
    assert_eq!(bounds, None);
}
