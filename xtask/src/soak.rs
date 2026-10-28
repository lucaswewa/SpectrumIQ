//! The soak (ADR-0069): viewers and an operator against a service of the soak's own, for a while,
//! and what they saw.
//!
//! It builds `spectrumiq-server` in release, and runs it on a copy of `config/sim.toml` in
//! `target/soak/`, with an admin, who operates, and a viewer added. Then, until the time is up:
//!
//! - the operator holds the control token, and runs 2 × 2 tile scans, deleting each one's dataset
//!   once it is done, with 5 s of live view between them;
//! - each viewer follows the overview and ROI streams, observes the camera and the stage over
//!   their sockets, and pans about a snap's deep-zoom tiles;
//! - every 5 s, the service's memory, CPU time and metrics are sampled.
//!
//! The report, also in `target/soak/report.json`, holds them against ADR-0069's targets.

mod client;
#[cfg(test)]
mod tests;

use std::fmt;
use std::fs::{self, File};
use std::io::Write as _;
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread::{self, ScopedJoinHandle};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail, ensure};
use serde_json::{Value, json};

use client::{Frames, PORT, Session};

/// How many viewers, and for how long.
#[derive(Debug, Clone, Copy)]
pub struct Options {
    /// How many viewers.
    pub viewers: usize,
    /// For how long, from the end of the preparation.
    pub duration: Duration,
}

impl Options {
    /// The smoke run: 2 viewers for 20 s.
    pub const SMOKE: Options = Options {
        viewers: 2,
        duration: Duration::from_secs(20),
    };
}

/// The soak's users' password: its service listens on this computer only.
const PASSWORD: &str = "a soak's own password";
/// How often the service is sampled.
const SAMPLE: Duration = Duration::from_secs(5);
/// The longest the preparation's steps may take: the calibration, the snap and its pyramid.
/// How long the operator looks at live view between scans.
const LOOKING: Duration = Duration::from_secs(5);
const PREPARING: Duration = Duration::from_secs(600);

/// Runs the soak from the workspace at `repo`: its report.
pub fn run(repo: &Path, options: Options) -> Result<Report> {
    println!("soak: building spectrumiq-server in release");
    let build = [
        "build",
        "--release",
        "--locked",
        "-q",
        "-p",
        "spectrumiq-server",
    ];
    let built = Command::new("cargo")
        .args(build)
        .current_dir(repo)
        .status()?;
    ensure!(built.success(), "the release build failed");
    let exe = format!("spectrumiq-server{}", std::env::consts::EXE_SUFFIX);
    let server = repo.join("target/release").join(exe);
    // Each soak starts from a fresh instrument: no calibrations, users or data.
    let folder = repo.join("target/soak");
    if folder.exists() {
        fs::remove_dir_all(&folder).with_context(|| format!("emptying {}", folder.display()))?;
    }
    fs::create_dir_all(folder.join("config"))?;
    let config = folder.join("config/sim.toml");
    fs::copy(repo.join("config/sim.toml"), &config)?;
    add_user(&server, &config, "soak-admin", "admin")?;
    add_user(&server, &config, "soak-viewer", "viewer")?;
    let log = File::create(folder.join("service.log"))?;
    let mut service = Command::new(&server)
        .arg("-c")
        .arg(&config)
        .args(["--port", &PORT.to_string()])
        .stdout(log.try_clone()?)
        .stderr(log)
        .spawn()?;
    let soaked = soak(options);
    // The service is the soak's own, started above.
    service.kill().ok();
    service.wait().ok();
    let report = soaked?;
    let json = serde_json::to_string_pretty(&report.json)?;
    fs::write(folder.join("report.json"), json)?;
    Ok(report)
}

/// Adds `name` with `role` through the service's CLI, the password given twice on its input.
fn add_user(server: &Path, config: &Path, name: &str, role: &str) -> Result<()> {
    let mut cli = Command::new(server)
        .args(["user", "add", name, "--role", role, "-c"])
        .arg(config)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    let mut input = cli.stdin.take().context("the CLI's input")?;
    input.write_all(format!("{PASSWORD}\n{PASSWORD}\n").as_bytes())?;
    drop(input);
    ensure!(cli.wait()?.success(), "adding {name} failed");
    Ok(())
}

fn soak(options: Options) -> Result<Report> {
    let anyone = Session::default();
    let started = Instant::now();
    while anyone.json("GET", "/api/siq/system/health", None).is_err() {
        ensure!(
            started.elapsed() < Duration::from_secs(60),
            "the service didn't start"
        );
        thread::sleep(Duration::from_millis(250));
    }
    let admin = Session::login("soak-admin", PASSWORD)?;
    println!("soak: preparing: the token, the calibration, live view and a snap to view");
    let image = prepare(&admin)?;
    let (viewers, seconds) = (options.viewers, options.duration.as_secs_f64());
    println!("soak: {viewers} viewers and an operator for {seconds} s");
    let start = Instant::now();
    let until = start + options.duration;
    thread::scope(|scope| {
        let operator = scope.spawn(|| operate(&admin, until));
        let image = &image;
        let viewers: Vec<_> = (0..viewers)
            .map(|n| scope.spawn(move || view(n as u64 + 1, image, until)))
            .collect();
        let mut samples = vec![sample(&admin, start)?];
        while Instant::now() < until {
            thread::sleep(SAMPLE.min(until.saturating_duration_since(Instant::now())));
            samples.push(sample(&admin, start)?);
        }
        let seen = viewers
            .into_iter()
            .map(joined)
            .collect::<Result<Vec<_>>>()?;
        let scans = joined(operator)?;
        Ok(Report::new(options, &samples, &seen, &scans))
    })
}

fn joined<T>(handle: ScopedJoinHandle<'_, Result<T>>) -> Result<T> {
    handle.join().map_err(|_| anyhow!("a client panicked"))?
}

/// The snap the viewers pan about: where its tiles are, and its size in pixels and tiles.
struct Image {
    tiles: String,
    width: u32,
    height: u32,
    tile: u32,
}

/// The operator's preparation: the token, scans unstitched since their data goes, the
/// stage–camera calibration, live view, and a snap to view, with its pyramid.
fn prepare(admin: &Session) -> Result<Image> {
    let until = Instant::now() + PREPARING;
    let invoke = |action: &str| {
        admin
            .invoke(action, &json!({}), until)?
            .with_context(|| format!("{action} took too long"))
    };
    admin.json("POST", "/api/siq/control/request", None)?;
    admin.json("PUT", "/api/v1/jobs/auto_stitch", Some(&json!(false)))?;
    invoke("calibration/stage_camera")?;
    invoke("camera/start_live")?;
    let snap = invoke("datasets/snap_to_dataset")?;
    let id = snap["dataset"]["id"]
        .as_str()
        .context("the snap's dataset")?;
    let image = snap["image"].as_str().context("the snap's image")?;
    loop {
        let manifest = admin.json("GET", &format!("/api/siq/data/{id}/manifest"), None)?;
        let history = manifest["history"].as_array().cloned().unwrap_or_default();
        let pyramid = history
            .iter()
            .rev()
            .find(|record| record["kind"] == "pyramid");
        match pyramid.and_then(|record| record["state"].as_str()) {
            Some("done") => break,
            Some(state @ ("failed" | "cancelled")) => bail!("the snap's pyramid: {state}"),
            _ => {}
        }
        ensure!(Instant::now() < until, "the snap's pyramid took too long");
        thread::sleep(Duration::from_millis(200));
    }
    let plane = format!("/api/siq/data/{id}/{image}/0/0/0");
    let dzi = admin.text(&format!("{plane}.dzi"))?;
    let attribute = |name: &str| -> Result<u32> {
        let at = dzi.find(&format!("{name}=\"")).context(name.to_owned())? + name.len() + 2;
        let value = dzi[at..].split('"').next().unwrap_or_default();
        value.parse().with_context(|| format!("the DZI's {name}"))
    };
    Ok(Image {
        tiles: format!("{plane}_files"),
        width: attribute("Width")?,
        height: attribute("Height")?,
        tile: attribute("TileSize")?,
    })
}

/// What the operator did: its scans, their frames, and how long each took.
#[derive(Default)]
struct Scans {
    done: u64,
    frames: u64,
    seconds: Vec<f64>,
}

/// Runs 2 × 2 tile scans until `until`, as `tile_scan.py` does, deleting each one's dataset, and
/// looks at live view between them.
fn operate(admin: &Session, until: Instant) -> Result<Scans> {
    let sensor = admin.json("GET", "/api/v1/camera/sensor", None)?;
    let number = |name: &str| sensor[name].as_f64().unwrap_or_default();
    // A frame's size on the stage through the 10× objective; a scan of a little under two each way.
    let pixel = number("pixel_size") / 10.0;
    let step = [
        0.9 * number("width") * pixel,
        0.9 * number("height") * pixel,
    ];
    let (x, y) = (-20_000.0, -15_000.0);
    let region = json!({
        "shape": "rect",
        "x0": x - 0.8 * step[0], "y0": y - 0.8 * step[1],
        "x1": x + 0.8 * step[0], "y1": y + 0.8 * step[1],
    });
    let scan = json!({ "name": "soak", "region": region, "overlap": 0.1, "channels": [1] });
    let mut scans = Scans::default();
    while Instant::now() < until {
        let started = Instant::now();
        let Some(done) = admin.invoke("workflow/tile_scan", &scan, until)? else {
            break;
        };
        scans.done += 1;
        scans.frames += done["frames"].as_u64().unwrap_or_default();
        scans.seconds.push(started.elapsed().as_secs_f64());
        if let Some(dataset) = done["dataset"].as_str() {
            let delete = json!({ "dataset": dataset });
            admin.invoke("datasets/delete", &delete, Instant::now() + PREPARING)?;
        }
        let stats = admin.json("GET", "/api/v1/camera/stats", None)?;
        if stats["mode"] != "live" {
            admin.invoke("camera/start_live", &json!({}), until)?;
        }
        thread::sleep(LOOKING.min(until.saturating_duration_since(Instant::now())));
    }
    Ok(scans)
}

/// What a viewer saw: each stream's frames, its observations, and its tiles' latencies.
struct Seen {
    overview: Frames,
    roi: Frames,
    observations: u64,
    tiles_ms: Vec<f64>,
    tiles_failed: u64,
}

impl Seen {
    /// The overview's frames, for 0, or the ROI's.
    fn stream(&self, index: usize) -> &Frames {
        if index == 0 {
            &self.overview
        } else {
            &self.roi
        }
    }
}

/// A viewer, logged in, until `until`; `seed` varies where it pans.
fn view(seed: u64, image: &Image, until: Instant) -> Result<Seen> {
    let viewer = Session::login("soak-viewer", PASSWORD)?;
    thread::scope(|scope| {
        let overview = scope.spawn(|| viewer.frames("/api/v1/camera/overview", until));
        let roi = scope.spawn(|| viewer.frames("/api/v1/camera/roi", until));
        let camera = scope.spawn(|| viewer.observe("camera", &["stats"], until));
        let stage = scope.spawn(|| viewer.observe("stage", &["position", "state"], until));
        let (tiles_ms, tiles_failed) = pan(&viewer, image, seed, until);
        Ok(Seen {
            overview: joined(overview)?,
            roi: joined(roi)?,
            observations: joined(camera)? + joined(stage)?,
            tiles_ms,
            tiles_failed,
        })
    })
}

/// Pans about `image` as a viewer would, until `until`: 4 × 3 tiles of a level at a time, a
/// second apart. The latencies of the tiles fetched, and how many failed.
fn pan(viewer: &Session, image: &Image, seed: u64, until: Instant) -> (Vec<f64>, u64) {
    // The DZI's top level, its full size: ⌈log₂ of its longer side⌉.
    let top = image
        .width
        .max(image.height)
        .next_power_of_two()
        .trailing_zeros();
    let mut state = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    let mut below = |n: u32| {
        // xorshift64: enough to vary where each viewer looks.
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        u32::try_from(state % u64::from(n.max(1))).unwrap_or_default()
    };
    let (mut latencies, mut failed) = (Vec::new(), 0);
    while Instant::now() < until {
        let level = top - below(4).min(top);
        let scale = 1 << (top - level);
        let cols = image.width.div_ceil(scale).div_ceil(image.tile);
        let rows = image.height.div_ceil(scale).div_ceil(image.tile);
        let (col0, row0) = (below(cols), below(rows));
        for row in row0..(row0 + 3).min(rows) {
            for col in col0..(col0 + 4).min(cols) {
                let path = format!("{}/{level}/{col}_{row}.jpeg", image.tiles);
                match viewer.timed(&path) {
                    Some(ms) => latencies.push(ms),
                    None => failed += 1,
                }
            }
        }
        thread::sleep(Duration::from_secs(1));
    }
    (latencies, failed)
}

/// The service at a moment, `at` seconds into the soak.
struct Sample {
    at: f64,
    cpu_s: f64,
    working_set: f64,
    private: f64,
    lost: f64,
    published: f64,
    /// The overview's and the ROI's latency, then their pipeline's work.
    previews: [Histogram; 4],
}

/// A latency histogram: its count, its sum, and each bucket's upper bound and count.
struct Histogram {
    count: f64,
    sum: f64,
    buckets: Vec<(Option<f64>, f64)>,
}

impl Histogram {
    fn of(metric: &Value) -> Histogram {
        let buckets = metric["buckets"].as_array().cloned().unwrap_or_default();
        Histogram {
            count: metric["count"].as_f64().unwrap_or_default(),
            sum: metric["sum"].as_f64().unwrap_or_default(),
            buckets: (buckets.iter())
                .map(|bucket| {
                    (
                        bucket["le"].as_f64(),
                        bucket["count"].as_f64().unwrap_or_default(),
                    )
                })
                .collect(),
        }
    }

    /// The bound below which `share` of the values counted since `before` fall: `None` past
    /// the last bound, or with no values.
    fn quantile_since(&self, before: &Histogram, share: f64) -> Option<f64> {
        let total = self.count - before.count;
        let mut sum = 0.0;
        for ((bound, count), (_, earlier)) in self.buckets.iter().zip(&before.buckets) {
            sum += count - earlier;
            if total > 0.0 && sum >= share * total {
                return *bound;
            }
        }
        None
    }
}

fn sample(admin: &Session, start: Instant) -> Result<Sample> {
    let memory = admin.json("GET", "/api/siq/system/memory", None)?;
    let metrics = admin.json("GET", "/api/siq/system/metrics", None)?;
    let gauge = |name: &str| metrics[name]["value"].as_f64().unwrap_or_default();
    let process = &memory["process"];
    Ok(Sample {
        at: start.elapsed().as_secs_f64(),
        cpu_s: process["cpu_s"].as_f64().unwrap_or_default(),
        working_set: process["working_set"].as_f64().unwrap_or_default(),
        private: process["private"].as_f64().unwrap_or_default(),
        lost: gauge("acquisition.lost_device") + gauge("acquisition.lost_no_slot"),
        published: gauge("acquisition.published"),
        previews: [
            Histogram::of(&metrics["preview.overview.latency_ms"]),
            Histogram::of(&metrics["preview.roi.latency_ms"]),
            Histogram::of(&metrics["preview.overview.work_ms"]),
            Histogram::of(&metrics["preview.roi.work_ms"]),
        ],
    })
}

/// ADR-0069's targets.
const WORK_P95_MS: f64 = 100.0;
const FRAMES_RECEIVED: f64 = 0.9;
const TILE_P95_MS: f64 = 1000.0;

/// What a soak found, as text and JSON, and the targets it missed.
pub struct Report {
    lines: Vec<String>,
    missed: Vec<String>,
    json: Value,
}

impl Report {
    fn new(options: Options, samples: &[Sample], seen: &[Seen], scans: &Scans) -> Report {
        let (first, last) = (&samples[0], &samples[samples.len() - 1]);
        let seconds = last.at - first.at;
        let mut lines = vec![format!(
            "{} viewers and an operator for {seconds:.0} s: {} scans, {} frames, {:.1} s a scan",
            options.viewers,
            scans.done,
            scans.frames,
            mean(&scans.seconds)
        )];
        let mut missed = Vec::new();
        let mut streams = Vec::new();
        for (index, name) in ["overview", "roi"].into_iter().enumerate() {
            let (now, then) = (&last.previews[index], &first.previews[index]);
            let (work, worked) = (&last.previews[index + 2], &first.previews[index + 2]);
            let made = now.count - then.count;
            let (latency_p95, work_p95) = (
                now.quantile_since(then, 0.95),
                work.quantile_since(worked, 0.95),
            );
            let bound =
                |p95: Option<f64>| p95.map_or("over 1000".to_owned(), |ms| format!("≤ {ms:.0}"));
            let received: Vec<f64> = seen
                .iter()
                .map(|viewer| viewer.stream(index).count as f64)
                .collect();
            let least = received.iter().copied().reduce(f64::min);
            let fewest = least.map_or("none: no viewer".to_owned(), |n| format!("{n:.0}"));
            let gaps: Vec<f64> = seen
                .iter()
                .flat_map(|viewer| viewer.stream(index).gaps_ms.clone())
                .collect();
            lines.push(format!(
                "{name}: {made:.0} made; from mid-exposure {:.0} ms on average, p95 {} ms; the \
                 pipeline's work {:.0} ms, p95 {} ms; the least a viewer received {fewest}; \
                 gaps p95 {:.0} ms, at most {:.0} ms",
                (now.sum - then.sum) / made.max(1.0),
                bound(latency_p95),
                (work.sum - worked.sum) / made.max(1.0),
                bound(work_p95),
                percentile(&gaps, 0.95),
                gaps.iter().copied().fold(0.0, f64::max)
            ));
            if work_p95.is_none_or(|ms| ms > WORK_P95_MS) {
                let p95 = bound(work_p95);
                missed.push(format!("{name}'s work p95 {p95} ms, over {WORK_P95_MS} ms"));
            }
            if let Some(least) = least
                && made > 0.0
                && least < FRAMES_RECEIVED * made
            {
                missed.push(format!(
                    "a viewer received {least:.0} of {name}'s {made:.0} frames"
                ));
            }
            streams.push(
                json!({ "stream": name, "made": made, "latency_p95_ms": latency_p95,
                "work_p95_ms": work_p95, "received": received,
                "gap_p95_ms": percentile(&gaps, 0.95) }),
            );
        }
        let observations = seen.iter().map(|viewer| viewer.observations).min();
        let tiles: Vec<f64> = seen
            .iter()
            .flat_map(|viewer| viewer.tiles_ms.clone())
            .collect();
        let failed: u64 = seen.iter().map(|viewer| viewer.tiles_failed).sum();
        let tile_p95 = percentile(&tiles, 0.95);
        lines.push(format!(
            "tiles: {} fetched, {failed} failed, p50 {:.0} ms, p95 {tile_p95:.0} ms, at most {:.0} \
             ms; observations: at least {} a viewer",
            tiles.len(),
            percentile(&tiles, 0.5),
            tiles.iter().copied().fold(0.0, f64::max),
            observations.unwrap_or_default()
        ));
        if failed > 0 || tile_p95 > TILE_P95_MS {
            missed.push(format!("tiles: {failed} failed, p95 {tile_p95:.0} ms"));
        }
        if observations == Some(0) {
            missed.push("a viewer observed nothing".to_owned());
        }
        let lost = last.lost - first.lost;
        lines.push(format!(
            "camera: {:.0} frames published, {lost:.0} lost",
            last.published - first.published
        ));
        if lost > 0.0 {
            missed.push(format!("the camera lost {lost:.0} frames"));
        }
        let cores = thread::available_parallelism().map_or(1.0, |n| n.get() as f64);
        let busy = |a: &Sample, b: &Sample| 100.0 * (b.cpu_s - a.cpu_s) / (b.at - a.at) / cores;
        let peak = samples
            .windows(2)
            .map(|pair| busy(&pair[0], &pair[1]))
            .fold(0.0, f64::max);
        let gb = |bytes: f64| bytes / 1e9;
        let most = |of: fn(&Sample) -> f64| samples.iter().map(of).fold(0.0, f64::max);
        lines.push(format!(
            "service: CPU {:.0} % of {cores} cores on average, {peak:.0} % at most; working set \
             {:.2} → {:.2} GB, at most {:.2}; private {:.2} → {:.2} GB, at most {:.2}",
            busy(first, last),
            gb(first.working_set),
            gb(last.working_set),
            gb(most(|s| s.working_set)),
            gb(first.private),
            gb(last.private),
            gb(most(|s| s.private)),
        ));
        let json = json!({
            "viewers": options.viewers, "seconds": seconds,
            "scans": { "done": scans.done, "frames": scans.frames, "seconds": scans.seconds },
            "streams": streams, "observations_least": observations,
            "tiles": { "fetched": tiles.len(), "failed": failed, "p95_ms": tile_p95 },
            "camera_lost": lost,
            "cpu_percent": busy(first, last), "cpu_peak_percent": peak,
            "samples": samples.iter().map(|s| json!({ "at": s.at, "cpu_s": s.cpu_s,
                "working_set": s.working_set, "private": s.private })).collect::<Vec<_>>(),
            "missed": missed,
        });
        Report {
            lines,
            missed,
            json,
        }
    }

    /// Whether the soak met every target.
    pub fn passed(&self) -> bool {
        self.missed.is_empty()
    }
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for line in &self.lines {
            writeln!(f, "{line}")?;
        }
        match self.missed.as_slice() {
            [] => write!(f, "soak: every target met (ADR-0069)"),
            missed => write!(f, "soak: missed: {}", missed.join("; ")),
        }
    }
}

fn mean(values: &[f64]) -> f64 {
    values.iter().sum::<f64>() / values.len().max(1) as f64
}

/// The value below which `share` of `values` fall; 0 for none.
fn percentile(values: &[f64], share: f64) -> f64 {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let at = ((sorted.len() as f64 * share).ceil() as usize).saturating_sub(1);
    sorted.get(at).copied().unwrap_or_default()
}
