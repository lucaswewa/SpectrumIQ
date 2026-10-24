//! Renders a region of each world to a 16-bit PNG, with ground truth beside it, then reads the
//! cells' PNG back as an image world and renders that at half the resolution.
//!
//! ```text
//! cargo run --release -p siq-synth --example render_world [-- <folder>]
//! ```
//!
//! The folder defaults to `target/render_world`. The full run renders 2048 × 1536 pixels of
//! 0.5 µm per world, and of 0.1 µm for the beads; smoke mode (`SIQ_SMOKE=1`) renders 256 × 192.

use std::fs::{self, File};
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::time::Instant;

use siq_synth::*;
use siq_types::{Micrometers, OwnedImage, RectUm, Vec2Um};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let smoke = std::env::var_os("SIQ_SMOKE").is_some();
    let (width, height) = if smoke { (256, 192) } else { (2048, 1536) };
    let folder = std::env::args_os()
        .nth(1)
        .map_or_else(|| PathBuf::from("target/render_world"), PathBuf::from);
    fs::create_dir_all(&folder)?;
    let scale = Micrometers(0.5);
    let size = Vec2Um::from_um(width as f64 * scale.0, height as f64 * scale.0);
    let region = RectUm {
        origin: size * -0.5,
        size,
    };

    let worlds: [(&str, Box<dyn World>); 4] = [
        (
            "cells",
            Box::new(Cells::new(CellsParams {
                seed: 1,
                ..CellsParams::default()
            })),
        ),
        (
            "grains",
            Box::new(Grains::new(GrainsParams {
                seed: 1,
                ..GrainsParams::default()
            })),
        ),
        (
            "height_field",
            Box::new(HeightField::new(HeightFieldParams {
                seed: 1,
                ..HeightFieldParams::default()
            })),
        ),
        (
            "beads",
            Box::new(Beads::new(BeadsParams {
                seed: 1,
                ..BeadsParams::default()
            })),
        ),
    ];
    for (name, world) in &worlds {
        // Sub-resolution beads are single pixels at 0.5 µm: they are shown at 0.1 µm.
        let (region, scale) = match *name {
            "beads" => (
                RectUm {
                    origin: region.origin * 0.2,
                    size: region.size * 0.2,
                },
                scale * 0.2,
            ),
            _ => (region, scale),
        };
        let started = Instant::now();
        let image = render(world.as_ref(), region, scale, width, height);
        let elapsed = started.elapsed();
        let path = folder.join(format!("{name}.png"));
        write_png(&path, &image)?;
        let truth = write_ground_truth(world.as_ref(), region, Micrometers(4.0), &folder, name)?;
        let extra = match (&truth.height_map, truth.beads.len()) {
            (Some(map), _) => format!(", a {}×{} height map", map.width, map.height),
            (None, beads) => format!(", {beads} beads"),
        };
        println!(
            "{name:<13} {width}×{height} in {elapsed:.0?} → {}{extra}",
            path.display()
        );
    }

    // The cells, back as an image world at their pixel size, rendered at twice the pixel size.
    let params = ImageParams {
        path: folder.join("cells.png"),
        pixel_size: scale,
    };
    let world = ImageWorld::open(params)?;
    let image = render(&world, region, scale * 2.0, width / 2, height / 2);
    let path = folder.join("cells_as_image_halved.png");
    write_png(&path, &image)?;
    println!(
        "image world   {:?} px at {scale} → {}",
        world.size(),
        path.display()
    );
    Ok(())
}

fn render(
    world: &dyn World,
    region: RectUm,
    scale: Micrometers,
    width: usize,
    height: usize,
) -> OwnedImage<f32> {
    let mut image = OwnedImage::new(width, height);
    sample(world, &grid(region.origin, scale), &mut image.view_mut());
    image
}

fn write_png(path: &Path, image: &OwnedImage<f32>) -> Result<(), Box<dyn std::error::Error>> {
    let file = BufWriter::new(File::create(path)?);
    let mut encoder = png::Encoder::new(file, image.width() as u32, image.height() as u32);
    encoder.set_color(png::ColorType::Grayscale);
    encoder.set_depth(png::BitDepth::Sixteen);
    let pixels = image
        .view()
        .rows()
        .flatten()
        .map(|&v| (v.clamp(0.0, 1.0) * 65535.0).round() as u16);
    let bytes: Vec<u8> = pixels.flat_map(u16::to_be_bytes).collect();
    encoder.write_header()?.write_image_data(&bytes)?;
    Ok(())
}
