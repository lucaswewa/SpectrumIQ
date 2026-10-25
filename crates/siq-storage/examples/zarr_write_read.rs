//! Writes a synthetic 3-channel OME-Zarr image, its chunks in parallel, then reads a region back
//! and checks it.
//!
//! ```text
//! cargo run --release -p siq-storage --example zarr_write_read [-- --out <folder>]
//! ```
//!
//! The image is 6144 × 4096 per channel, 151 MB of u16. It goes in a temporary folder removed at
//! the end, or in `<folder>/synthetic.ome.zarr` with `--out`, to open in napari:
//! `napari --plugin napari-ome-zarr <folder>/synthetic.ome.zarr`. Smoke mode (`SIQ_SMOKE=1`) uses
//! 768 × 512.

use std::error::Error;
use std::path::PathBuf;
use std::time::Instant;

use rayon::prelude::*;
use siq_storage::zarr::{CHUNK, ChannelInfo, ImageInfo, SampleType, Size5, ZarrImage};
use siq_types::{OwnedImage, RectPx};

/// Channel `c`'s pixel at (`x`, `y`): rings, a gradient, and a checkerboard, each 14-bit.
fn pixel(c: u64, x: u64, y: u64, (width, height): (u64, u64)) -> u16 {
    let (fx, fy) = (x as f64 / width as f64, y as f64 / height as f64);
    let value = match c {
        0 => {
            let r = ((fx - 0.5).powi(2) + (fy - 0.5).powi(2)).sqrt();
            0.5 + 0.5 * (r * 80.0).cos()
        }
        1 => (fx + fy) / 2.0,
        _ => f64::from(u8::from((x / 256 + y / 256).is_multiple_of(2))),
    };
    (value * 16_000.0) as u16
}

fn main() -> Result<(), Box<dyn Error>> {
    let (width, height) = if std::env::var_os("SIQ_SMOKE").is_some() {
        (768, 512)
    } else {
        (6144, 4096)
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
        .join("synthetic.ome.zarr");
    if path.exists() {
        std::fs::remove_dir_all(&path)?;
    }

    let channel = |label: &str, colour| ChannelInfo {
        label: label.to_owned(),
        colour,
        window: (0.0, 16_000.0),
    };
    let info = ImageInfo {
        name: "synthetic".to_owned(),
        sample: SampleType::U16,
        size: Size5 {
            t: 1,
            c: 3,
            z: 1,
            y: height,
            x: width,
        },
        levels: 1,
        chunk: CHUNK,
        pixel_size: [1.0, 0.376, 0.376],
        origin: [0.0; 3],
        channels: vec![
            channel("rings", [255, 0, 255]),
            channel("gradient", [0, 255, 0]),
            channel("checkers", [0, 128, 255]),
        ],
    };
    let image = ZarrImage::create(&path, info)?;
    let (rows, cols) = image.info().chunks(0);
    let chunks: Vec<(u64, u64, u64)> = (0..3)
        .flat_map(|c| (0..rows).flat_map(move |row| (0..cols).map(move |col| (c, row, col))))
        .collect();

    let started = Instant::now();
    chunks.par_iter().try_for_each(|&(c, row, col)| {
        let (y0, x0) = (row * CHUNK, col * CHUNK);
        let (h, w) = (CHUNK.min(height - y0), CHUNK.min(width - x0));
        let chunk = OwnedImage::from_fn(w as usize, h as usize, |x, y| {
            pixel(c, x0 + x as u64, y0 + y as u64, (width, height))
        });
        image.write_chunk(0, (0, c, 0), (row, col), chunk.view())
    })?;
    let written = started.elapsed();
    let bytes = 3 * width * height * 2;
    let on_disk: u64 = walk(&path)?;
    println!(
        "wrote {} chunks of 3 × {width} × {height} u16 in {:.2} s: {:.0} MB/s, {:.0} MB on disk \
         ({:.1}× smaller)",
        chunks.len(),
        written.as_secs_f64(),
        bytes as f64 / 1e6 / written.as_secs_f64(),
        on_disk as f64 / 1e6,
        bytes as f64 / on_disk as f64,
    );

    let image = ZarrImage::open(&path)?;
    let rect = RectPx::new(
        width as u32 / 3,
        height as u32 / 3,
        600.min(width as u32 / 2),
        400.min(height as u32 / 2),
    );
    let started = Instant::now();
    let region = image.read_region::<u16>(0, (0, 0, 0), rect)?;
    let read = started.elapsed();
    let view = region.view();
    let wrong = view
        .rows()
        .enumerate()
        .flat_map(|(y, row)| row.iter().enumerate().map(move |(x, &p)| (x, y, p)))
        .filter(|&(x, y, p)| {
            p != pixel(
                0,
                u64::from(rect.x) + x as u64,
                u64::from(rect.y) + y as u64,
                (width, height),
            )
        })
        .count();
    println!(
        "read back {} × {} at ({}, {}) of the rings in {:.1} ms: {wrong} pixels differ",
        rect.width,
        rect.height,
        rect.x,
        rect.y,
        read.as_secs_f64() * 1e3,
    );
    if out.is_some() {
        println!("kept in {}", path.display());
    }
    if wrong > 0 {
        return Err("the region read back isn't what was written".into());
    }
    Ok(())
}

/// The bytes of the files under `path`.
fn walk(path: &std::path::Path) -> std::io::Result<u64> {
    let mut total = 0;
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        total += if entry.file_type()?.is_dir() {
            walk(&entry.path())?
        } else {
            entry.metadata()?.len()
        };
    }
    Ok(total)
}
