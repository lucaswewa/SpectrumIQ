//! The 500 MP benchmark (P8.5): `siq-datagen`'s tile scan of 8 × 7 tiles of 4096 × 3072 over
//! cells, 15 % overlapping, the stage off by 5 µm a standard deviation; its flat estimated from its
//! frames; then stitched as the service stitches it, by the stitch job within the service's 14 GB
//! budget. It reports each stage's wall time, processors' time, peak working set and private
//! bytes, and bytes written, and the tiles' placement against the truth, and fails outside the
//! targets.
//!
//! ```text
//! cargo run --release -p siq-jobs --example bench_stitch_500mp [-- --out <folder>] [--world grains]
//! ```
//!
//! The scan goes in a temporary data root removed at the end, or under `<folder>` with `--out`.
//! Smoke mode (`SIQ_SMOKE=1`) uses 3 × 2 tiles of 512 × 384, and sets no targets.

use std::error::Error;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use siq_buffer::{process_memory, process_usage};
use siq_calib::shading::Estimator;
use siq_datagen::tilescan::{TileScanArgs, tilescan};
use siq_jobs::kinds::StitchJob;
use siq_jobs::{JobInfo, JobState, Priority, Scheduler, SchedulerConfig};
use siq_stitch::{MappedTiles, Mosaic, Report, StitchOptions, TileSource};
use siq_storage::{Dataset, DatasetReader};
use siq_types::{Affine2, Roi};

/// The simulator's 10× pixel, in µm, and its sensor's dark level.
const PIXEL_UM: f64 = 0.376;
const DARK: f32 = 100.0;

/// The service's budget for its jobs' memory.
const BUDGET: u64 = 14_000_000_000;

/// The targets (ADR-0064, amended), from the first measurement on the 16-thread workstation, 7.6 s
/// and 1.24 GB, with room for a slower disk or processors: the flat and the stitch within 30 s,
/// against the brief's 3 minutes; the stitch's working set within 3 GB, mapped tiles included,
/// against the budget's 14; and the tiles within 0.3 px RMS and 1 px of the truth.
const END_TO_END: Duration = Duration::from_secs(30);
const PEAK_RAM: u64 = 3_000_000_000;

/// The stitch job's stages, each ended once the job says so: the pairs at a quarter of its
/// progress, the solve at 0.3, the composite when the pyramid begins, and the pyramid with the job.
const STAGES: [&str; 4] = ["pairs", "solve", "composite", "pyramid"];

/// Whether stage `k` is over, by `info`.
fn ended(k: usize, info: &JobInfo) -> bool {
    let message = info.message.as_deref().unwrap_or_default();
    info.state.is_finished()
        || match k {
            0 => info.progress >= 0.25,
            1 => info.progress >= 0.3,
            2 => message.starts_with("building the pyramid") || info.progress > 0.87,
            _ => false,
        }
}

/// The process's counters at a moment.
#[derive(Clone, Copy)]
struct Mark {
    at: Instant,
    cpu: Duration,
    written: u64,
}

impl Mark {
    fn now() -> Result<Self, Box<dyn Error>> {
        let usage = process_usage()?;
        Ok(Self {
            at: Instant::now(),
            cpu: usage.cpu,
            written: usage.written_bytes,
        })
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let smoke = std::env::var_os("SIQ_SMOKE").is_some();
    let args: Vec<String> = std::env::args().collect();
    let after = |flag: &str| {
        let at = args.iter().position(|arg| arg == flag)?;
        args.get(at + 1).cloned()
    };
    let temporary = tempfile::tempdir()?;
    let root = after("--out").map_or_else(|| temporary.path().to_owned(), PathBuf::from);
    let ((columns, rows), (width, height)) = match smoke {
        true => ((3, 2), (512, 384)),
        false => ((8, 7), (4096, 3072)),
    };
    let scan = TileScanArgs {
        world: after("--world").unwrap_or_else(|| "cells".to_owned()),
        columns,
        rows,
        width,
        height,
        overlap: 0.15,
        stage_error: 5.0,
        light: 1.0,
        no_mosaic: true,
    };
    let output = siq_datagen::Output {
        data_root: root,
        smoke: false,
        seed: 500,
    };
    let began = Instant::now();
    let path = tilescan(&scan, &output)?;
    let raw = u64::from(columns * rows) * u64::from(width * height) * 2;
    println!(
        "generated {columns} × {rows} tiles of {width} × {height} over {}: {:.2} GB in {:.1?}",
        scan.world,
        raw as f64 / 1e9,
        began.elapsed()
    );

    // The flat, from the frames, as the shading job estimates it.
    let first = Mark::now()?;
    let dataset = Dataset::open(&path)?;
    let reader = DatasetReader::open(&dataset)?;
    let (source, frames) = MappedTiles::open(&reader, |_| true)?;
    let mut estimator = Estimator::new(DARK);
    for k in 0..frames.len() {
        estimator.add(&source.pixels(k)?.view()?)?;
    }
    let flat = estimator.finish(Roi::full(width, height), [width, height])?;
    drop(source);
    let flattened = Mark::now()?;

    // Stitched by the job, its stages told by its progress.
    let l = Affine2::scale(1.0 / PIXEL_UM, 1.0 / PIXEL_UM);
    let job = StitchJob {
        dataset: path.clone(),
        image: "stitched".to_owned(),
        l,
        channels: Vec::new(),
        shading: vec![(0, DARK, flat)],
        options: StitchOptions::default(),
    };
    let config = SchedulerConfig::new(BUDGET, 0);
    let threads = config.threads;
    let scheduler = Scheduler::new(config, None)?;
    let id = scheduler.submit(Box::new(job), Priority::Interactive, Some(path.clone()));
    let mut marks = vec![flattened];
    let mut peaks = vec![[0usize; 2]; STAGES.len()];
    let info = loop {
        let info = scheduler.info(id).ok_or("the job is gone")?;
        let memory = process_memory()?;
        // Each stage over ends now.
        while let Some(peak) = peaks.get_mut(marks.len() - 1) {
            *peak = [peak[0].max(memory.working_set), peak[1].max(memory.private)];
            if !ended(marks.len() - 1, &info) {
                break;
            }
            marks.push(Mark::now()?);
        }
        if info.state.is_finished() {
            break info;
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    if info.state != JobState::Done {
        return Err(format!("the stitch {:?}: {:?}", info.state, info.error).into());
    }
    let last = *marks.last().ok_or("no marks")?;
    let peak = peaks
        .iter()
        .fold([0, 0], |most, at| [most[0].max(at[0]), most[1].max(at[1])]);

    // What was made, and how well the tiles were placed: each corner against the truth's.
    let text = std::fs::read_to_string(dataset.qa_dir().join("stitched.json"))?;
    let report: Report = serde_json::from_str(&text)?;
    let text = std::fs::read_to_string(dataset.path().join("truth/tilescan.json"))?;
    let truth: serde_json::Value = serde_json::from_str(&text)?;
    let mosaic = Mosaic::place(&frames, &l)?;
    let errors: Vec<[f64; 2]> = report
        .tiles
        .iter()
        .filter_map(|placed| {
            let tile = mosaic.tiles.iter().find(|tile| tile.key.p == placed.p)?;
            let at = &truth["tiles"][placed.p as usize];
            let moved = |axis: usize| {
                let of = |key: &str| at[key][axis].as_f64().unwrap_or(f64::NAN);
                (of("true") - of("nominal")) / PIXEL_UM
            };
            Some([0, 1].map(|axis| placed.corner[axis] - tile.nominal[axis] - moved(axis)))
        })
        .collect();
    let n = errors.len() as f64;
    let mean = [0, 1].map(|axis| errors.iter().map(|e| e[axis]).sum::<f64>() / n);
    let off: Vec<f64> = errors
        .iter()
        .map(|e| (e[0] - mean[0]).hypot(e[1] - mean[1]))
        .collect();
    let rms = (off.iter().map(|e| e * e).sum::<f64>() / n).sqrt();
    let worst = off.iter().copied().fold(0.0, f64::max);
    let summary = report.summary;
    let [w, h] = report.bounds.size;
    println!(
        "stitched into {w} × {h} pixels ({:.0} MP) on {threads} threads; {} pairs, {} trusted, {} rejected; the edges at {:.3} px RMS",
        (w * h) as f64 / 1e6,
        summary.pairs,
        summary.trusted,
        summary.rejected,
        summary.rms_px
    );
    let gb = |bytes: usize| bytes as f64 / 1e9;
    println!("  stage         wall    processors' time    peak working set, private   written");
    let row = |name: &str, from: Mark, to: Mark, peak: Option<[usize; 2]>| {
        let wall = to.at - from.at;
        let cpu = (to.cpu - from.cpu).as_secs_f64();
        let peak = peak.map_or_else(String::new, |[working, private]| {
            format!("{:.2} GB, {:.2} GB", gb(working), gb(private))
        });
        println!(
            "  {name:10} {:7.2} s  {cpu:7.1} s ({:3.0} % of {threads})  {peak:>22}  {:7.2} GB",
            wall.as_secs_f64(),
            100.0 * cpu / wall.as_secs_f64().max(1e-9) / threads as f64,
            (to.written - from.written) as f64 / 1e9
        );
    };
    row("flat", first, flattened, None);
    for (k, name) in STAGES.iter().enumerate() {
        row(name, marks[k], marks[k + 1], Some(peaks[k]));
    }
    row("end to end", first, last, Some(peak));
    println!(
        "the process's working set at its peak, the generator's included: {:.2} GB; the tiles, {:.2} GB, read mapped",
        gb(process_memory()?.peak_working_set),
        raw as f64 / 1e9
    );
    println!("placed within {rms:.3} px RMS and {worst:.3} px at worst of the truth");
    if !smoke {
        let elapsed = last.at - first.at;
        let within =
            elapsed <= END_TO_END && peak[0] as u64 <= PEAK_RAM && rms <= 0.3 && worst <= 1.0;
        let peak_gb = gb(peak[0]);
        if !within {
            let failed = format!("outside the targets: {elapsed:.1?} end to end, {peak_gb:.2} GB");
            return Err(failed.into());
        }
        println!(
            "within the targets: {elapsed:.1?} end to end of {END_TO_END:.0?}, {peak_gb:.2} GB of {:.0}",
            PEAK_RAM as f64 / 1e9
        );
    }
    Ok(())
}
