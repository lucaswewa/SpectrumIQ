//! Image formation: determinism, defocus, vignetting, noise, ROI and binning, clipping, and the
//! renderer's cache.

use std::sync::Arc;

use siq_devices::{CameraSettings, TriggerMode};
use siq_imgproc::{FocusMeasure, focus};
use siq_sim::optics::*;
use siq_synth::{Cells, CellsParams, World, WorldInfo};
use siq_types::{
    Affine2, Binning, Cfa, ImageView, Micrometers, Millis, OwnedImage, PixelFormat, RectPx, Roi,
    StagePose, Vec2Um,
};

/// The same intensity everywhere, on a flat surface.
struct Uniform(f32);

impl World for Uniform {
    fn intensity(&self, _: Vec2Um, _: Micrometers) -> f32 {
        self.0
    }

    fn surface_z(&self, _: Vec2Um) -> Option<Micrometers> {
        Some(Micrometers::ZERO)
    }

    fn info(&self) -> WorldInfo {
        WorldInfo::Cells(CellsParams::default())
    }
}

/// Cells on two levels: the surface is at 0 where x < 0, and 30 µm lower where x ≥ 0.
struct Step(Cells);

impl World for Step {
    fn intensity(&self, at: Vec2Um, footprint: Micrometers) -> f32 {
        self.0.intensity(at, footprint)
    }

    fn surface_z(&self, at: Vec2Um) -> Option<Micrometers> {
        Some(Micrometers(if at.x.0 < 0.0 { 0.0 } else { -30.0 }))
    }

    fn info(&self) -> WorldInfo {
        self.0.info()
    }
}

fn cells() -> Cells {
    Cells::new(CellsParams {
        seed: 3,
        spacing: Micrometers(12.0),
        radius: [Micrometers(3.0), Micrometers(5.0)],
        ..CellsParams::default()
    })
}

/// A 256 × 192 sensor at 0.5 µm per pixel, focused on z = 0.
fn shot(focus: f64) -> Shot {
    let settings = CameraSettings {
        roi: Roi::full(256, 192),
        binning: Binning::NONE,
        pixel_format: PixelFormat::Mono14,
        exposure: Millis(10.0),
        gain: 1.0,
        trigger: TriggerMode::FreeRun,
    };
    Shot {
        pose: StagePose {
            focus: Micrometers(focus),
            ..StagePose::default()
        },
        calibration: Affine2::scale(2.0, 2.0),
        sensor: (256, 192),
        settings,
    }
}

fn quiet() -> SensorModel {
    SensorModel {
        dsnu: 0.0,
        prnu: 0.0,
        ..SensorModel::default()
    }
}

fn signal(
    world: &dyn World,
    optics: &OpticsModel,
    sensor: &SensorModel,
    shot: &Shot,
) -> OwnedImage<f32> {
    let mut scratch = Scratch::default();
    render_signal(world, optics, sensor, shot, 1, &mut scratch).unwrap();
    let view = scratch.signal();
    OwnedImage::from_fn(view.width(), view.height(), |x, y| view.row(y)[x])
}

fn frame(
    world: &dyn World,
    optics: &OpticsModel,
    sensor: &SensorModel,
    shot: &Shot,
    seq: u64,
) -> OwnedImage<u16> {
    let (width, height) = shot.image_size();
    let mut out = OwnedImage::new(width, height);
    form_image(
        world,
        optics,
        sensor,
        shot,
        seq,
        &mut Scratch::default(),
        &mut out.view_mut(),
    )
    .unwrap();
    out
}

fn pixels<T: Copy>(view: ImageView<'_, T>) -> Vec<T> {
    view.rows().flatten().copied().collect()
}

// REQ-SIM-001
#[test]
fn a_frame_repeats_for_its_seed_and_number() {
    let (world, optics, sensor) = (cells(), OpticsModel::default(), SensorModel::default());
    let first = pixels(frame(&world, &optics, &sensor, &shot(0.0), 7).view());
    assert_eq!(
        first,
        pixels(frame(&world, &optics, &sensor, &shot(0.0), 7).view())
    );
    assert_ne!(
        first,
        pixels(frame(&world, &optics, &sensor, &shot(0.0), 8).view()),
        "the noise is per frame"
    );
    let other = SensorModel { seed: 1, ..sensor };
    assert_ne!(
        first,
        pixels(frame(&world, &optics, &other, &shot(0.0), 7).view()),
        "the pattern is per sensor"
    );
}

// REQ-SIM-001
#[test]
fn sharpness_falls_as_the_focus_moves_away_either_way() {
    let (world, optics, sensor) = (cells(), OpticsModel::default(), quiet());
    let sharpness = |dz: f64| {
        focus(
            &signal(&world, &optics, &sensor, &shot(dz)).view(),
            FocusMeasure::Tenengrad,
        )
    };
    let curve: Vec<f64> = [0.0, 1.0, 2.0, 5.0, 10.0, 20.0, 40.0]
        .map(sharpness)
        .to_vec();
    assert!(curve.windows(2).all(|w| w[1] < w[0]), "{curve:?}");
    assert!(curve[6] < curve[0] / 20.0, "{curve:?}");
    let (above, below) = (sharpness(10.0), sharpness(-10.0));
    assert!(
        (above - below).abs() <= 0.02 * above,
        "symmetric: {above} and {below}"
    );
    assert!(
        (optics.blur(Micrometers(0.0)).0 - 0.21 * 0.55 / 0.30).abs() < 1e-12,
        "diffraction-limited in focus"
    );
}

// REQ-SIM-001
#[test]
fn vignetting_falls_from_the_centre_to_its_corner_value() {
    let sensor = quiet();
    for vignetting in [
        Vignetting::Cos4 { corner: 0.8 },
        Vignetting::Polynomial { a: -0.3, b: 0.1 },
    ] {
        let optics = OpticsModel {
            vignetting,
            ..OpticsModel::default()
        };
        let image = signal(&Uniform(0.5), &optics, &sensor, &shot(0.0));
        let view = image.view();
        let centre = f64::from(view.row(96)[128]);
        let diagonal: Vec<f64> = (0..96)
            .map(|i| f64::from(view.row(96 + i)[128 + i * 4 / 3]))
            .collect();
        assert!(
            diagonal.windows(2).all(|w| w[1] <= w[0]),
            "{vignetting:?} falls outward"
        );
        let corner = f64::from(view.row(191)[255]) / centre;
        assert!(
            (corner - vignetting.at(1.0)).abs() < 0.01,
            "{vignetting:?}: corner {corner}"
        );
    }
    assert!((Vignetting::Cos4 { corner: 0.8 }.at(1.0) - 0.8).abs() < 1e-12);
}

// REQ-SIM-001
#[test]
fn noise_has_the_mean_and_variance_of_poisson_plus_read_noise() {
    let sensor = SensorModel {
        electrons_per_adu: 1.0,
        dark_offset: 100.0,
        ..quiet()
    };
    let optics = OpticsModel {
        vignetting: Vignetting::None,
        flux: 100.0,
        ..OpticsModel::default()
    };
    // 1000 e⁻ takes the Gaussian path, 5 e⁻ the Poisson one.
    for electrons in [1000.0, 5.0] {
        let world = Uniform((electrons / (100.0 * 10.0)) as f32);
        let values: Vec<f64> = (0..4)
            .flat_map(|seq| pixels(frame(&world, &optics, &sensor, &shot(0.0), seq).view()))
            .map(f64::from)
            .collect();
        let n = values.len() as f64;
        let mean = values.iter().sum::<f64>() / n;
        let variance = values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / n;
        let expected = electrons + sensor.read_noise.powi(2) + 1.0 / 12.0;
        assert!(
            (mean - 100.0 - electrons).abs() < 0.02 * electrons.max(5.0),
            "{electrons} e⁻: mean {mean}"
        );
        assert!(
            (variance - expected).abs() < 0.03 * expected,
            "{electrons} e⁻: variance {variance}, expected {expected}"
        );
    }
    // Without read noise, a mean of 2 e⁻ gives no electrons e⁻² of the time.
    let exact = SensorModel {
        read_noise: 0.0,
        dark_offset: 0.0,
        ..sensor
    };
    let values = pixels(frame(&Uniform(0.002), &optics, &exact, &shot(0.0), 1).view());
    let zeros = values.iter().filter(|&&v| v == 0).count() as f64 / values.len() as f64;
    assert!((zeros - (-2.0f64).exp()).abs() < 0.01, "P(0) = {zeros}");
}

// REQ-SIM-001, REQ-ACQ-003
#[test]
fn an_roi_is_a_crop_and_binning_a_sum() {
    let (world, optics, sensor) = (cells(), OpticsModel::default(), SensorModel::default());
    let full = shot(0.0);
    let mut roi = full;
    roi.settings.roi = Roi {
        offset_x: 64,
        offset_y: 32,
        width: 96,
        height: 64,
    };
    // Away from the ROI's edges, where the blur reaches past it, the crop is exact, noise and all.
    let (whole, part) = (
        frame(&world, &optics, &sensor, &full, 3),
        frame(&world, &optics, &sensor, &roi, 3),
    );
    let margin = 4;
    let inner =
        |x: u32, y: u32| RectPx::new(x + margin, y + margin, 96 - 2 * margin, 64 - 2 * margin);
    let crop = pixels(whole.view().sub_view(inner(64, 32)).unwrap());
    assert_eq!(crop, pixels(part.view().sub_view(inner(0, 0)).unwrap()));

    // A binned pixel is the sum of its four, to within the world's own averaging. Without the
    // gain spread, which a binned pixel takes once, and at a blur both sample at full detail.
    let blurred = shot(10.0);
    let mut binned = blurred;
    binned.settings.binning = Binning::new(2, 2).unwrap();
    let (fine, coarse) = (
        signal(&world, &optics, &quiet(), &blurred),
        signal(&world, &optics, &quiet(), &binned),
    );
    let (fine, coarse) = (fine.view(), coarse.view());
    let (mut worst, mut total, mut count) = (0.0f32, 0.0f32, 0.0f32);
    for y in 2..94 {
        for x in 2..126 {
            let sum: f32 = [(0, 0), (1, 0), (0, 1), (1, 1)]
                .iter()
                .map(|(dx, dy)| fine.row(2 * y + dy)[2 * x + dx])
                .sum();
            let difference = (coarse.row(y)[x] - sum).abs() / sum;
            (worst, total, count) = (worst.max(difference), total + difference, count + 1.0);
        }
    }
    let mean = total / count;
    assert!(
        mean < 0.005 && worst < 0.03,
        "binned pixels differ from sums by {mean} on average, {worst} at most"
    );
}

// REQ-SIM-001
#[test]
fn a_frame_clips_to_its_pixel_format() {
    let optics = OpticsModel {
        flux: 1e6,
        ..OpticsModel::default()
    };
    for (format, max) in [
        (PixelFormat::Mono14, 16383),
        (PixelFormat::Mono12, 4095),
        (PixelFormat::Mono8, 255),
    ] {
        let mut bright = shot(0.0);
        bright.settings.pixel_format = format;
        let values =
            pixels(frame(&Uniform(1.0), &optics, &SensorModel::default(), &bright, 0).view());
        assert!(values.iter().all(|&v| v == max), "{format:?}");
    }
    let dark = SensorModel {
        dark_offset: 0.0,
        ..SensorModel::default()
    };
    let values = pixels(frame(&Uniform(0.0), &OpticsModel::default(), &dark, &shot(0.0), 0).view());
    assert!(
        values.contains(&0) && values.iter().all(|&v| v < 20),
        "no light: noise about 0, clipped at 0"
    );
}

// REQ-SIM-001
#[test]
fn a_varying_defocus_blurs_only_where_the_surface_is_out_of_focus() {
    let world = Step(cells());
    let mut scratch = Scratch::default();
    let rendered = render_signal(
        &world,
        &OpticsModel::default(),
        &quiet(),
        &shot(0.0),
        1,
        &mut scratch,
    )
    .unwrap();
    assert!(rendered.blur[1] > 10.0 * rendered.blur[0], "{rendered:?}");
    let image = scratch.signal();
    let (left, right) = (
        image.sub_view(RectPx::new(8, 8, 100, 176)).unwrap(),
        image.sub_view(RectPx::new(148, 8, 100, 176)).unwrap(),
    );
    let (sharp, blurred) = (
        focus(&left, FocusMeasure::Tenengrad),
        focus(&right, FocusMeasure::Tenengrad),
    );
    assert!(
        sharp > 10.0 * blurred,
        "in focus {sharp}, out of focus {blurred}"
    );
}

// REQ-SIM-001
#[test]
fn the_renderer_drafts_while_moving_keeps_a_still_signal_and_samples_strong_blur_coarsely() {
    let mut renderer = Renderer::new(
        Arc::new(cells()),
        OpticsModel::default(),
        SensorModel::default(),
    );
    // Drafts of a quarter of this frame's pixels: half its resolution.
    renderer.set_draft_samples(256 * 192 / 4);
    let mut out = OwnedImage::new(256, 192);
    let mut render = |shot: &Shot, seq| renderer.render(shot, seq, &mut out.view_mut()).unwrap();
    let still = shot(0.0);
    let (draft, full, kept) = (render(&still, 0), render(&still, 1), render(&still, 2));
    assert_eq!((draft.detail, draft.cached), (2, false));
    assert_eq!((full.detail, full.cached), (1, false));
    assert_eq!((kept.detail, kept.cached), (1, true));
    let mut longer = still;
    longer.settings.exposure = Millis(20.0);
    assert!(
        render(&longer, 3).cached,
        "the exposure changes only the digitization"
    );
    let mut moved = still;
    moved.pose.x = Micrometers(5.0);
    assert_eq!(render(&moved, 4).detail, 2, "a new pose is drafted");

    let defocused = render(&shot(80.0), 5);
    assert!(
        defocused.detail >= 4 && defocused.blur[0] > 8.0,
        "{defocused:?}"
    );
    assert!(
        render(&shot(80.0), 6).detail >= 4,
        "a strong blur is still sampled coarsely at full detail"
    );
    assert!(render(&shot(80.0), 7).cached, "and then kept");
}

// REQ-ACQ-001, REQ-SIM-001: a colour sensor's pixel takes its colour's share of the light. A
// region is still a crop, its pattern the sensor's from its corner, and a binned frame keeps the
// pattern, as the Adimec's does.
#[test]
fn a_colour_sensor_samples_each_pixel_s_colour_through_its_filter() {
    let sensor = SensorModel {
        cfa: Some(Cfa::Gbrg),
        ..quiet()
    };
    let optics = OpticsModel::default();
    // Each colour's mean above the dark offset, against green's.
    let shares = |image: &OwnedImage<u16>, cfa: Cfa| {
        let mut sums = [(0.0, 0.0); 3];
        for (y, row) in image.view().rows().enumerate() {
            for (x, &value) in row.iter().enumerate() {
                let sum = &mut sums[cfa.colour(x, y)];
                *sum = (sum.0 + f64::from(value) - sensor.dark_offset, sum.1 + 1.0);
            }
        }
        let means = sums.map(|(sum, n)| sum / n);
        [means[0] / means[1], means[2] / means[1]]
    };
    let full = shot(0.0);
    let whole = frame(&Uniform(0.5), &optics, &sensor, &full, 0);
    let [red, blue] = shares(&whole, Cfa::Gbrg);
    assert!(
        (red - 0.75).abs() < 0.01 && (blue - 0.55).abs() < 0.01,
        "{red}, {blue}"
    );

    // Binned, a pixel sums four: less light, so that none clips.
    let mut binned = full;
    binned.settings.binning = Binning::new(2, 2).unwrap();
    let [red, blue] = shares(
        &frame(&Uniform(0.1), &optics, &sensor, &binned, 0),
        Cfa::Gbrg,
    );
    assert!(
        (red - 0.75).abs() < 0.01 && (blue - 0.55).abs() < 0.01,
        "binned: {red}, {blue}"
    );

    let world = cells();
    let mut odd = full;
    odd.settings.roi = Roi {
        offset_x: 65,
        offset_y: 33,
        width: 96,
        height: 64,
    };
    let (whole, part) = (
        frame(&world, &optics, &sensor, &full, 3),
        frame(&world, &optics, &sensor, &odd, 3),
    );
    let inner = |x: u32, y: u32| RectPx::new(x + 4, y + 4, 88, 56);
    let crop = pixels(whole.view().sub_view(inner(65, 33)).unwrap());
    assert_eq!(crop, pixels(part.view().sub_view(inner(0, 0)).unwrap()));
}
