//! A tile scan stitched end to end (ADR-0061 to ADR-0063): a `siq-datagen` scan, or a dataset of
//! yours, its pairs registered and solved, its flat estimated from its own frames, and the mosaic
//! composited into OME-Zarr out of core with its pyramid; then set against the ideal mosaic where
//! the dataset has one.
//!
//! ```text
//! cargo run --release -p siq-stitch --example composite_dataset [-- <dataset folder>]
//! ```
//!
//! Without a folder, it makes a scan of 4 × 3 tiles of 2048 × 1536 over cells at 15 % overlap, the
//! stage off by 5 µm a standard deviation, with its ideal mosaic, in a temporary folder. The mosaic
//! is written as the dataset's `zarr/stitched.ome.zarr`. The tiles are placed through the
//! dataset's calibration, or at 0.376 µm a pixel (the simulator's 10×) without one, from its first
//! channel and plane; the dark is 100. Smoke mode (`SIQ_SMOKE=1`) makes 3 × 2 tiles of 512 × 384.

use std::error::Error;
use std::path::PathBuf;
use std::time::Instant;

use siq_calib::CalibrationRecord;
use siq_calib::shading::Estimator;
use siq_datagen::Output;
use siq_datagen::tilescan::{TileScanArgs, tilescan};
use siq_stitch::composite::{self, Bounds, CompositeOptions, Placed, Plane};
use siq_stitch::pairs::{self, PairOptions};
use siq_stitch::solve::{self, SolveOptions};
use siq_stitch::{MappedTiles, Mosaic, TileSource};
use siq_storage::zarr::{CHUNK, ChannelInfo, ImageInfo, SampleType, Size5, ZarrImage};
use siq_storage::{Dataset, DatasetReader};
use siq_types::{Affine2, RectPx, Roi};

/// The simulator's 10× pixel, in µm, and its sensor's dark level.
const PIXEL_UM: f64 = 0.376;
const DARK: f32 = 100.0;

fn main() -> Result<(), Box<dyn Error>> {
    let smoke = std::env::var_os("SIQ_SMOKE").is_some();
    let temporary = tempfile::tempdir()?;
    let path = match std::env::args().nth(1) {
        Some(folder) => PathBuf::from(folder),
        None => {
            let (columns, rows, width, height) = if smoke {
                (3, 2, 512, 384)
            } else {
                (4, 3, 2048, 1536)
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
                no_mosaic: false,
            };
            let out = Output {
                data_root: temporary.path().to_owned(),
                smoke: false,
                seed: 3,
            };
            println!("making {columns} × {rows} tiles of {width} × {height}, and the ideal mosaic");
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
    let reference = reader
        .frames()
        .first()
        .ok_or("the dataset has no frames")?
        .sidecar
        .key;
    let (source, frames) = MappedTiles::open(&reader, |key| {
        (key.t, key.c, key.z) == (reference.t, reference.c, reference.z)
    })?;
    let threads = rayon::current_num_threads();
    let source = source.with_capacity(4 * threads);
    let mosaic = Mosaic::place(&frames, &l)?;

    // Registered and solved.
    let began = Instant::now();
    let options = PairOptions::default();
    let found = mosaic.overlaps(options.min_overlap_px, options.min_overlap_share);
    let edges = pairs::register(&source, &mosaic, &found, None, &options, threads)?;
    let solution = solve::solve(&mosaic, &edges, &SolveOptions::default());
    let solved = began.elapsed();

    // The flat, from the frames themselves.
    let began = Instant::now();
    let mut estimator = Estimator::new(DARK);
    for k in 0..mosaic.tiles.len() {
        estimator.add(&source.pixels(k)?.view()?)?;
    }
    let [width, height] = mosaic.tiles[0].size;
    let flat = estimator.finish(Roi::full(width, height), [width, height])?;
    let flattened = began.elapsed();

    // Composited, then its pyramid.
    let placed: Vec<Placed> = (0..mosaic.tiles.len())
        .map(|k| Placed {
            source: k,
            tile: mosaic.tiles[k],
            corner: solution.positions[k],
        })
        .collect();
    let sizes: Vec<[u32; 2]> = mosaic.tiles.iter().map(|tile| tile.size).collect();
    let bounds = Bounds::of(&solution.positions, &sizes);
    let info = ImageInfo {
        name: "stitched".to_owned(),
        sample: SampleType::U16,
        size: Size5 {
            t: 1,
            c: 1,
            z: 1,
            y: bounds.size[1],
            x: bounds.size[0],
        },
        levels: 1,
        chunk: CHUNK,
        pixel_size: [1.0, 1.0 / mosaic.px_per_um, 1.0 / mosaic.px_per_um],
        origin: [0.0; 3],
        channels: vec![ChannelInfo {
            label: "stitched".to_owned(),
            colour: [255; 3],
            window: (0.0, 16_383.0),
        }],
        cfa: None,
    };
    let mut image = ZarrImage::create(&dataset.zarr_dir().join("stitched.ome.zarr"), info)?;
    let plane = Plane {
        at: (0, 0, 0),
        tiles: placed,
    };
    let options = CompositeOptions::default();
    let began = Instant::now();
    let shading = Some((DARK, &flat));
    composite::composite(&source, &[plane], &bounds, &image, shading, &options, &())?;
    let composited = began.elapsed();
    let began = Instant::now();
    siq_pyramid::build(&mut image, siq_pyramid::TOP, &())?;
    let built = began.elapsed();
    let megapixels = (bounds.size[0] * bounds.size[1]) as f64 / 1e6;
    println!(
        "{} tiles, {} pairs, {} trusted: registered and solved in {solved:.2?} ({:.3} px RMS of the edges)",
        mosaic.tiles.len(),
        edges.len(),
        edges.iter().filter(|edge| edge.confident).count(),
        solution.rms_px
    );
    println!(
        "the flat estimated in {flattened:.2?}; {} × {} pixels ({megapixels:.1} MP) composited in {composited:.2?} ({:.0} MP/s), {:.0} MB a block held at most; the pyramid's {} levels in {built:.2?}",
        bounds.size[0],
        bounds.size[1],
        megapixels / composited.as_secs_f64(),
        composite::bytes(CHUNK, &options, threads) as f64 / threads as f64 / 1e6,
        image.info().levels
    );

    // Against the ideal mosaic, where there is one: its pixel `i` is the mosaic's `i − size / 2`,
    // once the solution's mean offset from the truth is taken out, as the stage's mean error.
    let Ok(ideal) = ZarrImage::open(&dataset.zarr_dir().join("truth.ome.zarr")) else {
        return Ok(());
    };
    let text = std::fs::read_to_string(dataset.path().join("truth/tilescan.json"))?;
    let truth: serde_json::Value = serde_json::from_str(&text)?;
    let n = mosaic.tiles.len() as f64;
    let offset = [0, 1].map(|axis| {
        (0..mosaic.tiles.len())
            .map(|p| {
                let at =
                    |key: &str| truth["tiles"][p][key][axis].as_f64().unwrap_or(0.0) / PIXEL_UM;
                mosaic.tiles[p].nominal[axis] + at("true")
                    - at("nominal")
                    - solution.positions[p][axis]
            })
            .sum::<f64>()
            / n
    });
    let half = [i64::from(width / 2), i64::from(height / 2)];
    let ideal_size = [ideal.info().size.x as i64, ideal.info().size.y as i64];
    // The ideal's pixel is the composite's plus `k`.
    let k = [0, 1].map(|axis| bounds.origin[axis] + offset[axis].round() as i64 + half[axis]);
    let margin = 64i64;
    let first = [0, 1].map(|axis| margin.max(margin - k[axis]));
    let last = [0, 1]
        .map(|axis| (bounds.size[axis] as i64 - margin).min(ideal_size[axis] - margin - k[axis]));
    let size = [0, 1].map(|axis| (last[axis] - first[axis]).max(1) as u32);
    let rect = |at: [i64; 2]| RectPx::new(at[0] as u32, at[1] as u32, size[0], size[1]);
    let from = first;
    let there = [first[0] + k[0], first[1] + k[1]];
    let got = image.read_region::<u16>(0, (0, 0, 0), rect(from))?;
    let want = ideal.read_region::<u16>(0, (0, 0, 0), rect(there))?;
    let (got, want): (Vec<f64>, Vec<f64>) = (
        got.pixels.iter().map(|&v| f64::from(v)).collect(),
        want.pixels.iter().map(|&v| f64::from(v)).collect(),
    );
    let n = got.len() as f64;
    let (mg, mw) = (got.iter().sum::<f64>() / n, want.iter().sum::<f64>() / n);
    let (mut gw, mut ww) = (0.0, 0.0);
    for (g, w) in got.iter().zip(&want) {
        (gw, ww) = (gw + (g - mg) * (w - mw), ww + (w - mw) * (w - mw));
    }
    let a = gw / ww;
    let off = (got
        .iter()
        .zip(&want)
        .map(|(g, w)| (g - (a * (w - mw) + mg)).powi(2))
        .sum::<f64>()
        / n)
        .sqrt();
    let peak =
        got.iter().copied().fold(f64::MIN, f64::max) - got.iter().copied().fold(f64::MAX, f64::min);
    println!(
        "against the ideal mosaic: PSNR {:.1} dB, its optics' blur and noise aside",
        20.0 * (peak / off).log10()
    );
    Ok(())
}
