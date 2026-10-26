//! Phase correlation (ADR-0048) on `siq-synth` patches with known shifts: sub-pixel accuracy with
//! noise, large shifts coarse to fine, low overlaps, low confidence on periodic and unrelated
//! images, the inputs refused, and the plans kept. REQ-STI-002, REQ-CAL-001.

use siq_register::*;
use siq_synth::{Cells, CellsParams, Grains, GrainsParams, World, grid, sample};
use siq_types::{Micrometers, OwnedImage, Vec2Um};

/// The pixel size the patches are sampled at, in µm.
const PIXEL: f64 = 0.5;

fn cells(seed: u64) -> Cells {
    Cells::new(CellsParams {
        seed,
        ..CellsParams::default()
    })
}

/// A noise value for pixel `i` of `seed`, uniform in ±`amplitude`.
fn noise(seed: u64, i: usize, amplitude: f32) -> f32 {
    let mut z = seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ i as u64;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    let unit = ((z ^ (z >> 31)) >> 40) as f32 / (1u64 << 24) as f32;
    (unit * 2.0 - 1.0) * amplitude
}

/// `world` sampled over `size` pixels from `origin` (in pixels), with noise of `seed`.
fn patch(world: &dyn World, origin: [f64; 2], size: (usize, usize), seed: u64) -> OwnedImage<f32> {
    let mut image = OwnedImage::new(size.0, size.1);
    let at = Vec2Um::from_um(origin[0] * PIXEL, origin[1] * PIXEL);
    sample(world, &grid(at, Micrometers(PIXEL)), &mut image.view_mut());
    let mut i = 0;
    for row in image.view_mut().rows_mut() {
        for value in row {
            *value += noise(seed, i, 0.03);
            i += 1;
        }
    }
    image
}

/// Two patches of `world` whose content is `shift` apart: `b` shows at `x` what `a` shows at
/// `x − shift`.
fn pair(
    world: &dyn World,
    shift: [f64; 2],
    size: (usize, usize),
) -> (OwnedImage<f32>, OwnedImage<f32>) {
    let a = patch(world, [100.0, 60.0], size, 1);
    let b = patch(world, [100.0 - shift[0], 60.0 - shift[1]], size, 2);
    (a, b)
}

fn error(found: [f64; 2], shift: [f64; 2]) -> f64 {
    (found[0] - shift[0]).hypot(found[1] - shift[1])
}

/// Known shifts, whole and fractional, up to 40 pixels each way.
fn shifts() -> Vec<[f64; 2]> {
    (0..24)
        .map(|k| {
            let t = f64::from(k);
            [
                ((t * 7.31) % 80.0) - 40.0 + 0.137 * t % 1.0,
                ((t * 11.7) % 80.0) - 40.0 - 0.291 * t % 1.0,
            ]
        })
        .collect()
}

#[test]
fn known_shifts_are_found_within_a_tenth_of_a_pixel() {
    let world = cells(3);
    let registrar = Registrar::default();
    let mut squares = 0.0;
    for shift in shifts() {
        let (a, b) = pair(&world, shift, (256, 256));
        let found = registrar.register(a.view(), b.view()).unwrap();
        let e = error(found.shift, shift);
        assert!(e < 0.1, "{shift:?}: found {:?}, {e:.3} px off", found.shift);
        assert!(found.ncc > 0.8 && found.margin > 0.3, "{found:?}");
        squares += e * e;
    }
    let rms = (squares / shifts().len() as f64).sqrt();
    assert!(rms < 0.05, "rms {rms:.3} px");
    // The plans are kept: the same pairs again plan nothing new.
    let planned = registrar.planned();
    for shift in shifts() {
        let (a, b) = pair(&world, shift, (256, 256));
        registrar.register(a.view(), b.view()).unwrap();
    }
    assert_eq!(registrar.planned(), planned);
}

#[test]
fn large_shifts_are_found_coarse_to_fine() {
    let world = cells(5);
    let registrar = Registrar::default();
    for shift in [[300.3, -211.7], [-450.6, 120.2], [12.4, 380.9]] {
        let (a, b) = pair(&world, shift, (1024, 768));
        let found = registrar.register(a.view(), b.view()).unwrap();
        let e = error(found.shift, shift);
        assert!(e < 0.1, "{shift:?}: found {:?}, {e:.3} px off", found.shift);
    }
    // The coarse size, and the fine windows' sizes.
    assert!(registrar.planned() >= 2);
}

#[test]
fn a_low_overlap_is_found_by_its_correlation() {
    // Grains have texture everywhere: a sparse strip of cells may hold none to register by.
    let world = Grains::new(GrainsParams {
        seed: 7,
        ..GrainsParams::default()
    });
    let registrar = Registrar::default();
    // A fifth of the image overlapping, a strip of 102 pixels, either way: as tiles of a scan.
    for shift in [[410.4, -10.6], [-6.2, 409.8], [-408.3, 7.7]] {
        let (a, b) = pair(&world, shift, (512, 512));
        let found = registrar.register(a.view(), b.view()).unwrap();
        let e = error(found.shift, shift);
        assert!(e < 0.2, "{shift:?}: found {:?}, {e:.3} px off", found.shift);
    }
}

/// A grating with a period of 16 pixels.
struct Grating;

impl World for Grating {
    fn intensity(&self, at: Vec2Um, _: Micrometers) -> f32 {
        let [x, y] = at
            .to_um()
            .map(|v| (std::f64::consts::TAU * v / (16.0 * PIXEL)).sin());
        (0.5 + 0.25 * (x + y)) as f32
    }

    fn info(&self) -> siq_synth::WorldInfo {
        unreachable!("not described")
    }
}

#[test]
fn periodic_and_unrelated_images_are_told_as_doubtful() {
    let registrar = Registrar::default();
    let (a, b) = pair(&cells(3), [5.3, 2.1], (256, 256));
    let clear = registrar.register(a.view(), b.view()).unwrap();
    // A grating's peaks repeat every period: none stands out.
    let (a, b) = pair(&Grating, [5.3, 2.1], (256, 256));
    let periodic = registrar.register(a.view(), b.view()).unwrap();
    assert!(periodic.margin < 0.2, "{periodic:?}");
    assert!(clear.margin > 0.5, "{clear:?}");
    // Two unrelated patches. A clear match stands out by thousands, unrelated content by tens:
    // the peak-to-correlation energy tells them apart, where a chance correlation of blobs can
    // reach 0.7.
    let a = patch(&cells(3), [0.0, 0.0], (256, 256), 1);
    let b = patch(&cells(4), [0.0, 0.0], (256, 256), 2);
    let unrelated = registrar.register(a.view(), b.view()).unwrap();
    assert!(clear.pce > 1000.0, "{clear:?}");
    assert!(unrelated.pce < 100.0, "{unrelated:?}");
}

#[test]
fn every_option_meets_its_accuracy() {
    let world = cells(9);
    for (window, subpixel, smoothing, limit) in [
        (Window::Hann, Subpixel::Gaussian, 1.0, 0.1),
        (Window::Hann, Subpixel::Parabolic, 1.0, 0.1),
        (Window::Tukey(0.2), Subpixel::Parabolic, 1.0, 0.1),
        // Foroosh's estimate assumes the unwindowed peak: the weakest here.
        (Window::Hann, Subpixel::Foroosh, 0.0, 0.5),
        (Window::Tukey(0.2), Subpixel::Foroosh, 0.0, 0.5),
    ] {
        let registrar = Registrar::new(Options {
            window,
            subpixel,
            smoothing,
            ..Options::default()
        });
        let worst = shifts()
            .into_iter()
            .map(|shift| {
                let (a, b) = pair(&world, shift, (256, 256));
                error(registrar.register(a.view(), b.view()).unwrap().shift, shift)
            })
            .fold(0.0, f64::max);
        assert!(
            worst < limit,
            "{window:?}, {subpixel:?}, {smoothing}: {worst:.3} px at worst"
        );
    }
}

#[test]
fn bad_inputs_are_refused() {
    let registrar = Registrar::default();
    let (a, b) = (
        OwnedImage::<u16>::new(64, 48),
        OwnedImage::<u16>::new(64, 40),
    );
    assert!(matches!(
        registrar.register(a.view(), b.view()),
        Err(RegisterError::Sizes { .. })
    ));
    let tiny = OwnedImage::<u16>::new(8, 64);
    assert!(matches!(
        registrar.register(tiny.view(), tiny.view()),
        Err(RegisterError::TooSmall(_))
    ));
    let flat = OwnedImage::from_fn(64, 64, |_, _| 1000u16);
    assert_eq!(
        registrar.register(flat.view(), flat.view()),
        Err(RegisterError::Flat)
    );
    // A frame of u16 is taken as it is.
    let world = cells(3);
    let (a, b) = pair(&world, [3.0, -4.0], (128, 96));
    let to_u16 = |image: &OwnedImage<f32>| {
        let view = image.view();
        OwnedImage::from_fn(128, 96, |x, y| (view.get(x, y).unwrap() * 10_000.0) as u16)
    };
    let (a, b) = (to_u16(&a), to_u16(&b));
    let found = registrar.register(a.view(), b.view()).unwrap();
    assert!(error(found.shift, [3.0, -4.0]) < 0.1, "{found:?}");
}
