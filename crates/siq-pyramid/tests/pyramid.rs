//! The pyramid (ADR-0044): every level against a naive reference, on odd sizes, in `u16` and
//! `f32`; the level count; the metadata as built; and cancelling. REQ-DATA-003, REQ-DATA-005.

use std::sync::atomic::{AtomicUsize, Ordering};

use siq_pyramid::*;
use siq_storage::zarr::{ChannelInfo, ImageInfo, SampleType, Size5, ZarrImage, ZarrSample};
use siq_types::{Cfa, OwnedImage, RectPx};

fn info(size: Size5, sample: SampleType, chunk: u64) -> ImageInfo {
    let channels = (0..size.c)
        .map(|c| ChannelInfo {
            label: format!("c{c}"),
            colour: [255, 255, 255],
            window: (0.0, 1000.0),
        })
        .collect();
    ImageInfo {
        name: "pyramid".to_owned(),
        sample,
        size,
        levels: 1,
        chunk,
        pixel_size: [1.0, 0.5, 0.5],
        origin: [0.0; 3],
        channels,
        cfa: None,
    }
}

/// Writes level 0 of every plane from `pixel(plane, x, y)`.
fn write_level_0<T: ZarrSample>(image: &ZarrImage, pixel: impl Fn(u64, u64, u64) -> T) {
    let info = image.info().clone();
    let (rows, cols) = info.chunks(0);
    for c in 0..info.size.c {
        for row in 0..rows {
            for col in 0..cols {
                let (x0, y0) = (col * info.chunk, row * info.chunk);
                let w = info.chunk.min(info.size.x - x0) as usize;
                let h = info.chunk.min(info.size.y - y0) as usize;
                let chunk =
                    OwnedImage::from_fn(w, h, |x, y| pixel(c, x0 + x as u64, y0 + y as u64));
                image
                    .write_chunk(0, (0, c, 0), (row, col), chunk.view())
                    .unwrap();
            }
        }
    }
}

/// The naive pyramid of a whole plane, in memory: each level from the one before, block by block.
fn naive<T: Mean>(
    mut level: Vec<T>,
    mut width: usize,
    mut height: usize,
    levels: usize,
) -> Vec<Vec<T>> {
    let mut all = vec![level.clone()];
    for _ in 1..levels {
        let (w, h) = (width.div_ceil(2), height.div_ceil(2));
        let mut next = Vec::with_capacity(w * h);
        for y in 0..h {
            for x in 0..w {
                let mut block = Vec::new();
                for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    let (sx, sy) = (2 * x + dx, 2 * y + dy);
                    if sx < width && sy < height {
                        block.push(level[sy * width + sx]);
                    }
                }
                next.push(T::mean(&block));
            }
        }
        (level, width, height) = (next, w, h);
        all.push(level.clone());
    }
    all
}

fn whole<T: ZarrSample>(image: &ZarrImage, level: usize, c: u64) -> Vec<T> {
    let size = image.info().size(level);
    let rect = RectPx::new(0, 0, size.x as u32, size.y as u32);
    image
        .read_region::<T>(level, (0, c, 0), rect)
        .unwrap()
        .pixels
}

#[test]
fn the_levels_go_on_until_the_image_fits_the_top() {
    assert_eq!(levels_for(256, 100, 256), 1);
    assert_eq!(levels_for(257, 100, 256), 2);
    assert_eq!(levels_for(20_000, 20_000, 256), 8);
    assert_eq!(levels_for(11_648, 8742, TOP), 7, "a snap: 11648 → 182");
    assert_eq!(levels_for(300, 173, 16), 6, "300, 150, 75, 38, 19, 10");
}

#[test]
fn every_level_of_an_odd_sized_u16_image_is_the_naive_pyramid() {
    let folder = tempfile::tempdir().unwrap();
    let size = Size5 {
        t: 1,
        c: 2,
        z: 1,
        y: 173,
        x: 300,
    };
    let mut image = ZarrImage::create(
        &folder.path().join("p.ome.zarr"),
        info(size, SampleType::U16, 64),
    )
    .unwrap();
    let pixel = |c: u64, x: u64, y: u64| ((x * 37 + y * 101 + c * 9001) % 4093) as u16;
    write_level_0(&image, pixel);
    build(&mut image, 16, &()).unwrap();
    assert_eq!(image.info().levels, 6);

    let image = ZarrImage::open(image.path()).unwrap();
    for c in 0..2 {
        let level_0: Vec<u16> = (0..173)
            .flat_map(|y| (0..300).map(move |x| pixel(c, x, y)))
            .collect();
        let expected = naive(level_0, 300, 173, 6);
        for (level, expected) in expected.iter().enumerate() {
            assert_eq!(
                &whole::<u16>(&image, level, c),
                expected,
                "level {level} of channel {c}"
            );
        }
    }
}

#[test]
fn a_float_image_s_levels_are_exact_means() {
    let folder = tempfile::tempdir().unwrap();
    let size = Size5 {
        t: 1,
        c: 1,
        z: 1,
        y: 129,
        x: 131,
    };
    let mut image = ZarrImage::create(
        &folder.path().join("f.ome.zarr"),
        info(size, SampleType::F32, 32),
    )
    .unwrap();
    let pixel = |_: u64, x: u64, y: u64| x as f32 * 0.25 - y as f32;
    write_level_0(&image, pixel);
    build(&mut image, 32, &()).unwrap();
    let level_0: Vec<f32> = (0..129)
        .flat_map(|y| (0..131).map(move |x| pixel(0, x, y)))
        .collect();
    let expected = naive(level_0, 131, 129, image.info().levels);
    assert_eq!(expected.len(), 4, "131, 66, 33, 17");
    for (level, expected) in expected.iter().enumerate() {
        assert_eq!(&whole::<f32>(&image, level, 0), expected, "level {level}");
    }
    // The 2 × 2 block at the top left of level 1: (0 + 0.25 − 1 + −0.75) / 4.
    assert_eq!(whole::<f32>(&image, 1, 0)[0], -0.375);
}

#[test]
fn the_metadata_lists_every_level_built() {
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("m.ome.zarr");
    let size = Size5 {
        t: 1,
        c: 1,
        z: 1,
        y: 200,
        x: 300,
    };
    let mut image = ZarrImage::create(&path, info(size, SampleType::U16, 64)).unwrap();
    write_level_0(&image, |_, x, y| (x + y) as u16);
    build(&mut image, 64, &()).unwrap();
    let group: serde_json::Value =
        serde_json::from_slice(&std::fs::read(path.join("zarr.json")).unwrap()).unwrap();
    insta::assert_json_snapshot!(
        "datasets",
        group["attributes"]["ome"]["multiscales"][0]["datasets"]
    );
    let shapes: Vec<serde_json::Value> = (0..4)
        .map(|level| {
            let array: serde_json::Value = serde_json::from_slice(
                &std::fs::read(path.join(format!("{level}/zarr.json"))).unwrap(),
            )
            .unwrap();
            array["shape"].clone()
        })
        .collect();
    assert_eq!(
        shapes,
        [
            [1, 1, 1, 200, 300],
            [1, 1, 1, 100, 150],
            [1, 1, 1, 50, 75],
            [1, 1, 1, 25, 38]
        ]
        .map(|s| serde_json::json!(s))
    );
}

/// Stops after `after` chunks.
struct StopAfter {
    after: usize,
    reports: AtomicUsize,
}

impl Control for StopAfter {
    fn progress(&self, _: f64) {
        self.reports.fetch_add(1, Ordering::Relaxed);
    }

    fn cancelled(&self) -> bool {
        self.reports.load(Ordering::Relaxed) >= self.after
    }
}

#[test]
fn a_build_stops_when_cancelled() {
    let folder = tempfile::tempdir().unwrap();
    let size = Size5 {
        t: 1,
        c: 1,
        z: 1,
        y: 512,
        x: 512,
    };
    let mut image = ZarrImage::create(
        &folder.path().join("c.ome.zarr"),
        info(size, SampleType::U16, 32),
    )
    .unwrap();
    write_level_0(&image, |_, x, _| x as u16);
    let control = StopAfter {
        after: 3,
        reports: AtomicUsize::new(0),
    };
    assert!(matches!(
        build(&mut image, 16, &control),
        Err(PyramidError::Cancelled)
    ));
    assert!(
        control.reports.load(Ordering::Relaxed) < 85,
        "it stopped well before the 85 chunks"
    );
}

// ADR-0071: a colour image's levels keep its filter's cells whole, each pixel the mean of its
// colour in the 2 × 2 cells below its cell, at odd sizes and across chunks.
#[test]
fn a_colour_image_s_levels_keep_its_filter_s_cells_whole() {
    let folder = tempfile::tempdir().unwrap();
    let size = Size5 {
        t: 1,
        c: 1,
        z: 1,
        y: 173,
        x: 300,
    };
    let colour = ImageInfo {
        cfa: Some(Cfa::Grbg),
        ..info(size, SampleType::U16, 64)
    };
    let mut image = ZarrImage::create(&folder.path().join("c.ome.zarr"), colour).unwrap();
    let pixel = |_, x: u64, y: u64| ((x * 37 + y * 101) % 4093) as u16;
    write_level_0(&image, pixel);
    build(&mut image, 16, &()).unwrap();

    let (mut level, mut width, mut height): (Vec<u16>, usize, usize) = (
        (0..173)
            .flat_map(|y| (0..300).map(move |x| pixel(0, x, y)))
            .collect(),
        300,
        173,
    );
    for l in 1..image.info().levels {
        let (w, h) = (width.div_ceil(2), height.div_ceil(2));
        let mut next = Vec::new();
        for y in 0..h {
            for x in 0..w {
                // The same phase of the cell's own colour, in the 2 × 2 cells below.
                let mut block = Vec::new();
                for (i, j) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    let sx = 4 * (x / 2) + x % 2 + 2 * i;
                    let sy = 4 * (y / 2) + y % 2 + 2 * j;
                    if sx < width && sy < height {
                        assert_eq!(Cfa::Grbg.colour(sx, sy), Cfa::Grbg.colour(x, y));
                        block.push(level[sy * width + sx]);
                    }
                }
                next.push(u16::mean(&block));
            }
        }
        (level, width, height) = (next, w, h);
        assert_eq!(whole::<u16>(&image, l, 0), level, "level {l}");
    }
    // A colour image's chunks hold whole cells.
    let odd = ImageInfo {
        cfa: Some(Cfa::Rggb),
        ..info(size, SampleType::U16, 63)
    };
    assert!(ZarrImage::create(&folder.path().join("odd.ome.zarr"), odd).is_err());
}
