//! A synthetic two-channel dataset's image exported as OME-TIFF, through the scheduler: its
//! pyramid job, then its OME-TIFF export, as the service runs them.
//!
//! ```text
//! cargo run --release -p siq-jobs --example export_ometiff [-- --out <folder>]
//! ```
//!
//! The image is 12 000 × 8000, two channels (96 MP each). The dataset goes in a temporary data
//! root removed at the end, or under `<folder>` with `--out`: open `exports/synthetic.ome.tif` in
//! Fiji (Bio-Formats) or QuPath. Smoke mode (`SIQ_SMOKE=1`) uses 1024 × 768.

use std::error::Error;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use rayon::prelude::*;
use siq_jobs::kinds::{OmeTiffExportJob, PyramidJob};
use siq_jobs::{JobState, Priority, Scheduler, SchedulerConfig};
use siq_storage::ometiff::TiffCompression;
use siq_storage::zarr::{CHUNK, ChannelInfo, ImageInfo, SampleType, Size5, ZarrImage};
use siq_storage::{Dataset, DatasetKind, NewDataset};
use siq_types::OwnedImage;

fn main() -> Result<(), Box<dyn Error>> {
    let (width, height) = if std::env::var_os("SIQ_SMOKE").is_some() {
        (1024, 768)
    } else {
        (12_000, 8000)
    };
    let args: Vec<String> = std::env::args().collect();
    let out = args
        .iter()
        .position(|arg| arg == "--out")
        .and_then(|at| args.get(at + 1))
        .map(PathBuf::from);
    let temporary = tempfile::tempdir()?;
    let root = out.as_deref().unwrap_or(temporary.path());

    let dataset = Dataset::create(root, NewDataset::new(DatasetKind::Synthetic, "synthetic"))?;
    let path = dataset.zarr_dir().join("synthetic.ome.zarr");
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
            c: 2,
            z: 1,
            y: height,
            x: width,
        },
        levels: 1,
        chunk: CHUNK,
        pixel_size: [1.0, 0.376, 0.376],
        origin: [0.0; 3],
        channels: vec![
            channel("rings", [0, 255, 0]),
            channel("grid", [255, 0, 255]),
        ],
        cfa: None,
    };
    let image = ZarrImage::create(&path, info)?;
    let (rows, cols) = image.info().chunks(0);
    (0..2 * rows * cols).into_par_iter().try_for_each(|k| {
        let (c, row, col) = (k / (rows * cols), k / cols % rows, k % cols);
        let (x0, y0) = (col * CHUNK, row * CHUNK);
        let (w, h) = (CHUNK.min(width - x0), CHUNK.min(height - y0));
        let chunk = OwnedImage::from_fn(w as usize, h as usize, |x, y| {
            let (x, y) = (x0 + x as u64, y0 + y as u64);
            if c == 0 {
                let (fx, fy) = (
                    x as f32 - width as f32 / 2.0,
                    y as f32 - height as f32 / 2.0,
                );
                (8000.0 + 8000.0 * ((fx * fx + fy * fy).sqrt() / 40.0).cos()) as u16
            } else if x % 500 < 6 || y % 500 < 6 {
                12_000
            } else {
                500
            }
        });
        image.write_chunk(0, (0, c, 0), (row, col), chunk.view())
    })?;
    drop(image);

    let jobs = Scheduler::new(SchedulerConfig::new(14_000_000_000, 0), None)?;
    let folder = Some(dataset.path().to_owned());
    let started = Instant::now();
    let pyramid = jobs.submit(
        Box::new(PyramidJob::new(&path)),
        Priority::Interactive,
        folder.clone(),
    );
    let built = jobs
        .wait(pyramid, Duration::from_secs(600))
        .ok_or("the pyramid took too long")?;
    let pyramid_time = started.elapsed();
    let file = dataset.exports_dir().join("synthetic.ome.tif");
    let export = OmeTiffExportJob {
        image: path.clone(),
        out: file.clone(),
        compression: TiffCompression::Deflate,
    };
    let started = Instant::now();
    let exported = jobs.submit(Box::new(export), Priority::Background, folder);
    let exported = jobs
        .wait(exported, Duration::from_secs(600))
        .ok_or("the export took too long")?;
    let export_time = started.elapsed();
    if built.state != JobState::Done || exported.state != JobState::Done {
        return Err(format!("{:?} and {:?}", built.error, exported.error).into());
    }

    let levels = ZarrImage::open(&path)?.info().levels;
    println!(
        "{width} × {height} × 2 channels: the pyramid in {:.2} s",
        pyramid_time.as_secs_f64()
    );
    println!(
        "the OME-TIFF export: {levels} levels, {:.1} MB deflated (of {:.1} MB of pixels at level 0), \
         in {:.2} s",
        std::fs::metadata(&file)?.len() as f64 / 1e6,
        (width * height * 2 * 2) as f64 / 1e6,
        export_time.as_secs_f64()
    );
    let history = Dataset::open(dataset.path())?.manifest().history.clone();
    for record in history {
        println!(
            "recorded: {} {:?} → {:?}",
            record.kind, record.state, record.outputs
        );
    }
    if out.is_some() {
        println!("kept: {}", file.display());
    }
    Ok(())
}
