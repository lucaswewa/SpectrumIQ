//! Software triggers and a strobe (P10.4, ADR-0072): the camera of this PC's grabber triggered
//! again and again, a strobe on one of the grabber's output lines during each exposure. It prints
//! each trigger's latency, from the call to the exposure's start, and the pulses the grabber
//! counted: the camera's triggers, the strobes, and the output line's own rising edges, read back
//! through a line input tool, which needs no wiring.
//!
//! ```text
//! cargo run --release -p siq-euresys --features sdk --example strobe_timing -- [--triggers N] [--exposure ms] [--line IOUT11] [--delay ms] [--duration ms] [--gap ms]
//! ```
//!
//! 20 triggers, 200 ms apart, at a 5 ms exposure, of 2048 × 2048 in the middle, the strobe on
//! `IOUT11` for the whole exposure. The exposure's start is the grabber's stamp, the end of the
//! exposure, less the exposure, on the host's clock (P10.2). Everything it changes, on the camera
//! and on the grabber, is put back as it was.

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use siq_buffer::{Backend, FillGuard, FramePool, PoolConfig};
use siq_devices::{Camera, CameraSettings, FrameSink, TriggerMode};
use siq_euresys::{EGrabber, EuresysCamera, FeatureMap, Grabber, Module, Strobe, module_of};
use siq_types::{Clock, FrameMeta, Millis, Roi, SystemClock, Timestamp};

/// The camera's own features it changes, read first and put back after, in that order.
const KEPT: [&str; 12] = [
    "PixelFormat",
    "BinningHorizontal",
    "BinningVertical",
    "Width",
    "Height",
    "OffsetX",
    "OffsetY",
    "ExposureMode",
    "TriggerSource",
    "AcquisitionFramePeriod",
    "ExposureTime",
    "Gain",
];

/// What the grabber's camera controller shows in `RC`, which the strobe sets.
const CONTROLLER: [&str; 5] = [
    "device:CycleTriggerSource",
    "device:ExposureTime",
    "device:StrobeDelay",
    "device:StrobeDuration",
    "device:EventSelector",
];

/// Every setting to put back, in order: a selector set before what it selects.
fn saved(line: &str) -> Result<Vec<(String, String)>, String> {
    let mut g = EGrabber::open(None, 0, 0).map_err(|e| e.to_string())?;
    let mut kept = Vec::new();
    let mut read = |g: &mut EGrabber, feature: &str| -> Result<String, String> {
        let (module, name) = module_of(feature);
        let value = g.get(module, name).map_err(|e| format!("{feature}: {e}"))?;
        kept.push((feature.to_owned(), value.clone()));
        Ok(value)
    };
    for feature in KEPT {
        read(&mut g, feature)?;
    }
    let method = g
        .get(Module::Device, "CameraControlMethod")
        .map_err(|e| e.to_string())?;
    let set = |g: &mut EGrabber, feature: &str, value: &str| {
        let (module, name) = module_of(feature);
        g.set(module, name, value)
            .map_err(|e| format!("{feature}: {e}"))
    };
    set(&mut g, "device:CameraControlMethod", "RC")?;
    read(&mut g, "device:CameraControlMethod")?;
    for feature in CONTROLLER {
        read(&mut g, feature)?;
    }
    set(&mut g, "device:CameraControlMethod", &method)?;
    read(&mut g, "device:CameraControlMethod")?;
    let selected = read(&mut g, "interface:LineSelector")?;
    set(&mut g, "interface:LineSelector", line)?;
    read(&mut g, "interface:LineSelector")?;
    read(&mut g, "interface:LineSource")?;
    let tool = read(&mut g, "interface:LineInputToolSelector")?;
    set(&mut g, "interface:LineInputToolSelector", "LIN1")?;
    read(&mut g, "interface:LineInputToolSelector")?;
    read(&mut g, "interface:LineInputToolSource")?;
    read(&mut g, "interface:LineInputToolActivation")?;
    read(&mut g, "interface:EventSelector")?;
    set(&mut g, "interface:LineSelector", &selected)?;
    set(&mut g, "interface:LineInputToolSelector", &tool)?;
    kept.push(("interface:LineSelector".into(), selected));
    kept.push(("interface:LineInputToolSelector".into(), tool));
    Ok(kept)
}

/// Sets `saved` again, the camera's offsets cleared first so that its size fits.
fn put_back(saved: &[(String, String)]) -> Result<(), String> {
    let mut g = EGrabber::open(None, 0, 0).map_err(|e| e.to_string())?;
    let cleared = [("OffsetX", "0"), ("OffsetY", "0")];
    let all = cleared
        .into_iter()
        .chain(saved.iter().map(|(f, v)| (f.as_str(), v.as_str())));
    for (feature, value) in all {
        let (module, name) = module_of(feature);
        g.set(module, name, value)
            .map_err(|e| format!("{feature}={value}: {e}"))?;
    }
    Ok(())
}

/// The grabber's count of event `event`, of `module`.
fn count(g: &mut EGrabber, module: Module, event: &str) -> u64 {
    let counted = g
        .set(module, "EventSelector", event)
        .and_then(|()| g.get(module, "EventCount"));
    counted.ok().and_then(|n| n.parse().ok()).unwrap_or(0)
}

/// The grabber's counts: the camera's triggers, the strobes, and the line's rising edges.
fn counts(g: &mut EGrabber) -> [u64; 3] {
    [
        count(g, Module::Device, "CameraTriggerRisingEdge"),
        count(g, Module::Device, "StrobeRisingEdge"),
        count(g, Module::Interface, "LIN1"),
    ]
}

/// Each frame's metadata, its slot given back at once.
#[derive(Default)]
struct Frames(Mutex<Vec<FrameMeta>>);

impl FrameSink for Frames {
    fn frame(&self, slot: FillGuard, meta: FrameMeta) {
        self.0.lock().unwrap().push(meta);
        drop(slot.publish(meta));
    }
}

/// The least, mean and most of `values`, and their standard deviation.
fn stats(values: &[f64]) -> String {
    let n = values.len().max(1) as f64;
    let mean = values.iter().sum::<f64>() / n;
    let sd = (values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / n).sqrt();
    let least = values.iter().copied().fold(f64::INFINITY, f64::min);
    let most = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    format!("{least:.3} at least, {mean:.3} on average, {most:.3} at most, {sd:.3} rms")
}

async fn run(options: &Options) -> Result<(), String> {
    let path = std::env::var_os("SIQ_EURESYS_MAP").map_or_else(
        || {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../config/cameras/adimec-d103a12c.toml")
        },
        PathBuf::from,
    );
    let map = FeatureMap::load(&path).map_err(|e| e.to_string())?;
    let clock: Arc<dyn Clock> = Arc::new(SystemClock::new());
    let grabber = EGrabber::open(None, 0, 0).map_err(|e| e.to_string())?;
    let strobe = Strobe {
        line: options.line.clone(),
        delay: options.delay,
        duration: options.duration,
    };
    let camera = EuresysCamera::new(grabber, map, clock.clone())
        .and_then(|camera| camera.with_strobe(strobe))
        .map_err(|e| e.to_string())?;
    let middle = Roi {
        offset_x: 4608,
        offset_y: 3584,
        width: 2048,
        height: 2048,
    };
    let settings = CameraSettings {
        roi: middle,
        exposure: options.exposure,
        trigger: TriggerMode::Software,
        ..camera.settings()
    };
    camera
        .configure(settings)
        .await
        .map_err(|e| e.to_string())?;
    // The line's own rising edges, read back through a line input tool.
    let before = camera.with_grabber(|g| {
        let tool = [
            ("LineInputToolSelector", "LIN1"),
            ("LineInputToolSource", options.line.as_str()),
            ("LineInputToolActivation", "RisingEdge"),
        ];
        for (name, value) in tool {
            g.set(Module::Interface, name, value)?;
        }
        Ok::<_, siq_euresys::GrabberError>(counts(g))
    });
    let before = before.map_err(|e| e.to_string())?;
    let pool = PoolConfig {
        name: "strobe_timing".into(),
        slots: 8,
        slot_bytes: settings.frame_bytes(),
        backend: Backend::Heap,
    };
    let pool = FramePool::new(&pool, clock.clone(), None).map_err(|e| e.to_string())?;
    let frames = Arc::new(Frames::default());
    camera
        .start(pool, frames.clone())
        .await
        .map_err(|e| e.to_string())?;
    let mut calls: Vec<(Timestamp, Timestamp)> = Vec::new();
    for _ in 0..options.triggers {
        let called = clock.now();
        camera.trigger().await.map_err(|e| e.to_string())?;
        calls.push((called, clock.now()));
        tokio::time::sleep(options.gap).await;
    }
    tokio::time::sleep(Duration::from_millis(500)).await;
    camera.stop().await.map_err(|e| e.to_string())?;
    let after = camera.with_grabber(counts);
    drop(camera);

    let frames = frames.0.lock().unwrap().clone();
    println!(
        "{} triggers, {} frames, {} ms apart, a {} ms exposure",
        calls.len(),
        frames.len(),
        options.gap.as_millis(),
        options.exposure.0
    );
    let ms = |from: Timestamp, to: Timestamp| (to.0 as f64 - from.0 as f64) / 1e6;
    let calling: Vec<f64> = calls.iter().map(|&(a, b)| ms(a, b)).collect();
    println!("the trigger call, ms: {}", stats(&calling));
    // The offset of the grabber's clock settles over the first frames: those after the fifth.
    let half = (options.exposure.0 * 1e6 / 2.0) as u64;
    let latency: Vec<f64> = calls
        .iter()
        .zip(&frames)
        .skip(5)
        .map(|(&(called, _), meta)| ms(called, Timestamp(meta.host_ts_mid_exposure.0 - half)))
        .collect();
    println!(
        "from the call to the exposure's start, ms: {}",
        stats(&latency)
    );
    let [triggers, strobes, edges] = [0, 1, 2].map(|k| after[k].saturating_sub(before[k]));
    println!(
        "counted: {triggers} camera triggers, {strobes} strobes, {edges} rising edges on {}",
        options.line
    );
    Ok(())
}

struct Options {
    triggers: usize,
    exposure: Millis,
    line: String,
    delay: Millis,
    duration: Option<Millis>,
    gap: Duration,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let after = |flag: &str| {
        let at = args.iter().position(|arg| arg == flag)?;
        args.get(at + 1).cloned()
    };
    let number =
        |flag: &str, default: f64| after(flag).and_then(|v| v.parse().ok()).unwrap_or(default);
    let options = Options {
        triggers: number("--triggers", 20.0) as usize,
        exposure: Millis(number("--exposure", 5.0)),
        line: after("--line").unwrap_or_else(|| "IOUT11".into()),
        delay: Millis(number("--delay", 0.0)),
        duration: after("--duration").and_then(|v| v.parse().ok()).map(Millis),
        gap: Duration::from_secs_f64(number("--gap", 200.0) / 1e3),
    };
    let saved = match saved(&options.line) {
        Ok(saved) => saved,
        Err(error) => {
            eprintln!("strobe_timing: {error}");
            return ExitCode::FAILURE;
        }
    };
    let ran = run(&options).await;
    let restored = put_back(&saved);
    if let Err(error) = &restored {
        eprintln!("strobe_timing: putting the settings back: {error}");
    }
    match (ran, restored) {
        (Ok(()), Ok(())) => ExitCode::SUCCESS,
        (Err(error), _) => {
            eprintln!("strobe_timing: {error}");
            ExitCode::FAILURE
        }
        _ => ExitCode::FAILURE,
    }
}
