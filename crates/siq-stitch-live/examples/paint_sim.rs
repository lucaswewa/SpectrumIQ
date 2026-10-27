//! The live canvas (ADR-0059) on this machine's GPU: a 6 × 6 grid of synthetic frames of a slide
//! of grains at 10 % overlap, the camera turned 0.4°, each reported 0.5 µm from where it truly
//! was, painted through its affine across the corner where four pages meet; then the painted
//! region, shrunk, written as a viewport PNG.
//!
//! ```text
//! cargo run --release -p siq-stitch-live --example paint_sim [-- <out.png>]
//! ```
//!
//! The frames are 1024 × 768 at 0.376 µm a pixel, painted at half a canvas pixel each. The PNG goes
//! to `<out.png>`, or `paint_sim.png` in the temporary folder. Smoke mode (`SIQ_SMOKE=1`) paints
//! 2 × 2 frames of 256 × 192.

use std::fs::File;
use std::io::BufWriter;
use std::path::PathBuf;
use std::time::Instant;

use siq_gpu::{GpuHandle, GpuOptions};
use siq_stitch_live::paint::Frame;
use siq_stitch_live::{Canvas, CanvasGeometry, CanvasOptions, PAGE, frame_to_canvas};
use siq_synth::{Grains, GrainsParams, World};
use siq_types::{Affine2, AffineParams, Micrometers, Vec2Um};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let smoke = std::env::var_os("SIQ_SMOKE").is_some();
    let out = std::env::args()
        .nth(1)
        .map_or_else(|| std::env::temp_dir().join("paint_sim.png"), PathBuf::from);
    let (grid, width, height) = if smoke { (2, 256, 192) } else { (6, 1024, 768) };
    let handle = match GpuHandle::open(&GpuOptions::default()) {
        Ok(handle) => handle,
        Err(error) => {
            println!("no GPU, nothing painted: {error}");
            return Ok(());
        }
    };
    println!("on {} ({})", handle.info().name, handle.info().backend);

    // The simulator's mount: 2.66 sensor pixels a µm, turned 0.4°.
    let l = Affine2::from_params(&AffineParams {
        scale_x: 1.0 / 0.376,
        scale_y: 1.0 / 0.376,
        rotation: 0.4_f64.to_radians(),
        shear: 0.0,
        translation: [0.0, 0.0],
    });
    let inverse = l.invert().expect("the mount is invertible");
    let geometry = CanvasGeometry::for_calibration(&l, 0.5);
    let world = Grains::new(GrainsParams::default());
    // Tiles a frame less 10 % apart, along the camera's axes, about the place on the stage that
    // is a page corner of the canvas.
    let corner = geometry.to_stage([f64::from(PAGE), f64::from(PAGE)]);
    let steps = [
        inverse.apply_vector([0.9 * f64::from(width), 0.0]),
        inverse.apply_vector([0.0, 0.9 * f64::from(height)]),
    ];
    let half = f64::from(grid - 1) / 2.0;
    let mut frames = Vec::new();
    for row in 0..grid {
        for column in 0..grid {
            let (i, j) = (f64::from(column) - half, f64::from(row) - half);
            let truth = [
                corner[0] + i * steps[0][0] + j * steps[1][0],
                corner[1] + i * steps[0][1] + j * steps[1][1],
            ];
            // The stage's report: half a µm off, a different way each time.
            let angle = f64::from(row * grid + column) * 2.4;
            let reported = [truth[0] + 0.5 * angle.cos(), truth[1] + 0.5 * angle.sin()];
            let samples: Vec<u16> = (0..height)
                .flat_map(|y| (0..width).map(move |x| (x, y)))
                .map(|(x, y)| {
                    let d = [
                        f64::from(x) + 0.5 - f64::from(width) / 2.0,
                        f64::from(y) + 0.5 - f64::from(height) / 2.0,
                    ];
                    let [dx, dy] = inverse.apply_vector(d);
                    let at = Vec2Um::from_um(truth[0] + dx, truth[1] + dy);
                    (1000.0 + 40_000.0 * world.intensity(at, Micrometers(0.376))) as u16
                })
                .collect();
            frames.push((samples, reported));
        }
    }
    println!("{} frames of {width} × {height} made", frames.len());

    let (count, painted, pages, bounds, region, size) = handle.run_blocking(move |gpu| {
        let mut canvas = Canvas::new(CanvasOptions::default());
        let began = Instant::now();
        for (samples, pose) in &frames {
            let m = frame_to_canvas(&l, [width, height], 1, *pose, &geometry).expect("invertible");
            let frame = Frame {
                samples,
                width,
                height,
            };
            canvas.paint(gpu, frame, &m)?;
        }
        let painted = began.elapsed();
        let [x0, y0, x1, y1] = canvas.bounds().expect("painted");
        let origin = [x0.floor() as i64, y0.floor() as i64];
        let size = [(x1 - x0).ceil() as u32, (y1 - y0).ceil() as u32];
        let region = canvas.region(gpu, origin, size)?;
        Ok((
            frames.len(),
            painted,
            canvas.on_gpu(),
            [x0, y0, x1, y1],
            region,
            size,
        ))
    })?;
    println!(
        "painted in {:.1?}, {:.1?} a frame, on {pages} pages; the mosaic {:.0} × {:.0} canvas pixels",
        painted,
        painted / count as u32,
        bounds[2] - bounds[0],
        bounds[3] - bounds[1]
    );

    // The region shrunk to at most 1600 pixels across, its painted values stretched to 8 bits.
    let factor = size[0].div_ceil(1600).max(1);
    let (w, h) = (size[0] / factor, size[1] / factor);
    let values: Vec<u16> = region.iter().map(|&word| word as u16).collect();
    let painted_values = values
        .iter()
        .zip(&region)
        .filter(|(_, word)| **word >> 16 > 0);
    let (low, high) = painted_values.fold((u16::MAX, 0), |(l, h), (&v, _)| (l.min(v), h.max(v)));
    let span = f64::from(high.saturating_sub(low).max(1));
    let mut pixels = Vec::with_capacity((w * h) as usize);
    for y in 0..h {
        for x in 0..w {
            let at = ((y * factor) * size[0] + x * factor) as usize;
            let value = f64::from(values[at].saturating_sub(low)) / span;
            pixels.push((value.clamp(0.0, 1.0) * 255.0) as u8);
        }
    }
    let mut encoder = png::Encoder::new(BufWriter::new(File::create(&out)?), w, h);
    encoder.set_color(png::ColorType::Grayscale);
    encoder.write_header()?.write_image_data(&pixels)?;
    println!("the viewport, {w} × {h}, in {}", out.display());
    Ok(())
}
