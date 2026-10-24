//! Renders frames of a Cells world on the 100 MP sensor, through a 10× objective, and writes the
//! last as a raw frame plus an 8-bit PNG preview. It prints how long each kind of frame takes.
//!
//! ```text
//! cargo run --release -p siq-sim --example render_frame [-- <x µm> <y µm> <focus µm> [<folder>]]
//! ```
//!
//! The folder defaults to `target/render_frame`. Smoke mode (`SIQ_SMOKE=1`) uses a 1 MP sensor.

use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use siq_devices::{CameraSettings, TriggerMode};
use siq_imgproc::{Colormap, DisplayMapping, Histogram, Lut, downsample_area, histogram};
use siq_sim::optics::{OpticsModel, Renderer, SensorModel, Shot};
use siq_synth::{Cells, CellsParams};
use siq_types::{Affine2, Binning, Micrometers, Millis, OwnedImage, PixelFormat, Roi, StagePose};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let number = |n: usize| args.get(n).map_or(Ok(0.0), |a| a.parse::<f64>());
    let pose = StagePose {
        x: Micrometers(number(0)?),
        y: Micrometers(number(1)?),
        focus: Micrometers(number(2)?),
        ..StagePose::default()
    };
    let folder = args
        .get(3)
        .map_or_else(|| PathBuf::from("target/render_frame"), PathBuf::from);
    let smoke = std::env::var_os("SIQ_SMOKE").is_some();
    let sensor = if smoke { (1152, 864) } else { (11_648, 8_742) };

    // A 3.76 µm pixel through 10×: 0.376 µm on the sample.
    let world = Arc::new(Cells::new(CellsParams {
        seed: 1,
        ..CellsParams::default()
    }));
    let mut renderer = Renderer::new(world, OpticsModel::default(), SensorModel::default());
    let settings = CameraSettings {
        roi: Roi::full(sensor.0, sensor.1),
        binning: Binning::NONE,
        pixel_format: PixelFormat::Mono14,
        exposure: Millis(10.0),
        gain: 1.0,
        trigger: TriggerMode::FreeRun,
    };
    let shot = Shot {
        pose,
        calibration: Affine2::scale(1.0 / 0.376, 1.0 / 0.376),
        sensor,
        settings,
    };
    let mut frame = OwnedImage::<u16>::new(sensor.0 as usize, sensor.1 as usize);
    println!(
        "{} × {} pixels, {} threads, at {pose:?}",
        sensor.0,
        sensor.1,
        threads()
    );

    // A new pose, the same pose, and the same pose again; then the same with a quarter of the
    // pixels (the `fast` profile, 2 × 2 binned).
    let mut moved = shot;
    moved.pose.x += Micrometers(25.0);
    let mut fast = shot;
    fast.settings.binning = Binning::new(2, 2).expect("2 isn't 0");
    let mut fast_frame = OwnedImage::<u16>::new(sensor.0 as usize / 2, sensor.1 as usize / 2);
    let cases: [(&str, &Shot, bool); 8] = [
        ("draft, the pose new", &moved, false),
        ("draft, the pose new", &shot, false),
        ("full, the pose still", &shot, false),
        ("noise only, the pose still", &shot, false),
        ("noise only, the pose still", &shot, false),
        ("fast profile: draft", &fast, true),
        ("fast profile: full", &fast, true),
        ("fast profile: noise only", &fast, true),
    ];
    for (seq, (name, case, binned)) in (0u64..).zip(cases) {
        let started = Instant::now();
        let target = if binned { &mut fast_frame } else { &mut frame };
        let rendered = renderer.render(case, seq, &mut target.view_mut())?;
        let ms = started.elapsed().as_secs_f64() * 1e3;
        let fps = 1e3 / ms;
        println!(
            "{name:<28} {ms:>8.1} ms ({fps:>5.1} frames/s), detail {}, blur {:.2}–{:.2} px",
            rendered.detail, rendered.blur[0], rendered.blur[1]
        );
    }
    renderer.render(&shot, 99, &mut frame.view_mut())?;

    // The raw frame, little-endian u16, and a preview with auto-contrast.
    fs::create_dir_all(&folder)?;
    let mut raw = BufWriter::new(File::create(folder.join("frame.raw"))?);
    for value in frame.view().rows().flatten() {
        raw.write_all(&value.to_le_bytes())?;
    }
    let mut counts = Histogram::new(14);
    histogram(&frame.view(), 4, &mut counts)?;
    let (min, max) = counts.auto_window(0.001, 0.999);
    let scale = (1920.0 / f64::from(sensor.0)).min(1.0);
    let (width, height) = (
        (f64::from(sensor.0) * scale) as usize,
        (f64::from(sensor.1) * scale) as usize,
    );
    let mut small = OwnedImage::<u16>::new(width, height);
    downsample_area(&frame.view(), &mut small.view_mut())?;
    let mut shown = OwnedImage::<u8>::new(width, height);
    Lut::new(&DisplayMapping {
        min,
        max,
        gamma: 1.0,
        colormap: Colormap::Grey,
    })?
    .apply(&small.view(), &mut shown.view_mut())?;
    let mut encoder = png::Encoder::new(
        BufWriter::new(File::create(folder.join("preview.png"))?),
        width as u32,
        height as u32,
    );
    encoder.set_color(png::ColorType::Grayscale);
    encoder.set_depth(png::BitDepth::Eight);
    let bytes: Vec<u8> = shown.view().rows().flatten().copied().collect();
    encoder.write_header()?.write_image_data(&bytes)?;
    println!(
        "wrote {} (Mono14 in u16, {} × {}) and preview.png, window {min}–{max}",
        folder.join("frame.raw").display(),
        sensor.0,
        sensor.1
    );
    Ok(())
}

fn threads() -> usize {
    std::thread::available_parallelism().map_or(1, |n| n.get())
}
