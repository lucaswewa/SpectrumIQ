//! A synthetic dataset's image exported as a static DZI pyramid, through the scheduler: its
//! pyramid job, then its DZI export, as the service runs them.
//!
//! ```text
//! cargo run --release -p siq-jobs --example export_dzi [-- --out <folder>]
//! ```
//!
//! The image is 8192 × 6144, one channel. The dataset goes in a temporary data root removed at the
//! end, or under `<folder>` with `--out`: open `exports/dzi/synthetic.dzi` in any DZI viewer. Smoke
//! mode (`SIQ_SMOKE=1`) uses 1024 × 768.

use std::error::Error;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use rayon::prelude::*;
use siq_jobs::kinds::{DziExportJob, PyramidJob};
use siq_jobs::{JobState, Priority, Scheduler, SchedulerConfig};
use siq_pyramid::tiles::TileFormat;
use siq_storage::zarr::{CHUNK, ChannelInfo, ImageInfo, SampleType, Size5, ZarrImage};
use siq_storage::{Dataset, DatasetKind, NewDataset};
use siq_types::OwnedImage;

fn main() -> Result<(), Box<dyn Error>> {
    let (width, height) = if std::env::var_os("SIQ_SMOKE").is_some() {
        (1024, 768)
    } else {
        (8192, 6144)
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
    let info = ImageInfo {
        name: "synthetic".to_owned(),
        sample: SampleType::U16,
        size: Size5 {
            t: 1,
            c: 1,
            z: 1,
            y: height,
            x: width,
        },
        levels: 1,
        chunk: CHUNK,
        pixel_size: [1.0, 0.376, 0.376],
        origin: [0.0; 3],
        channels: vec![ChannelInfo {
            label: "rings".to_owned(),
            colour: [255, 255, 255],
            window: (0.0, 16_000.0),
        }],
    };
    let image = ZarrImage::create(&path, info)?;
    let (rows, cols) = image.info().chunks(0);
    (0..rows * cols).into_par_iter().try_for_each(|k| {
        let (row, col) = (k / cols, k % cols);
        let (x0, y0) = (col * CHUNK, row * CHUNK);
        let (w, h) = (CHUNK.min(width - x0), CHUNK.min(height - y0));
        let chunk = OwnedImage::from_fn(w as usize, h as usize, |x, y| {
            let (fx, fy) = (
                (x0 + x as u64) as f32 - width as f32 / 2.0,
                (y0 + y as u64) as f32 - height as f32 / 2.0,
            );
            (8000.0 + 8000.0 * ((fx * fx + fy * fy).sqrt() / 40.0).cos()) as u16
        });
        image.write_chunk(0, (0, 0, 0), (row, col), chunk.view())
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
    let export = DziExportJob {
        image: path.clone(),
        plane: (0, 0, 0),
        lut: None,
        format: TileFormat::Jpeg,
        out: dataset.exports_dir().join("dzi"),
        name: "synthetic".to_owned(),
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

    let files = dataset.exports_dir().join("dzi").join("synthetic_files");
    let (mut tiles, mut bytes, mut levels) = (0, 0, 0);
    for level in std::fs::read_dir(&files)? {
        levels += 1;
        for tile in std::fs::read_dir(level?.path())? {
            tiles += 1;
            bytes += tile?.metadata()?.len();
        }
    }
    println!(
        "{width} × {height}: the pyramid in {:.2} s",
        pyramid_time.as_secs_f64()
    );
    println!(
        "the DZI export: {levels} levels, {tiles} tiles, {:.1} MB of JPEG, in {:.2} s",
        bytes as f64 / 1e6,
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
        println!("kept in {}", dataset.path().display());
    }
    Ok(())
}
