//! The overlap graph and pairwise registration of a tile scan (ADR-0061): a `siq-datagen` scan, or
//! a dataset of yours, its tiles placed by their poses, every overlapping pair registered, and the
//! offsets set against the truth where the dataset has one.
//!
//! ```text
//! cargo run --release -p siq-stitch --example pairs_on_dataset [-- <dataset folder>]
//! ```
//!
//! Without a folder, it makes a scan of 5 × 4 tiles of 2048 × 1536 over cells at 15 % overlap, the
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
                (5, 4, 2048, 1536)
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
                seed: 1,
            };
            let began = Instant::now();
            let made = tilescan(&args, &out)?;
            println!(
                "made {columns} × {rows} tiles of {width} × {height} in {:.1?}",
                began.elapsed()
            );
            made
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

    // The reference channel and plane: the first frame's.
    let reader = DatasetReader::open(&dataset)?;
    let first = reader.frames().first().ok_or("the dataset has no frames")?;
    let reference = first.sidecar.key;
    let (source, frames) = MappedTiles::open(&reader, |key| {
        (key.t, key.c, key.z) == (reference.t, reference.c, reference.z)
    })?;
    let mosaic = Mosaic::place(&frames, &l)?;
    let options = PairOptions::default();
    let found = mosaic.overlaps(options.min_overlap_px, options.min_overlap_share);
    let threads = rayon::current_num_threads();
    let each = pairs::bytes_per_pair(&mosaic, &found, &options);
    let began = Instant::now();
    let edges = pairs::register(&source, &mosaic, &found, None, &options, threads)?;
    let trusted: Vec<_> = edges.iter().filter(|edge| edge.confident).collect();
    println!(
        "{} tiles, {} pairs registered in {:.2?} on {threads} threads, {:.0} MB each at most: {} trusted",
        mosaic.tiles.len(),
        edges.len(),
        began.elapsed(),
        each as f64 / 1e6,
        trusted.len()
    );
    for edge in edges.iter().filter(|edge| !edge.confident) {
        println!(
            "  not trusted: {} – {}, energy {:.0}, correlation {:.2}, over {} × {} pixels",
            edge.i, edge.j, edge.pce, edge.ncc, edge.overlap_px[0], edge.overlap_px[1]
        );
    }

    // Against the truth, where there is one: the offsets measured, and the stage's own.
    let Ok(text) = std::fs::read_to_string(dataset.path().join("truth/tilescan.json")) else {
        return Ok(());
    };
    let truth: Value = serde_json::from_str(&text)?;
    let pixel = truth["pixel_size"]
        .as_f64()
        .ok_or("the truth's pixel size")?;
    let offset = |i: usize, j: usize, which: &str| -> [f64; 2] {
        let at = |p: usize, axis: usize| truth["tiles"][p][which][axis].as_f64().unwrap_or(0.0);
        [0, 1].map(|axis| (at(j, axis) - at(i, axis)) / pixel)
    };
    let miss = |found: [f64; 2], i: usize, j: usize| {
        let [x, y] = offset(i, j, "true");
        (found[0] - x).hypot(found[1] - y)
    };
    let rms =
        |errors: &[f64]| (errors.iter().map(|e| e * e).sum::<f64>() / errors.len() as f64).sqrt();
    let measured: Vec<f64> = trusted.iter().map(|e| miss(e.shift, e.i, e.j)).collect();
    for (edge, &off) in trusted.iter().zip(&measured).filter(|&(_, &off)| off > 1.0) {
        println!(
            "  trusted but {off:.1} px off: {} – {}, energy {:.0}, correlation {:.2}, over {} × {} pixels",
            edge.i, edge.j, edge.pce, edge.ncc, edge.overlap_px[0], edge.overlap_px[1]
        );
    }
    let stage: Vec<f64> = edges
        .iter()
        .map(|e| miss(offset(e.i, e.j, "nominal"), e.i, e.j))
        .collect();
    if measured.is_empty() {
        return Err("no pair was trusted".into());
    }
    println!(
        "the trusted offsets against the truth: {:.3} px RMS, {:.3} px at worst; the stage's alone: {:.1} px RMS",
        rms(&measured),
        measured.iter().copied().fold(0.0, f64::max),
        rms(&stage)
    );
    Ok(())
}
