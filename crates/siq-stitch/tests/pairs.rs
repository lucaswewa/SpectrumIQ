//! The overlap graph and pairwise registration (ADR-0061): tiles placed by their poses through the
//! calibration, the pairs that overlap enough, every pair's offset recovered on a `siq-datagen`
//! scan to a fraction of a pixel of the truth, flat-corrected or not, mono or colour, and an
//! overlap with nothing to register not trusted. REQ-STI-002.

use serde_json::Value;
use siq_calib::shading::{DEGREE, FlatField};
use siq_datagen::Output;
use siq_datagen::tilescan::{TileScanArgs, tilescan};
use siq_stitch::pairs::{self, PairOptions};
use siq_stitch::{MappedTiles, Mosaic, Tile};
use siq_storage::{Dataset, DatasetReader, FrameKey};
use siq_types::{
    Affine2, AffineParams, Binning, FrameFlags, FrameMeta, FrameSeq, Micrometers, Millis,
    OwnedImage, PixelFormat, PoseQuality, Roi, StagePose, Timestamp,
};

/// The generator's camera: 3.76 µm pixels behind a 10× objective.
const PIXEL_UM: f64 = 0.376;

fn meta(width: u32, height: u32, binning: u8, [x, y]: [f64; 2]) -> FrameMeta {
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
        pose: Some(StagePose {
            x: Micrometers(x),
            y: Micrometers(y),
            ..StagePose::default()
        }),
        pose_quality: PoseQuality::Settled,
        channel: None,
        objective: None,
        calibration: None,
        flags: FrameFlags::default(),
    }
}

fn key(p: u32) -> FrameKey {
    FrameKey {
        t: 0,
        c: 0,
        z: 0,
        p,
    }
}

#[test]
fn tiles_are_placed_by_their_poses_and_joined_where_they_overlap() {
    // 2.66 sensor pixels a µm, turned 0.4°, binned 2 × 2: 1.33 mosaic pixels a µm, and a frame of
    // 100 × 75 pixels.
    let l = Affine2::from_params(&AffineParams {
        scale_x: 1.0 / PIXEL_UM,
        scale_y: 1.0 / PIXEL_UM,
        rotation: 0.4_f64.to_radians(),
        shear: 0.0,
        translation: [0.0, 0.0],
    });
    let at = [-20_000.0, -15_000.0];
    let mosaic = Mosaic::place(&[(key(0), meta(200, 150, 2, at))], &l).unwrap();
    assert!((mosaic.px_per_um - 0.5 / PIXEL_UM).abs() < 1e-12);
    let [x, y] = l.apply_vector(at);
    let corner = mosaic.tiles[0].nominal;
    assert!((corner[0] - (x / 2.0 - 50.0)).abs() < 1e-9);
    assert!((corner[1] - (y / 2.0 - 37.5)).abs() < 1e-9);

    // A 3 × 3 grid of 100 × 80 tiles, 85 and 68 apart: 15 × 12 pixels at the corners.
    let tiles = (0..9u32)
        .map(|p| Tile {
            key: key(p),
            size: [100, 80],
            nominal: [f64::from(p % 3) * 85.0, f64::from(p / 3) * 68.0],
            sensor: [0, 0],
            binning: 1,
            cfa: None,
        })
        .collect();
    let grid = Mosaic {
        tiles,
        px_per_um: 1.0,
    };
    // The sides overlap by 15 % of a tile; the corners by 2.25 %.
    let sides = grid.overlaps(12.0, 0.05);
    assert_eq!(sides.len(), 12, "{sides:?}");
    assert!(sides.contains(&(0, 1)) && sides.contains(&(0, 3)) && !sides.contains(&(0, 4)));
    assert_eq!(grid.overlaps(12.0, 0.02).len(), 20);
    assert!(grid.overlaps(13.0, 0.0).iter().all(|&(i, j)| j == i + 1));
}

/// A `siq-datagen` scan of `world`, `columns` × `rows` tiles of 512 × 384 at 20 % overlap, the
/// stage off by 5 µm a standard deviation, in colour if `colour`: its dataset and truth.
fn scan(
    root: &std::path::Path,
    world: &str,
    (columns, rows): (u32, u32),
    colour: bool,
) -> (Dataset, Value) {
    let args = TileScanArgs {
        world: world.to_owned(),
        columns,
        rows,
        width: 512,
        height: 384,
        overlap: 0.2,
        stage_error: 5.0,
        light: 1.0,
        no_mosaic: true,
        colour,
    };
    let out = Output {
        data_root: root.to_owned(),
        smoke: false,
        seed: 3,
    };
    let dataset = Dataset::open(tilescan(&args, &out).unwrap()).unwrap();
    let text = std::fs::read_to_string(dataset.path().join("truth/tilescan.json")).unwrap();
    (dataset, serde_json::from_str(&text).unwrap())
}

/// Where the truth puts tile `p`'s corner from tile 0's, in mosaic pixels.
fn true_offset(truth: &Value, i: usize, j: usize) -> [f64; 2] {
    let at = |p: usize, axis: usize| truth["tiles"][p]["true"][axis].as_f64().unwrap();
    [0, 1].map(|axis| (at(j, axis) - at(i, axis)) / PIXEL_UM)
}

// ADR-0071: a colour scan's tiles keep their filter array, and register on their luminance as
// well as mono ones do.
#[test]
fn every_pair_s_offset_is_recovered_on_ground_truth() {
    for colour in [false, true] {
        recovered(colour);
    }
}

fn recovered(colour: bool) {
    let folder = tempfile::tempdir().unwrap();
    let (dataset, truth) = scan(folder.path(), "grains", (3, 3), colour);
    let reader = DatasetReader::open(&dataset).unwrap();
    let (source, frames) = MappedTiles::open(&reader, |_| true).unwrap();
    let l = Affine2::scale(1.0 / PIXEL_UM, 1.0 / PIXEL_UM);
    let mosaic = Mosaic::place(&frames, &l).unwrap();
    let cfa = colour.then_some(siq_types::Cfa::Gbrg);
    assert!(mosaic.tiles.iter().all(|tile| tile.cfa == cfa));
    let options = PairOptions::default();
    let pairs = mosaic.overlaps(options.min_overlap_px, options.min_overlap_share);
    // The 12 sides; not the corners, 102 × 77 pixels, 4 % of a tile.
    assert_eq!(pairs.len(), 12);
    // Uncorrected, then through a flat of even light, which changes nothing.
    let mut coefficients = vec![0.0; (DEGREE + 1) * (DEGREE + 2) / 2];
    coefficients[0] = 1.0;
    let even = FlatField {
        sensor: [512, 384],
        coefficients,
    };
    for shading in [None, Some((0.0, &even))] {
        let edges = pairs::register(&source, &mosaic, &pairs, shading, &options, 2).unwrap();
        let errors: Vec<f64> = edges
            .iter()
            .map(|edge| {
                assert!(edge.confident, "{edge:?}");
                let [tx, ty] = true_offset(&truth, edge.i, edge.j);
                (edge.shift[0] - tx).hypot(edge.shift[1] - ty)
            })
            .collect();
        let rms = (errors.iter().map(|e| e * e).sum::<f64>() / errors.len() as f64).sqrt();
        let worst = errors.iter().copied().fold(0.0, f64::max);
        eprintln!(
            "{} pairs trusted, colour {colour}: {rms:.3} px RMS, {worst:.3} px at worst",
            errors.len()
        );
        assert!(rms < 0.2 && worst < 0.5, "{rms} RMS, {worst} at worst");
    }
}

/// Texture that varies smoothly and in every bit, the same each run, offset by `at`.
fn textured(width: u32, height: u32, at: [u32; 2]) -> OwnedImage<u16> {
    OwnedImage::from_fn(width as usize, height as usize, |x, y| {
        let (fx, fy) = ((x as u32 + at[0]) as f64, (y as u32 + at[1]) as f64);
        let wave = (fx * 0.11).sin() * (fy * 0.07).cos() + (fx * 0.023 + fy * 0.031).sin();
        (20_000.0 + 8_000.0 * wave) as u16
    })
}

/// Noise about a level, nothing else: independent in each tile.
fn noise(width: u32, height: u32, seed: u32) -> OwnedImage<u16> {
    OwnedImage::from_fn(width as usize, height as usize, |x, y| {
        let mut h = (x as u32).wrapping_mul(374_761_393) ^ (y as u32).wrapping_mul(668_265_263);
        h = (h ^ seed.wrapping_mul(2_246_822_519)).wrapping_mul(1_274_126_177);
        1000 + (h >> 28) as u16
    })
}

#[test]
fn an_overlap_with_nothing_to_register_is_not_trusted() {
    // Two tiles side by side, 64 pixels over each other: textured, then only noise.
    let place = |x: f64| Tile {
        key: key(0),
        size: [256, 192],
        nominal: [x, 0.0],
        sensor: [0, 0],
        binning: 1,
        cfa: None,
    };
    let mosaic = Mosaic {
        tiles: vec![place(0.0), place(192.0)],
        px_per_um: 2.66,
    };
    let pairs = mosaic.overlaps(32.0, 0.05);
    assert_eq!(pairs, [(0, 1)]);
    let options = PairOptions::default();
    // The second tile truly 3 pixels right and 2 down of where it was said to be.
    let sharp = [textured(256, 192, [0, 0]), textured(256, 192, [195, 2])];
    let edges = pairs::register(&sharp[..], &mosaic, &pairs, None, &options, 1).unwrap();
    assert!(
        edges[0].confident && edges[0].pce > 1000.0,
        "{:?}",
        edges[0]
    );
    let [dx, dy] = edges[0].shift;
    assert!(
        (dx - 195.0).abs() < 0.05 && (dy - 2.0).abs() < 0.05,
        "{dx}, {dy}"
    );
    let dull = [noise(256, 192, 1), noise(256, 192, 2)];
    let edges = pairs::register(&dull[..], &mosaic, &pairs, None, &options, 1).unwrap();
    eprintln!("noise only: {:?}", edges[0]);
    assert!(!edges[0].confident, "{:?}", edges[0]);
    assert!(edges[0].pce < options.min_pce || edges[0].ncc < options.min_ncc);
}
