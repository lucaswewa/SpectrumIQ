//! Global alignment (ADR-0062): a `siq-datagen` scan's pairs, with outlier edges injected, solved
//! within the ground truth's 0.3 px RMS and 1 px at worst, the outliers rejected; a stage's drift
//! ramp removed; a tile's true edge kept over a false one by their correlations; disconnected components each kept at their stage positions; and the dense and
//! conjugate-gradient solvers agreeing. REQ-STI-002, REQ-STI-004.

use serde_json::Value;
use siq_datagen::Output;
use siq_datagen::tilescan::{TileScanArgs, tilescan};
use siq_stitch::pairs::{self, PairOptions};
use siq_stitch::solve::{self, Method, SolveOptions};
use siq_stitch::{Edge, MappedTiles, Mosaic, Tile};
use siq_storage::{Dataset, DatasetReader, FrameKey};
use siq_types::Affine2;

/// The generator's camera: 3.76 µm pixels behind a 10× objective.
const PIXEL_UM: f64 = 0.376;

/// A grid of `columns` × `rows` tiles of 400 × 300, 340 and 255 apart: its mosaic, at 2.66 pixels
/// a µm.
fn grid(columns: u32, rows: u32) -> Mosaic {
    let tiles = (0..columns * rows)
        .map(|p| Tile {
            key: FrameKey {
                t: 0,
                c: 0,
                z: 0,
                p,
            },
            size: [400, 300],
            nominal: [
                f64::from(p % columns) * 340.0,
                f64::from(p / columns) * 255.0,
            ],
            sensor: [0, 0],
            binning: 1,
            cfa: None,
        })
        .collect();
    Mosaic {
        tiles,
        px_per_um: 2.66,
    }
}

/// A small noise of `seed`, uniform in ±`amplitude`.
fn noise(seed: u64, amplitude: f64) -> f64 {
    let mut z = seed.wrapping_mul(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    ((z >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0) * amplitude
}

/// The edges of `mosaic`'s overlaps as the truth `corners` says, each off by up to `wobble` pixels.
fn edges(mosaic: &Mosaic, corners: &[[f64; 2]], wobble: f64) -> Vec<Edge> {
    let options = PairOptions::default();
    let found = mosaic.overlaps(options.min_overlap_px, options.min_overlap_share);
    (0u64..)
        .zip(found)
        .map(|(k, (i, j))| Edge {
            i,
            j,
            shift: [0, 1].map(|axis| {
                corners[j][axis] - corners[i][axis] + noise(2 * k + axis as u64, wobble)
            }),
            pce: 5000.0,
            margin: 0.9,
            ncc: 0.95,
            overlap_px: [60, 300],
            confident: true,
        })
        .collect()
}

/// The largest and the RMS distance of `solved` from `truth`, once their means agree.
fn misplaced(solved: &[[f64; 2]], truth: &[[f64; 2]]) -> (f64, f64) {
    let n = solved.len() as f64;
    let mean = [0, 1].map(|axis| {
        solved
            .iter()
            .zip(truth)
            .map(|(s, t)| s[axis] - t[axis])
            .sum::<f64>()
            / n
    });
    let errors: Vec<f64> = solved
        .iter()
        .zip(truth)
        .map(|(s, t)| (s[0] - t[0] - mean[0]).hypot(s[1] - t[1] - mean[1]))
        .collect();
    let rms = (errors.iter().map(|e| e * e).sum::<f64>() / n).sqrt();
    (errors.iter().copied().fold(0.0, f64::max), rms)
}

#[test]
fn a_scan_s_pairs_solve_to_its_truth_with_outliers_rejected() {
    let folder = tempfile::tempdir().unwrap();
    let args = TileScanArgs {
        world: "grains".to_owned(),
        columns: 4,
        rows: 4,
        width: 512,
        height: 384,
        overlap: 0.2,
        stage_error: 5.0,
        light: 1.0,
        no_mosaic: true,
        colour: false,
    };
    let out = Output {
        data_root: folder.path().to_owned(),
        smoke: false,
        seed: 4,
    };
    let dataset = Dataset::open(tilescan(&args, &out).unwrap()).unwrap();
    let text = std::fs::read_to_string(dataset.path().join("truth/tilescan.json")).unwrap();
    let truth: Value = serde_json::from_str(&text).unwrap();
    let reader = DatasetReader::open(&dataset).unwrap();
    let (source, frames) = MappedTiles::open(&reader, |_| true).unwrap();
    let l = Affine2::scale(1.0 / PIXEL_UM, 1.0 / PIXEL_UM);
    let mosaic = Mosaic::place(&frames, &l).unwrap();
    let options = PairOptions::default();
    let found = mosaic.overlaps(options.min_overlap_px, options.min_overlap_share);
    let mut edges = pairs::register(&source, &mosaic, &found, None, &options, 2).unwrap();
    assert_eq!(edges.len(), 24);
    // Two edges made wrong, and as sure as ever: 15 and 9 pixels off.
    edges[5].shift[0] += 15.0;
    edges[17].shift = [edges[17].shift[0] - 6.0, edges[17].shift[1] + 7.0];
    let solution = solve::solve(&mosaic, &edges, &SolveOptions::default());
    let rejected: Vec<usize> = (0..edges.len())
        .filter(|&k| !solution.edges[k].used)
        .collect();
    assert_eq!(rejected, [5, 17]);
    assert_eq!(solution.components, 1);
    assert_eq!(solution.method, Method::Dense);
    // The truth's corners, from its true poses: the mosaic's corners less the stage's errors.
    let corners: Vec<[f64; 2]> = (0..16)
        .map(|p| {
            let at = |axis: usize| truth["tiles"][p]["true"][axis].as_f64().unwrap() / PIXEL_UM;
            let tile = &mosaic.tiles[p];
            let nominal =
                |axis: usize| truth["tiles"][p]["nominal"][axis].as_f64().unwrap() / PIXEL_UM;
            [0, 1].map(|axis| tile.nominal[axis] + at(axis) - nominal(axis))
        })
        .collect();
    let (worst, rms) = misplaced(&solution.positions, &corners);
    let stage: Vec<[f64; 2]> = mosaic.tiles.iter().map(|tile| tile.nominal).collect();
    let (_, stage_rms) = misplaced(&stage, &corners);
    eprintln!(
        "placed within {rms:.3} px RMS, {worst:.3} at worst, of the truth; the stage {stage_rms:.1} px RMS"
    );
    assert!(rms <= 0.3 && worst <= 1.0, "{rms} RMS, {worst} at worst");
    assert!(
        solution.rms_px < 0.3 && solution.max_px < 1.0,
        "{solution:?}"
    );
}

#[test]
fn a_drift_ramp_of_the_stage_is_removed() {
    // A row of ten tiles, the stage drifting 0.7 pixels right and 0.3 down a tile.
    let mosaic = grid(10, 1);
    let truth: Vec<[f64; 2]> = (0..10)
        .map(|k| {
            let at = mosaic.tiles[k].nominal;
            [at[0] + 0.7 * k as f64, at[1] + 0.3 * k as f64]
        })
        .collect();
    let edges = edges(&mosaic, &truth, 0.05);
    let solution = solve::solve(&mosaic, &edges, &SolveOptions::default());
    let (worst, rms) = misplaced(&solution.positions, &truth);
    let stage: Vec<[f64; 2]> = mosaic.tiles.iter().map(|tile| tile.nominal).collect();
    let (stage_worst, _) = misplaced(&stage, &truth);
    eprintln!("the ramp: {stage_worst:.2} px at worst; solved, {worst:.3} ({rms:.3} RMS)");
    // The stage's weak prior pulls the row's ends toward where it said, by a tenth of a pixel.
    assert!(
        stage_worst > 3.0 && worst < 0.2,
        "{worst} from {stage_worst}"
    );
}

#[test]
fn where_a_tile_s_two_edges_disagree_the_better_match_wins() {
    // A 3 × 3 grid whose top right tile the stage missed by 25 pixels: its edge to the left is a
    // strong true match, and its edge below a weak false one, where the stage said it was.
    let mosaic = grid(3, 3);
    let mut truth: Vec<[f64; 2]> = mosaic.tiles.iter().map(|tile| tile.nominal).collect();
    truth[2][1] -= 25.0;
    let mut edges = edges(&mosaic, &truth, 0.05);
    for edge in &mut edges {
        match (edge.i, edge.j) {
            (1, 2) => edge.ncc = 0.995,
            (2, 5) => {
                edge.shift = [0, 1].map(|axis| truth[5][axis] - mosaic.tiles[2].nominal[axis]);
                edge.ncc = 0.81;
            }
            _ => {}
        }
    }
    let solution = solve::solve(&mosaic, &edges, &SolveOptions::default());
    let false_match = edges.iter().position(|edge| (edge.i, edge.j) == (2, 5));
    assert!(!solution.edges[false_match.unwrap()].used);
    let (worst, _) = misplaced(&solution.positions, &truth);
    assert!(worst < 0.2, "{worst} px at worst");
}

#[test]
fn disconnected_components_keep_their_stage_positions() {
    // A 3 × 3 grid whose middle column has no trusted edges: its left and right columns are
    // components of three, each off by its own shift, and the middle tiles stand alone.
    let mosaic = grid(3, 3);
    let truth: Vec<[f64; 2]> = (0..9)
        .map(|p| {
            let at = mosaic.tiles[p].nominal;
            let off = if p % 3 == 0 { [4.0, -2.0] } else { [-3.0, 5.0] };
            [at[0] + off[0], at[1] + off[1] + 0.5 * (p / 3) as f64]
        })
        .collect();
    let mut edges = edges(&mosaic, &truth, 0.0);
    for edge in &mut edges {
        edge.confident = edge.i % 3 == edge.j % 3 && edge.i % 3 != 1;
    }
    let solution = solve::solve(&mosaic, &edges, &SolveOptions::default());
    // The two columns of three, and the three tiles alone.
    assert_eq!(solution.components, 5);
    assert_eq!(solution.tiles[0].component, solution.tiles[6].component);
    for p in [1, 4, 7] {
        assert_eq!(solution.positions[p], mosaic.tiles[p].nominal);
    }
    // Each column as its edges say, its mean where the stage put it.
    for column in [0, 2] {
        let members = [column, column + 3, column + 6];
        for pair in members.windows(2) {
            let (a, b) = (solution.positions[pair[0]], solution.positions[pair[1]]);
            let want = [
                truth[pair[1]][0] - truth[pair[0]][0],
                truth[pair[1]][1] - truth[pair[0]][1],
            ];
            assert!((b[0] - a[0] - want[0]).abs() < 1e-3 && (b[1] - a[1] - want[1]).abs() < 1e-3);
        }
        for axis in 0..2 {
            let mean: f64 = members
                .iter()
                .map(|&p| solution.positions[p][axis] - mosaic.tiles[p].nominal[axis])
                .sum::<f64>()
                / 3.0;
            assert!(mean.abs() < 1e-6, "column {column}: {mean}");
        }
    }
}

#[test]
fn the_dense_and_the_iterative_solvers_agree() {
    let mosaic = grid(8, 6);
    let truth: Vec<[f64; 2]> = (0u64..48)
        .map(|p| {
            let at = mosaic.tiles[p as usize].nominal;
            [at[0] + noise(100 + p, 12.0), at[1] + noise(200 + p, 12.0)]
        })
        .collect();
    let edges = edges(&mosaic, &truth, 0.1);
    let with = |method| {
        let options = SolveOptions {
            method: Some(method),
            ..SolveOptions::default()
        };
        solve::solve(&mosaic, &edges, &options)
    };
    let (dense, iterative) = (with(Method::Dense), with(Method::ConjugateGradient));
    assert_eq!(iterative.method, Method::ConjugateGradient);
    let apart = dense
        .positions
        .iter()
        .zip(&iterative.positions)
        .map(|(a, b)| (a[0] - b[0]).hypot(a[1] - b[1]))
        .fold(0.0, f64::max);
    assert!(apart < 1e-6, "{apart} px apart");
    let (worst, _) = misplaced(&dense.positions, &truth);
    assert!(worst < 0.3, "{worst}");
}
