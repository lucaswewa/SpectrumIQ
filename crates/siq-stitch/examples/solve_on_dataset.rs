//! Global alignment of a tile scan (ADR-0062): a `siq-datagen` scan, or a dataset of yours, its
//! pairs registered, then its tiles solved together, and their placement set against the truth
//! where the dataset has one.
//!
//! ```text
//! cargo run --release -p siq-stitch --example solve_on_dataset [-- <dataset folder>]
//! ```
//!
//! Without a folder, it makes a scan of 6 × 5 tiles of 2048 × 1536 over cells at 15 % overlap, the
//! stage off by 5 µm a standard deviation, in a temporary folder. The tiles are placed through the
//! dataset's calibration, or at 0.376 µm a pixel (the simulator's 10×) without one, from its first
//! channel and plane. Smoke mode (`SIQ_SMOKE=1`) makes 3 × 3 tiles of 512 × 384.

use std::error::Error;
use std::path::PathBuf;
use std::time::Instant;

use serde_json::Value;
use siq_calib::CalibrationRecord;
use siq_datagen::Output;
use siq_datagen::tilescan::{TileScanArgs, tilescan};
use siq_stitch::pairs::{self, PairOptions};
use siq_stitch::solve::{self, SolveOptions};
use siq_stitch::{MappedTiles, Mosaic};
use siq_storage::{Dataset, DatasetReader};
use siq_types::Affine2;

/// The simulator's 10× pixel, in µm.
const PIXEL_UM: f64 = 0.376;

fn main() -> Result<(), Box<dyn Error>> {
    let smoke = std::env::var_os("SIQ_SMOKE").is_some();
    let temporary = tempfile::tempdir()?;
    let path = match std::env::args().nth(1) {
        Some(folder) => PathBuf::from(folder),
        None => {
            let (columns, rows, width, height) = if smoke {
                (3, 3, 512, 384)
            } else {
                (6, 5, 2048, 1536)
            };
            let args = TileScanArgs {
                world: "cells".to_owned(),
                columns,
                rows,
                width,
                height,
                overlap: 0.15,
                stage_error: 5.0,
                light: 1.0,
                no_mosaic: true,
            };
            let out = Output {
                data_root: temporary.path().to_owned(),
                smoke: false,
                seed: 2,
            };
            println!("making {columns} × {rows} tiles of {width} × {height}");
            tilescan(&args, &out)?
        }
    };
    let dataset = Dataset::open(&path)?;
    let calibration = dataset.manifest().calibration.as_ref();
    let record = calibration
        .map(|snapshot| serde_json::from_value::<CalibrationRecord>(snapshot.value.clone()))
        .transpose()?;
    let l = record.map_or_else(
        || Affine2::scale(1.0 / PIXEL_UM, 1.0 / PIXEL_UM),
        |record| record.affine(),
    );
    let reader = DatasetReader::open(&dataset)?;
    let first = reader.frames().first().ok_or("the dataset has no frames")?;
    let reference = first.sidecar.key;
    let (source, frames) = MappedTiles::open(&reader, |key| {
        (key.t, key.c, key.z) == (reference.t, reference.c, reference.z)
    })?;
    let mosaic = Mosaic::place(&frames, &l)?;

    let options = PairOptions::default();
    let found = mosaic.overlaps(options.min_overlap_px, options.min_overlap_share);
    let began = Instant::now();
    let threads = rayon::current_num_threads();
    let edges = pairs::register(&source, &mosaic, &found, None, &options, threads)?;
    let registered = began.elapsed();
    let began = Instant::now();
    let solution = solve::solve(&mosaic, &edges, &SolveOptions::default());
    let solved = began.elapsed();
    let trusted = edges.iter().filter(|edge| edge.confident).count();
    let rejected = solution.edges.iter().filter(|r| !r.used).count() - (edges.len() - trusted);
    println!(
        "{} tiles: {} pairs registered in {registered:.2?}, {trusted} trusted; solved in {solved:.2?} ({:?}, {} solves), {rejected} rejected, {} component(s)",
        mosaic.tiles.len(),
        edges.len(),
        solution.method,
        solution.solves,
        solution.components
    );
    println!(
        "the edges used against the solution: {:.3} px RMS, {:.3} px at worst",
        solution.rms_px, solution.max_px
    );

    // Against the truth, where there is one, once the means agree.
    let Ok(text) = std::fs::read_to_string(dataset.path().join("truth/tilescan.json")) else {
        return Ok(());
    };
    let truth: Value = serde_json::from_str(&text)?;
    let pixel = truth["pixel_size"]
        .as_f64()
        .ok_or("the truth's pixel size")?;
    let corners: Vec<[f64; 2]> = (0..mosaic.tiles.len())
        .map(|p| {
            let at = |which: &str, axis: usize| {
                truth["tiles"][p][which][axis].as_f64().unwrap_or(0.0) / pixel
            };
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
    let stage: Vec<[f64; 2]> = mosaic.tiles.iter().map(|tile| tile.nominal).collect();
    let ((rms, worst), (stage_rms, stage_worst)) =
        (misplaced(&solution.positions), misplaced(&stage));
    println!(
        "placed against the truth: {rms:.3} px RMS, {worst:.3} px at worst; the stage's alone, {stage_rms:.1} px RMS, {stage_worst:.1} px at worst"
    );
    Ok(())
}
