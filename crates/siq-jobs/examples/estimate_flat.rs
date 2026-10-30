//! A tile scan's flat estimated after the fact, as the service runs it: `siq-datagen`'s tile scan
//! of the cells world, every tile with the simulator's vignetting, through the shading job; then
//! the flat found against the vignetting the tiles were rendered with, and the seams before and
//! after.
//!
//! ```text
//! cargo run --release -p siq-jobs --example estimate_flat [-- --out <folder>]
//! ```
//!
//! The scan is 8 × 6 tiles of 2048 × 1536. It goes in a temporary data root removed at the end, or
//! under `<folder>` with `--out`, the flat with it under `settings/shading/`. Smoke mode
//! (`SIQ_SMOKE=1`) uses 3 × 2 tiles of 256 × 192.

use std::error::Error;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use siq_calib::shading::{Dark, FlatFieldView, ShadingKind, ShadingRegistry};
use siq_datagen::Camera;
use siq_datagen::tilescan::{TileScanArgs, tilescan};
use siq_jobs::kinds::ShadingJob;
use siq_jobs::{JobState, Priority, Scheduler, SchedulerConfig};
use siq_storage::{Dataset, DatasetReader};
use siq_types::{Binning, ImageView, OwnedImage};

/// The seam bands' light against the tiles' centres, less 1, over every tile: the bands are the
/// outer 15 % each way, where neighbours overlap, and the centre the middle third. Summed over
/// many tiles, the sample averages away and the shading stays.
fn seam_contrast<'a>(tiles: impl Iterator<Item = ImageView<'a, u16>>, dark: f64) -> f64 {
    let mut sums = [[0.0f64; 2]; 2];
    for tile in tiles {
        let (w, h) = (tile.width(), tile.height());
        let (bx, by) = (w * 15 / 100, h * 15 / 100);
        for (y, row) in tile.rows().enumerate() {
            for (x, &value) in row.iter().enumerate() {
                let band = x < bx || x >= w - bx || y < by || y >= h - by;
                let centre = (w / 3..2 * w / 3).contains(&x) && (h / 3..2 * h / 3).contains(&y);
                if band || centre {
                    let slot = &mut sums[usize::from(centre)];
                    (slot[0], slot[1]) = (slot[0] + f64::from(value) - dark, slot[1] + 1.0);
                }
            }
        }
    }
    let [band, centre] = sums.map(|[sum, count]| sum / count.max(1.0));
    (band / centre - 1.0).abs()
}

fn main() -> Result<(), Box<dyn Error>> {
    let smoke = std::env::var_os("SIQ_SMOKE").is_some();
    let args: Vec<String> = std::env::args().collect();
    let out = args
        .iter()
        .position(|arg| arg == "--out")
        .and_then(|at| args.get(at + 1))
        .map(PathBuf::from);
    let temporary = tempfile::tempdir()?;
    let root = out.as_deref().unwrap_or(temporary.path()).to_owned();

    let ((columns, rows), (width, height)) = match smoke {
        true => ((3, 2), (256, 192)),
        false => ((8, 6), (2048, 1536)),
    };
    let scan = TileScanArgs {
        world: "cells".to_owned(),
        columns,
        rows,
        width,
        height,
        overlap: 0.15,
        stage_error: 2.0,
        light: 1.0,
        no_mosaic: true,
        colour: false,
    };
    let output = siq_datagen::Output {
        data_root: root.clone(),
        smoke: false,
        seed: 7,
    };
    let started = Instant::now();
    let folder = tilescan(&scan, &output)?;
    println!(
        "{} tiles of {width} × {height}, rendered in {:.1} s",
        columns * rows,
        started.elapsed().as_secs_f64()
    );

    let camera = Camera::new((width, height), output.seed);
    let dark = camera.noise.dark_offset;
    let registry = Arc::new(ShadingRegistry::open(&root.join("settings"))?);
    let job = ShadingJob {
        registry: Arc::clone(&registry),
        key: (None, None),
        dark,
        sensor: [width, height],
    };
    let jobs = Scheduler::new(SchedulerConfig::new(14_000_000_000, 0), None)?;
    let started = Instant::now();
    let id = jobs.submit(Box::new(job), Priority::Background, Some(folder.clone()));
    let done = jobs
        .wait(id, Duration::from_secs(600))
        .ok_or("the estimate took too long")?;
    if done.state != JobState::Done {
        return Err(format!("{:?}", done.error).into());
    }
    let record = registry
        .current((None, None, ShadingKind::Flat))
        .ok_or("no flat was kept")?;
    let flat = record.flat.as_ref().ok_or("the record holds no flat")?;
    println!(
        "estimated in {:.2} s, from {} frames: {:.3} to {:.3} of its mean",
        started.elapsed().as_secs_f64(),
        record.frames,
        record.range.map_or(0.0, |[low, _]| low),
        record.range.map_or(0.0, |[_, high]| high)
    );

    // Against the vignetting, relative to its mean over the sensor, on a 17 × 13 grid.
    let (cx, cy) = (f64::from(width) / 2.0, f64::from(height) / 2.0);
    let truth = |x: f64, y: f64| {
        let rho2 = ((x - cx).powi(2) + (y - cy).powi(2)) / (cx * cx + cy * cy);
        camera.optics.vignetting.at(rho2)
    };
    let places: Vec<_> = (0..=12)
        .flat_map(|j| (0..=16).map(move |i| (f64::from(i) / 16.0, f64::from(j) / 12.0)))
        .map(|(u, v)| (u * f64::from(width), v * f64::from(height)))
        .collect();
    let mean = (0..height)
        .step_by(4)
        .flat_map(|y| {
            (0..width)
                .step_by(4)
                .map(move |x| (f64::from(x) + 2.0, f64::from(y) + 2.0))
        })
        .map(|(x, y)| (truth(x, y), 1.0))
        .fold((0.0, 0.0), |(s, n), (t, one)| (s + t, n + one));
    let mean = mean.0 / mean.1;
    let worst = places
        .iter()
        .map(|&(x, y)| (flat.at(x, y) / (truth(x, y) / mean) - 1.0).abs())
        .fold(0.0, f64::max);
    println!(
        "the flat against the vignetting: {:.2} % at worst",
        worst * 100.0
    );

    // The seams, before and after.
    let dataset = Dataset::open(&folder)?;
    let reader = DatasetReader::open(&dataset)?;
    let mapped = reader
        .frames()
        .iter()
        .map(|frame| frame.map())
        .collect::<Result<Vec<_>, _>>()?;
    let raw: Vec<_> = mapped
        .iter()
        .filter_map(|frame| frame.image_u16())
        .collect();
    let before = seam_contrast(raw.iter().copied(), dark);
    let level = OwnedImage::from_fn(width as usize, height as usize, |_, _| dark.round() as u16);
    let dark = Dark {
        image: level,
        binning: Binning::NONE,
    };
    let view = FlatFieldView::new(mapped[0].meta(), Some(&dark), Some(flat));
    let corrected = raw
        .iter()
        .map(|tile| view.read(tile, 16383))
        .collect::<Result<Vec<_>, _>>()?;
    let after = seam_contrast(corrected.iter().map(OwnedImage::view), 0.0);
    println!(
        "the seams: {:.2} % before, {:.3} % after, {:.0}× fainter",
        before * 100.0,
        after * 100.0,
        before / after.max(1e-9)
    );
    if out.is_some() {
        println!("kept in {}", folder.display());
    }
    Ok(())
}
