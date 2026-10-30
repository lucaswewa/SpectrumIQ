//! Stitching's accuracy on ground truth (ADR-0061 to ADR-0063, amended in P8.5): `siq-datagen`
//! scans over grains and cells, of different sizes, overlaps, stage errors and light, registered
//! and solved, each tile then placed within 0.3 px RMS and 1 px at worst of the truth. A few small
//! cases run in CI; the whole matrix runs with `cargo test --release -p siq-stitch --test
//! accuracy -- --ignored`. REQ-STI-002, REQ-STI-004.

use serde_json::Value;
use siq_datagen::Output;
use siq_datagen::tilescan::{TileScanArgs, tilescan};
use siq_stitch::pairs::{self, PairOptions};
use siq_stitch::solve::{self, SolveOptions};
use siq_stitch::{MappedTiles, Mosaic};
use siq_storage::{Dataset, DatasetReader};
use siq_types::Affine2;

/// The generator's camera: 3.76 µm pixels behind a 10× objective.
const PIXEL_UM: f64 = 0.376;

/// A scan of the matrix.
#[derive(Debug, Clone, Copy)]
struct Case {
    world: &'static str,
    grid: (u32, u32),
    tile: (u32, u32),
    overlap: f64,
    stage_um: f64,
    light: f64,
    seed: u64,
}

/// The exit criteria's scan: tiles of 2048 × 1536 over grains, 15 % overlapping, the stage off
/// by 5 µm, in the camera's usual light.
const BASE: Case = Case {
    world: "grains",
    grid: (3, 3),
    tile: (2048, 1536),
    overlap: 0.15,
    stage_um: 5.0,
    light: 1.0,
    seed: 0,
};

/// How a case went: its tiles' placement against the truth, and its pairs.
#[derive(Debug)]
struct Placement {
    rms_px: f64,
    max_px: f64,
    trusted: usize,
    pairs: usize,
    stage_rms_px: f64,
}

/// The case stitched: registered and solved with the defaults but the stage's error, which a scan
/// knows of its stage.
fn stitch(case: Case) -> Placement {
    let folder = tempfile::tempdir().unwrap();
    let args = TileScanArgs {
        world: case.world.to_owned(),
        columns: case.grid.0,
        rows: case.grid.1,
        width: case.tile.0,
        height: case.tile.1,
        overlap: case.overlap,
        stage_error: case.stage_um,
        light: case.light,
        no_mosaic: true,
        colour: false,
    };
    let out = Output {
        data_root: folder.path().to_owned(),
        smoke: false,
        seed: case.seed,
    };
    let dataset = Dataset::open(tilescan(&args, &out).unwrap()).unwrap();
    let text = std::fs::read_to_string(dataset.path().join("truth/tilescan.json")).unwrap();
    let truth: Value = serde_json::from_str(&text).unwrap();
    let reader = DatasetReader::open(&dataset).unwrap();
    let (source, frames) = MappedTiles::open(&reader, |_| true).unwrap();
    let l = Affine2::scale(1.0 / PIXEL_UM, 1.0 / PIXEL_UM);
    let mosaic = Mosaic::place(&frames, &l).unwrap();
    let options = PairOptions {
        stage_sigma_um: case.stage_um,
        ..PairOptions::default()
    };
    let found = mosaic.overlaps(options.min_overlap_px, options.min_overlap_share);
    let threads = rayon::current_num_threads();
    let edges = pairs::register(&source, &mosaic, &found, None, &options, threads).unwrap();
    let solving = SolveOptions {
        stage_sigma_um: case.stage_um,
        ..SolveOptions::default()
    };
    let solution = solve::solve(&mosaic, &edges, &solving);
    // The truth's corners, and each placement against them once the means agree.
    let corners: Vec<[f64; 2]> = (0..mosaic.tiles.len())
        .map(|p| {
            let at =
                |key: &str, axis: usize| truth["tiles"][p][key][axis].as_f64().unwrap() / PIXEL_UM;
            let nominal = mosaic.tiles[p].nominal;
            [0, 1].map(|axis| nominal[axis] + at("true", axis) - at("nominal", axis))
        })
        .collect();
    let misplaced = |placed: &[[f64; 2]]| {
        let n = placed.len() as f64;
        let mean = [0, 1].map(|axis| {
            placed
                .iter()
                .zip(&corners)
                .map(|(s, t)| s[axis] - t[axis])
                .sum::<f64>()
                / n
        });
        let errors: Vec<f64> = placed
            .iter()
            .zip(&corners)
            .map(|(s, t)| (s[0] - t[0] - mean[0]).hypot(s[1] - t[1] - mean[1]))
            .collect();
        let rms = (errors.iter().map(|e| e * e).sum::<f64>() / n).sqrt();
        (rms, errors.iter().copied().fold(0.0, f64::max))
    };
    let (rms_px, max_px) = misplaced(&solution.positions);
    let stage: Vec<[f64; 2]> = mosaic.tiles.iter().map(|tile| tile.nominal).collect();
    Placement {
        rms_px,
        max_px,
        trusted: edges.iter().filter(|edge| edge.confident).count(),
        pairs: edges.len(),
        stage_rms_px: misplaced(&stage).0,
    }
}

/// Stitches each case, and fails on any outside the targets, listing them all.
fn check(cases: &[Case]) {
    let mut missed = Vec::new();
    for case in cases {
        let placed = stitch(*case);
        eprintln!(
            "{:6} {}×{} of {}×{} at {:.0} %, stage {} µm, light {}: {:.3} px RMS, {:.3} at worst; {}/{} pairs trusted; the stage {:.1} px RMS",
            case.world,
            case.grid.0,
            case.grid.1,
            case.tile.0,
            case.tile.1,
            case.overlap * 100.0,
            case.stage_um,
            case.light,
            placed.rms_px,
            placed.max_px,
            placed.trusted,
            placed.pairs,
            placed.stage_rms_px
        );
        if placed.rms_px > 0.3 || placed.max_px > 1.0 {
            missed.push((*case, placed));
        }
    }
    assert!(missed.is_empty(), "outside the targets: {missed:#?}");
}

#[test]
fn small_scans_of_dense_and_sparse_specimens_are_placed_within_the_targets() {
    check(&[
        Case {
            tile: (512, 384),
            seed: 11,
            ..BASE
        },
        Case {
            grid: (4, 3),
            tile: (512, 384),
            overlap: 0.2,
            stage_um: 2.0,
            light: 0.05,
            seed: 12,
            ..BASE
        },
        Case {
            world: "cells",
            tile: (1024, 768),
            seed: 13,
            ..BASE
        },
    ]);
}

#[test]
#[ignore = "the whole matrix: minutes, in release"]
fn the_scan_matrix_is_placed_within_the_targets() {
    // The exit criteria's scans, then each of the 5 × 5 scan's parameters either way.
    let mut cases = Vec::new();
    for world in ["grains", "cells"] {
        for grid in [(3, 3), (5, 5), (10, 10)] {
            cases.push(Case {
                world,
                grid,
                ..BASE
            });
        }
        let around = Case {
            world,
            grid: (5, 5),
            ..BASE
        };
        for overlap in [0.1, 0.2] {
            cases.push(Case { overlap, ..around });
        }
        for stage_um in [2.0, 10.0] {
            cases.push(Case { stage_um, ..around });
        }
        for light in [0.1, 0.02] {
            cases.push(Case { light, ..around });
        }
    }
    for (seed, case) in (100..).zip(&mut cases) {
        case.seed = seed;
    }
    check(&cases);
}
