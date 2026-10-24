//! A real image as the world: PNG and TIFF, its pixel size, its pyramid, and bad files.

use std::fs::File;
use std::io::BufWriter;
use std::path::Path;

use siq_synth::*;
use siq_types::{Micrometers, OwnedImage, Vec2Um};

const WIDTH: u32 = 8;
const HEIGHT: u32 = 6;

/// A 16-bit ramp: pixel `(x, y)` is `1000 × (y × WIDTH + x) + 7`.
fn pixel(x: u32, y: u32) -> u16 {
    (1000 * (y * WIDTH + x) + 7) as u16
}

fn pixels() -> Vec<u16> {
    (0..HEIGHT)
        .flat_map(|y| (0..WIDTH).map(move |x| pixel(x, y)))
        .collect()
}

fn write_png(path: &Path) {
    let mut encoder = png::Encoder::new(BufWriter::new(File::create(path).unwrap()), WIDTH, HEIGHT);
    encoder.set_color(png::ColorType::Grayscale);
    encoder.set_depth(png::BitDepth::Sixteen);
    let bytes: Vec<u8> = pixels().iter().flat_map(|p| p.to_be_bytes()).collect();
    encoder
        .write_header()
        .unwrap()
        .write_image_data(&bytes)
        .unwrap();
}

fn write_tiff(path: &Path) {
    let mut encoder = tiff::encoder::TiffEncoder::new(File::create(path).unwrap()).unwrap();
    encoder
        .write_image::<tiff::encoder::colortype::Gray16>(WIDTH, HEIGHT, &pixels())
        .unwrap();
}

/// The image's pixels, sampled on its own grid: `scale` µm pixels from its top-left corner.
fn render(world: &ImageWorld, scale: f64, width: usize, height: usize) -> OwnedImage<f32> {
    let corner = Vec2Um::from_um(
        -f64::from(WIDTH) * 0.5 / 2.0,
        -f64::from(HEIGHT) * 0.5 / 2.0,
    );
    let mut image = OwnedImage::new(width, height);
    sample(
        world,
        &grid(corner, Micrometers(scale)),
        &mut image.view_mut(),
    );
    image
}

// REQ-SIM-005
#[test]
fn an_image_world_reads_png_and_tiff_and_lays_them_out_at_their_pixel_size() {
    let dir = tempfile::tempdir().unwrap();
    for name in ["ramp.png", "ramp.tif"] {
        let path = dir.path().join(name);
        if name.ends_with("png") {
            write_png(&path)
        } else {
            write_tiff(&path)
        }
        let params = ImageParams {
            path: path.clone(),
            pixel_size: Micrometers(0.5),
        };
        let world = ImageWorld::open(params.clone()).unwrap();
        assert_eq!(world.size(), (WIDTH as usize, HEIGHT as usize));
        assert_eq!(world.info(), WorldInfo::Image(params));

        let image = render(&world, 0.5, WIDTH as usize, HEIGHT as usize);
        for (y, row) in (0..).zip(image.view().rows()) {
            for (x, &value) in (0..).zip(row) {
                let expected = f32::from(pixel(x, y)) / 65535.0;
                assert!(
                    (value - expected).abs() < 1e-6,
                    "{name} ({x}, {y}): {value} ≠ {expected}"
                );
            }
        }
        // The image is centred on the origin, and dark outside.
        let centre = world.intensity(Vec2Um::from_um(0.25, 0.25), Micrometers(0.5));
        assert!((centre - f32::from(pixel(4, 3)) / 65535.0).abs() < 1e-6);
        assert_eq!(
            world.intensity(Vec2Um::from_um(10.0, 0.0), Micrometers(0.5)),
            0.0
        );
    }
}

// REQ-SIM-005
#[test]
fn a_coarse_sampling_sees_the_mean_of_each_block() {
    let image = OwnedImage::from_fn(WIDTH as usize, HEIGHT as usize, |x, y| {
        f32::from(pixel(x as u32, y as u32)) / 65535.0
    });
    let params = ImageParams {
        path: "ramp".into(),
        pixel_size: Micrometers(0.5),
    };
    let world = ImageWorld::from_image(params, image.clone());
    let halved = render(&world, 1.0, 4, 3);
    let source = image.view();
    for (y, row) in halved.view().rows().enumerate() {
        for (x, &value) in row.iter().enumerate() {
            let block =
                [(0, 0), (1, 0), (0, 1), (1, 1)].map(|(dx, dy)| source.row(2 * y + dy)[2 * x + dx]);
            let mean = block.iter().sum::<f32>() / 4.0;
            assert!((value - mean).abs() < 1e-6, "({x}, {y}): {value} ≠ {mean}");
        }
    }
    // Between two levels, the value is between them.
    let at = Vec2Um::from_um(0.1, -0.3);
    let (fine, coarse) = (
        world.intensity(at, Micrometers(0.5)),
        world.intensity(at, Micrometers(1.0)),
    );
    let between = world.intensity(at, Micrometers(0.7));
    assert!(fine.min(coarse) - 1e-6 <= between && between <= fine.max(coarse) + 1e-6);
}

// REQ-SIM-005
#[test]
fn a_missing_or_foreign_file_is_an_error_naming_it() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("missing.png");
    let open = |path: &Path| {
        ImageWorld::open(ImageParams {
            path: path.into(),
            pixel_size: Micrometers(1.0),
        })
    };
    assert!(matches!(open(&missing), Err(SynthError::Io { .. })));
    let text = dir.path().join("notes.tif");
    std::fs::write(&text, "not an image at all").unwrap();
    let error = open(&text).unwrap_err();
    assert!(
        matches!(error, SynthError::Image { .. }) && error.to_string().contains("notes.tif"),
        "{error}"
    );
}
