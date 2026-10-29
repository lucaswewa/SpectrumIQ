//! Shading (ADR-0050) on frames the simulator renders, with its cos⁴ vignetting (the corners at
//! 0.8 of the centre), its offset of 100 ADU, its per-pixel gains and offsets, and its noise: a
//! captured flat against the vignetting, a tile scan's estimated flat and its seams, and the
//! registry's darks and flats. REQ-CAL-002.

use siq_calib::Versioned;
use siq_calib::shading::*;
use siq_devices::{CameraSettings, TriggerMode};
use siq_sim::optics::{OpticsModel, Scratch, SensorModel, Shot, Vignetting, form_image};
use siq_synth::{Cells, CellsParams, World, WorldInfo};
use siq_types::*;

const SENSOR: (u32, u32) = (320, 240);

/// A blank slide: the same light everywhere.
struct Blank;

impl World for Blank {
    fn intensity(&self, _: Vec2Um, _: Micrometers) -> f32 {
        0.8
    }

    fn info(&self) -> WorldInfo {
        unreachable!("not described")
    }
}

fn settings() -> CameraSettings {
    CameraSettings {
        roi: Roi::full(SENSOR.0, SENSOR.1),
        binning: Binning::NONE,
        pixel_format: PixelFormat::Mono14,
        exposure: Millis(10.0),
        gain: 1.0,
        trigger: TriggerMode::Software,
    }
}

/// Frame `seq` of `world` with the stage at `(x, y)`, through `optics`.
fn frame(world: &dyn World, optics: &OpticsModel, (x, y): (f64, f64), seq: u64) -> OwnedImage<u16> {
    let shot = Shot {
        pose: StagePose {
            x: Micrometers(x),
            y: Micrometers(y),
            ..StagePose::default()
        },
        calibration: Affine2::scale(1.0 / 0.376, 1.0 / 0.376),
        sensor: SENSOR,
        settings: settings(),
    };
    let mut image = OwnedImage::new(SENSOR.0 as usize, SENSOR.1 as usize);
    let sensor = SensorModel::default();
    form_image(
        world,
        optics,
        &sensor,
        &shot,
        seq,
        &mut Scratch::default(),
        &mut image.view_mut(),
    )
    .unwrap();
    image
}

fn meta() -> FrameMeta {
    let settings = settings();
    FrameMeta {
        seq: FrameSeq(0),
        device_ts: None,
        host_ts_mid_exposure: Timestamp(0),
        exposure: settings.exposure,
        gain: settings.gain,
        roi: settings.roi,
        binning: settings.binning,
        pixel_format: settings.pixel_format,
        cfa: None,
        pose: None,
        pose_quality: PoseQuality::Settled,
        channel: None,
        objective: None,
        calibration: None,
        flags: FrameFlags::default(),
    }
}

/// The true light at sensor pixel `(x, y)`, against its mean over the sensor.
fn vignetting(x: f64, y: f64) -> f64 {
    let profile = Vignetting::Cos4 { corner: 0.8 };
    let (cx, cy) = (f64::from(SENSOR.0) / 2.0, f64::from(SENSOR.1) / 2.0);
    let at =
        |x: f64, y: f64| profile.at(((x - cx).powi(2) + (y - cy).powi(2)) / (cx * cx + cy * cy));
    let mean = (0..SENSOR.1)
        .flat_map(|j| (0..SENSOR.0).map(move |i| (f64::from(i) + 0.5, f64::from(j) + 0.5)))
        .map(|(x, y)| at(x, y))
        .sum::<f64>()
        / f64::from(SENSOR.0 * SENSOR.1);
    at(x, y) / mean
}

/// The largest error of `flat` against the true vignetting, relative.
fn worst(flat: &FlatField) -> f64 {
    let mut worst = 0.0f64;
    for j in 0..=12 {
        for i in 0..=16 {
            let (x, y) = (f64::from(i) * 20.0, f64::from(j) * 20.0);
            worst = worst.max((flat.at(x, y) / vignetting(x, y) - 1.0).abs());
        }
    }
    worst
}

/// The mean of `frames` frames through `optics`.
fn mean_of(world: &dyn World, optics: &OpticsModel, frames: u64) -> Accumulator {
    let mut sum = Accumulator::new(SENSOR.0 as usize, SENSOR.1 as usize);
    for seq in 0..frames {
        sum.add(&frame(world, optics, (0.0, 0.0), seq).view())
            .unwrap();
    }
    sum
}

#[test]
fn a_captured_flat_recovers_the_vignetting_within_two_percent() {
    let dark_optics = OpticsModel {
        flux: 0.0,
        ..OpticsModel::default()
    };
    let dark = mean_of(&Blank, &dark_optics, 8).mean();
    let dark = Dark {
        image: OwnedImage::from_fn(dark.width(), dark.height(), |x, y| {
            dark.view().row(y)[x].round() as u16
        }),
        binning: Binning::NONE,
    };
    assert!((dark.level() - 100.0).abs() < 1.0, "{}", dark.level());

    let flat = mean_of(&Blank, &OpticsModel::default(), 8).mean();
    let sensor = [SENSOR.0, SENSOR.1];
    let area = Roi::full(SENSOR.0, SENSOR.1);
    let flat = FlatField::fit(&flat.view(), area, sensor, dark.level() as f32).unwrap();
    assert!(worst(&flat) < 0.02, "off by {:.2} %", worst(&flat) * 100.0);
    let [low, high] = flat.range();
    assert!(low < 0.9 && high > 1.05, "{low} – {high}");

    // A frame corrected through the view is flat, within its noise and per-pixel gains.
    let raw = frame(&Blank, &OpticsModel::default(), (0.0, 0.0), 99);
    let corrected = FlatFieldView::new(&meta(), Some(&dark), Some(&flat))
        .read(&raw.view(), 16383)
        .unwrap();
    let band = |image: &OwnedImage<u16>, x: usize| {
        let rows = image.view();
        (0..240).map(|y| f64::from(rows.row(y)[x])).sum::<f64>() / 240.0
    };
    let (edge, centre) = (band(&raw, 2) - 100.0, band(&raw, 160) - 100.0);
    assert!(
        edge / centre < 0.93,
        "the raw frame's edge at {:.3}",
        edge / centre
    );
    let ratio = band(&corrected, 2) / band(&corrected, 160);
    assert!(
        (ratio - 1.0).abs() < 0.01,
        "the corrected frame's edge at {ratio:.4}"
    );
}

#[test]
fn a_region_s_and_a_binning_s_gains_are_the_flat_s_at_their_place() {
    let flat = FlatField {
        sensor: [SENSOR.0, SENSOR.1],
        coefficients: {
            // 1 − 0.2 x² − 0.1 y²: x² is term 2, and y² term 13, after the 7 of y⁰ and 6 of y¹.
            let mut c = vec![0.0; 28];
            (c[0], c[2], c[13]) = (1.0, -0.2, -0.1);
            c
        },
    };
    assert!((flat.at(0.0, 120.0) - 0.8).abs() < 1e-9);
    assert!((flat.at(160.0, 0.0) - 0.9).abs() < 1e-9);
    let roi = Roi {
        offset_x: 32,
        offset_y: 16,
        width: 128,
        height: 64,
    };
    let binning = Binning::new(2, 2).unwrap();
    let gains = flat.gains(roi, binning, (64, 32));
    // Pixel (5, 3) covers sensor columns 42–43 and rows 22–23: its centre is (43, 23).
    let expected = 1.0 / flat.at(43.0, 23.0);
    assert!((f64::from(gains.view().row(3)[5]) - expected).abs() < 1e-5);
}

/// A tile scan of the cells world, `columns` × `rows` tiles overlapping by 15 %: each tile with
/// the vignetting, and as it would be without.
fn tile_scan(columns: u32, rows: u32) -> Vec<(OwnedImage<u16>, OwnedImage<u16>)> {
    let world = Cells::new(CellsParams {
        seed: 4,
        ..CellsParams::default()
    });
    let flat = OpticsModel {
        vignetting: Vignetting::None,
        ..OpticsModel::default()
    };
    let field = [f64::from(SENSOR.0) * 0.376, f64::from(SENSOR.1) * 0.376];
    let mut tiles = Vec::new();
    for (p, (i, j)) in (0..).zip((0..rows).flat_map(|j| (0..columns).map(move |i| (i, j)))) {
        let at = (
            f64::from(i) * field[0] * 0.85,
            f64::from(j) * field[1] * 0.85,
        );
        let shaded = frame(&world, &OpticsModel::default(), at, p);
        tiles.push((shaded, frame(&world, &flat, at, p)));
    }
    tiles
}

/// The seam bands' light against the tiles' centres, each against the tiles as they would be
/// without vignetting, less 1: the bands are the outer 15 % each way, where neighbours overlap,
/// and the centre is the middle third.
fn seam_contrast(tiles: &[(OwnedImage<u16>, OwnedImage<u16>)]) -> f64 {
    let (w, h) = (SENSOR.0 as usize, SENSOR.1 as usize);
    let (bx, by) = (w * 15 / 100, h * 15 / 100);
    let mut sums = [[0.0f64; 2]; 2];
    for (image, truth) in tiles {
        for y in 0..h {
            let (image, truth) = (image.view().row(y), truth.view().row(y));
            for x in 0..w {
                let band = x < bx || x >= w - bx || y < by || y >= h - by;
                let centre = (w / 3..2 * w / 3).contains(&x) && (h / 3..2 * h / 3).contains(&y);
                if band || centre {
                    let slot = &mut sums[usize::from(centre)];
                    slot[0] += f64::from(image[x]) - 100.0;
                    slot[1] += f64::from(truth[x]) - 100.0;
                }
            }
        }
    }
    let [band, centre] = sums.map(|[image, truth]| image / truth);
    (band / centre - 1.0).abs()
}

#[test]
fn a_tile_scan_s_estimated_flat_recovers_the_vignetting_and_fades_its_seams() {
    let tiles = tile_scan(6, 5);
    let sensor = [SENSOR.0, SENSOR.1];
    let area = Roi::full(SENSOR.0, SENSOR.1);
    let mut estimator = Estimator::new(100.0);
    for (image, _) in &tiles {
        estimator.add(&image.view()).unwrap();
    }
    assert_eq!(estimator.frames(), 30);
    let flat = estimator.finish(area, sensor).unwrap();
    assert!(worst(&flat) < 0.03, "off by {:.2} %", worst(&flat) * 100.0);

    let dark = Dark {
        image: OwnedImage::from_fn(SENSOR.0 as usize, SENSOR.1 as usize, |_, _| 100),
        binning: Binning::NONE,
    };
    let view = FlatFieldView::new(&meta(), Some(&dark), Some(&flat));
    let corrected: Vec<_> = tiles
        .iter()
        .map(|(image, truth)| (view.read(&image.view(), 16383).unwrap(), truth.clone()))
        .collect();
    let (before, after) = (seam_contrast(&tiles), seam_contrast(&corrected));
    assert!(before > 0.04, "the seams at {:.2} %", before * 100.0);
    assert!(
        after * 5.0 < before,
        "the seams from {:.2} % to {:.2} %",
        before * 100.0,
        after * 100.0
    );

    // Too few frames are refused, and frames of another size.
    let mut few = Estimator::new(100.0);
    for (image, _) in tiles.iter().take(3) {
        few.add(&image.view()).unwrap();
    }
    assert!(few.finish(area, sensor).is_err());
    assert!(few.add(&OwnedImage::<u16>::new(64, 48).view()).is_err());
}

#[test]
fn darks_and_flats_are_kept_per_objective_and_channel() {
    let settings = tempfile::tempdir().unwrap();
    let registry = ShadingRegistry::open(settings.path()).unwrap();
    let key = (Some(ObjectiveId(1)), Some(ChannelId(2)));
    let dark = Dark {
        image: OwnedImage::from_fn(32, 24, |x, y| 100 + (x + y) as u16 % 3),
        binning: Binning::NONE,
    };
    let saved = registry.save_dark(key, &dark, &meta(), 8).unwrap();
    let flat = FlatField {
        sensor: [32, 24],
        coefficients: (0..28).map(|k| f64::from(u8::from(k == 0))).collect(),
    };
    let record = ShadingRecord::new(key, &meta(), 16, "capture").with_flat(flat, saved.level);
    registry.save(&record, &[]).unwrap();

    let again = ShadingRegistry::open(settings.path()).unwrap();
    let (objective, channel) = key;
    assert_eq!(
        again.current((objective, channel, ShadingKind::Dark)),
        Some(saved.clone())
    );
    assert_eq!(
        again.current((objective, channel, ShadingKind::Flat)),
        Some(record.clone())
    );
    assert_eq!(again.current((objective, None, ShadingKind::Flat)), None);
    assert_eq!(record.range, Some([1.0, 1.0]));
    assert_eq!(saved.key(), (objective, channel, ShadingKind::Dark));
    let read = again.dark(&saved).unwrap();
    assert_eq!(
        read.image.view().rows().flatten().collect::<Vec<_>>(),
        dark.image.view().rows().flatten().collect::<Vec<_>>()
    );
    let file = again.path(&saved, "dark.npy");
    assert!(
        std::fs::read(file)
            .unwrap()
            .starts_with(b"\x93NUMPY\x01\x00")
    );
}
