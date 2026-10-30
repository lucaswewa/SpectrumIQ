//! Builds the pyramid of a synthetic 20 000 × 20 000 OME-Zarr image (400 MP, 800 MB of u16): its
//! level 0 written chunk by chunk, then levels 1 to 7, out of core. Prints the time and the
//! process's peak memory.
//!
//! ```text
//! cargo run --release -p siq-pyramid --example build_pyramid [-- --out <folder>]
//! ```
//!
//! The image goes in a temporary folder removed at the end, or in `<folder>/pyramid.ome.zarr` with
//! `--out`. Smoke mode (`SIQ_SMOKE=1`) uses 2000 × 2000.

use std::error::Error;
use std::path::PathBuf;
use std::time::Instant;

use rayon::prelude::*;
use siq_pyramid::{TOP, build};
use siq_storage::zarr::{CHUNK, ChannelInfo, ImageInfo, SampleType, Size5, ZarrImage};
use siq_types::{OwnedImage, RectPx};

/// A pixel of the synthetic specimen: rings of cells on a slow gradient, in 14 bits.
fn pixel(x: u64, y: u64) -> u16 {
    let (fx, fy) = (x as f32 / 97.0, y as f32 / 89.0);
    let cells = (fx.sin() * fy.cos()).abs();
    let gradient = (x + y) as f32 / 40_000.0;
    (2000.0 + 9000.0 * cells + 4000.0 * gradient) as u16
}

fn megabytes(bytes: usize) -> f64 {
    bytes as f64 / 1e6
}

fn main() -> Result<(), Box<dyn Error>> {
    let side: u64 = if std::env::var_os("SIQ_SMOKE").is_some() {
        2000
    } else {
        20_000
    };
    let args: Vec<String> = std::env::args().collect();
    let out = args
        .iter()
        .position(|arg| arg == "--out")
        .and_then(|at| args.get(at + 1))
        .map(PathBuf::from);
    let temporary = tempfile::tempdir()?;
    let path = out
        .as_deref()
        .unwrap_or(temporary.path())
        .join("pyramid.ome.zarr");
    if path.exists() {
        std::fs::remove_dir_all(&path)?;
    }
    let info = ImageInfo {
        name: "synthetic mosaic".to_owned(),
        sample: SampleType::U16,
        size: Size5 {
            t: 1,
            c: 1,
            z: 1,
            y: side,
            x: side,
        },
        levels: 1,
        chunk: CHUNK,
        pixel_size: [1.0, 0.376, 0.376],
        origin: [0.0; 3],
        channels: vec![ChannelInfo {
            label: "cells".to_owned(),
            colour: [255, 255, 255],
            window: (2000.0, 14_000.0),
        }],
        cfa: None,
    };
    let mut image = ZarrImage::create(&path, info)?;
    let (rows, cols) = image.info().chunks(0);
    let started = Instant::now();
    (0..rows * cols).into_par_iter().try_for_each(|k| {
        let (row, col) = (k / cols, k % cols);
        let (x0, y0) = (col * CHUNK, row * CHUNK);
        let (w, h) = (CHUNK.min(side - x0), CHUNK.min(side - y0));
        let chunk = OwnedImage::from_fn(w as usize, h as usize, |x, y| {
            pixel(x0 + x as u64, y0 + y as u64)
        });
        image.write_chunk(0, (0, 0, 0), (row, col), chunk.view())
    })?;
    let written = started.elapsed();
    let before = siq_buffer::process_memory()?;
    println!(
        "level 0: {side} × {side}, {} chunks written in {:.1} s",
        rows * cols,
        written.as_secs_f64()
    );

    let started = Instant::now();
    build(&mut image, TOP, &())?;
    let built = started.elapsed();
    let after = siq_buffer::process_memory()?;
    let levels = image.info().levels;
    let top = image.info().size(levels - 1);
    println!(
        "levels 1 to {}, down to {} × {}, built in {:.1} s on {} threads",
        levels - 1,
        top.x,
        top.y,
        built.as_secs_f64(),
        rayon::current_num_threads(),
    );
    println!(
        "working set: {:.0} MB before the build, {:.0} MB after; the process's peak {:.0} MB, \
         against {:.0} MB of level-0 pixels",
        megabytes(before.working_set),
        megabytes(after.working_set),
        megabytes(after.peak_working_set),
        megabytes((side * side * 2) as usize),
    );

    // The top level's mean is level 0's, near enough.
    let top_level = image.read_region::<u16>(
        levels - 1,
        (0, 0, 0),
        RectPx::new(0, 0, top.x as u32, top.y as u32),
    )?;
    let mean =
        top_level.pixels.iter().map(|&p| f64::from(p)).sum::<f64>() / top_level.pixels.len() as f64;
    println!("the top level's mean: {mean:.0}");
    if out.is_some() {
        println!("kept in {}", path.display());
    }
    Ok(())
}
