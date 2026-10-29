//! The features of the camera on this PC's grabber, for writing its feature map (ADR-0070), and
//! for keeping its settings around an experiment.
//!
//! ```text
//! cargo run --release -p siq-euresys --features sdk --example eg_features -- [remote|device|stream|interface]
//! cargo run --release -p siq-euresys --features sdk --example eg_features -- --save <file>
//! cargo run --release -p siq-euresys --features sdk --example eg_features -- --restore <file>
//! ```
//!
//! With a module, it lists that module's features (the camera's by default), each with its value,
//! and an enumeration's entries; it changes nothing. `--save` writes the settings the camera
//! changes, one `feature=value` a line; `--restore` sets them again, in order.

use std::process::ExitCode;

use siq_euresys::{EGrabber, Grabber, GrabberError, Module, module_of};

/// What `--save` keeps, in the order `--restore` sets it: the camera shortens an exposure to fit
/// its period, so the period comes before the exposure.
const KEPT: [&str; 13] = [
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
    "device:CameraControlMethod",
];

fn list(grabber: &mut EGrabber, module: Module) -> Result<(), GrabberError> {
    for feature in grabber.features(module)? {
        let feature = feature.as_str();
        let value = grabber
            .get(module, feature)
            .unwrap_or_else(|_| "(unreadable)".into());
        let entries = grabber.entries(module, feature).unwrap_or_default();
        match entries.as_slice() {
            [] => println!("{feature} = {value}"),
            entries => println!("{feature} = {value}  [{}]", entries.join(", ")),
        }
    }
    Ok(())
}

fn save(grabber: &mut EGrabber, path: &str) -> Result<(), String> {
    let mut text = String::new();
    for feature in KEPT {
        let (module, name) = module_of(feature);
        if let Ok(value) = grabber.get(module, name) {
            text += &format!("{feature}={value}\n");
        }
    }
    std::fs::write(path, &text).map_err(|e| format!("{path}: {e}"))?;
    print!("{text}");
    Ok(())
}

fn restore(grabber: &mut EGrabber, path: &str) -> Result<(), String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
    // The offsets first, so that the size fits.
    let cleared = ["OffsetX=0", "OffsetY=0"];
    for line in cleared.into_iter().chain(text.lines()) {
        let Some((feature, value)) = line.split_once('=') else {
            continue;
        };
        let (module, name) = module_of(feature);
        grabber
            .set(module, name, value)
            .map_err(|e| e.to_string())?;
    }
    println!("{} settings put back from {path}", text.lines().count());
    Ok(())
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut grabber = EGrabber::open(None, 0, 0).map_err(|e| e.to_string())?;
    match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["--save", path] => save(&mut grabber, path),
        ["--restore", path] => restore(&mut grabber, path),
        [] | ["remote"] => list(&mut grabber, Module::Remote).map_err(|e| e.to_string()),
        ["device"] => list(&mut grabber, Module::Device).map_err(|e| e.to_string()),
        ["stream"] => list(&mut grabber, Module::Stream).map_err(|e| e.to_string()),
        ["interface"] => list(&mut grabber, Module::Interface).map_err(|e| e.to_string()),
        _ => Err(
            "eg_features [remote|device|stream|interface] | --save <file> | --restore <file>"
                .into(),
        ),
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("eg_features: {error}");
            ExitCode::FAILURE
        }
    }
}
