//! OME-Zarr images (ADR-0042): round trips on odd sizes, chunks clipped at the edges, the NGFF
//! metadata, writers storing disjoint chunks at once, and whole planes. REQ-DATA-003.

use rayon::prelude::*;
use siq_storage::zarr::*;
use siq_types::{OwnedImage, RectPx};

fn info(size: Size5, sample: SampleType, chunk: u64, levels: usize) -> ImageInfo {
    let channels = (0..size.c)
        .map(|c| ChannelInfo {
            label: format!("channel {c}"),
            colour: [[0, 0, 255], [0, 255, 0], [255, 0, 0]][c as usize % 3],
            window: (100.0, 16000.0),
        })
        .collect();
    ImageInfo {
        name: "test".to_owned(),
        sample,
        size,
        levels,
        chunk,
        pixel_size: [2.0, 0.376, 0.376],
        origin: [10.0, -500.0, 250.0],
        channels,
    }
}

/// Plane (`c`, `z`)'s pixel at (`x`, `y`): different everywhere.
fn value(c: u64, z: u64, x: u64, y: u64) -> u16 {
    ((c * 7919 + z * 104_729 + y * 1000 + x) % 65_521) as u16
}

/// Writes every chunk of every plane of level 0, as `value` gives them.
fn fill_u16(image: &ZarrImage) {
    let info = image.info().clone();
    let (rows, cols) = info.chunks(0);
    for c in 0..info.size.c {
        for z in 0..info.size.z {
            for (row, col) in (0..rows).flat_map(|row| (0..cols).map(move |col| (row, col))) {
                let (y0, x0) = (row * info.chunk, col * info.chunk);
                let height = info.chunk.min(info.size.y - y0) as usize;
                let width = info.chunk.min(info.size.x - x0) as usize;
                let chunk = OwnedImage::from_fn(width, height, |x, y| {
                    value(c, z, x0 + x as u64, y0 + y as u64)
                });
                image
                    .write_chunk(0, (0, c, z), (row, col), chunk.view())
                    .unwrap();
            }
        }
    }
}

#[test]
fn an_odd_sized_image_reads_back_as_written_anywhere() {
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("odd.ome.zarr");
    let size = Size5 {
        t: 1,
        c: 2,
        z: 3,
        y: 1000,
        x: 777,
    };
    let image = ZarrImage::create(&path, info(size, SampleType::U16, 256, 1)).unwrap();
    assert_eq!(image.info().chunks(0), (4, 4));
    fill_u16(&image);

    let image = ZarrImage::open(&path).unwrap();
    assert_eq!(image.info(), &info(size, SampleType::U16, 256, 1));
    for rect in [
        RectPx::new(0, 0, 777, 1000),   // the whole plane
        RectPx::new(250, 250, 300, 20), // across chunks
        RectPx::new(770, 990, 7, 10),   // in the bottom-right corner's partial chunk
    ] {
        let region = image.read_region::<u16>(0, (0, 1, 2), rect).unwrap();
        let view = region.view();
        assert_eq!(
            (view.width(), view.height()),
            (rect.width as usize, rect.height as usize)
        );
        for (y, row) in view.rows().enumerate() {
            for (x, &pixel) in row.iter().enumerate() {
                let (x, y) = (u64::from(rect.x) + x as u64, u64::from(rect.y) + y as u64);
                assert_eq!(pixel, value(1, 2, x, y), "({x}, {y}) of {rect:?}");
            }
        }
    }
}

#[test]
fn edge_chunks_are_given_clipped_and_read_as_zeros_beyond_what_was_written() {
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("edges.ome.zarr");
    let size = Size5 {
        t: 1,
        c: 1,
        z: 1,
        y: 300,
        x: 300,
    };
    let image = ZarrImage::create(&path, info(size, SampleType::U16, 256, 1)).unwrap();
    // The bottom-right chunk is 44 × 44; a whole one there is refused.
    let whole = OwnedImage::<u16>::new(256, 256);
    assert!(
        image
            .write_chunk(0, (0, 0, 0), (1, 1), whole.view())
            .is_err()
    );
    let corner = OwnedImage::from_fn(44, 44, |x, y| (x + y) as u16 + 1);
    image
        .write_chunk(0, (0, 0, 0), (1, 1), corner.view())
        .unwrap();
    let region = image
        .read_region::<u16>(0, (0, 0, 0), RectPx::new(250, 250, 50, 50))
        .unwrap();
    let view = region.view();
    assert_eq!(
        view.row(0)[..6],
        [0; 6],
        "the chunk to the left wasn't written"
    );
    assert_eq!(view.row(6)[6], 1);
    assert_eq!(view.row(49)[49], 44 + 44 - 1);
    // Out of the image, or of another type, is an error.
    assert!(
        image
            .read_region::<u16>(0, (0, 0, 0), RectPx::new(290, 0, 20, 1))
            .is_err()
    );
    assert!(
        image
            .read_region::<f32>(0, (0, 0, 0), RectPx::new(0, 0, 1, 1))
            .is_err()
    );
}

#[test]
fn a_float_image_has_its_levels_halved_rounding_up() {
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("float.ome.zarr");
    let size = Size5 {
        t: 2,
        c: 1,
        z: 1,
        y: 513,
        x: 1025,
    };
    let image = ZarrImage::create(&path, info(size, SampleType::F32, 512, 4)).unwrap();
    let sizes: Vec<(u64, u64)> = (0..4)
        .map(|l| image.info().size(l))
        .map(|s| (s.y, s.x))
        .collect();
    assert_eq!(sizes, [(513, 1025), (257, 513), (129, 257), (65, 129)]);
    let plane = OwnedImage::from_fn(129, 65, |x, y| x as f32 * 0.5 - y as f32);
    image
        .write_chunk(3, (1, 0, 0), (0, 0), plane.view())
        .unwrap();
    let image = ZarrImage::open(&path).unwrap();
    assert_eq!(
        (image.info().levels, image.info().sample),
        (4, SampleType::F32)
    );
    let region = image
        .read_region::<f32>(3, (1, 0, 0), RectPx::new(128, 64, 1, 1))
        .unwrap();
    assert_eq!(region.pixels, [64.0 - 64.0]);
    let untouched = image
        .read_region::<f32>(3, (0, 0, 0), RectPx::new(0, 0, 2, 1))
        .unwrap();
    assert_eq!(untouched.pixels, [0.0, 0.0]);
}

#[test]
fn the_ngff_metadata_is_as_napari_and_fiji_read_it() {
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("meta.ome.zarr");
    let size = Size5 {
        t: 1,
        c: 2,
        z: 1,
        y: 2048,
        x: 3000,
    };
    ZarrImage::create(&path, info(size, SampleType::U16, 512, 3)).unwrap();
    let read = |file: &str| -> serde_json::Value {
        serde_json::from_slice(&std::fs::read(path.join(file)).unwrap()).unwrap()
    };
    insta::assert_json_snapshot!("group", read("zarr.json"));
    insta::assert_json_snapshot!("level_1", read("1/zarr.json"));
}

#[test]
fn disjoint_chunks_are_written_at_once_from_many_threads() {
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("parallel.ome.zarr");
    let size = Size5 {
        t: 1,
        c: 1,
        z: 1,
        y: 2048,
        x: 2048,
    };
    let image = ZarrImage::create(&path, info(size, SampleType::U16, 256, 1)).unwrap();
    let chunks: Vec<(u64, u64)> = (0..8)
        .flat_map(|row| (0..8).map(move |col| (row, col)))
        .collect();
    // In an order of the caller's choosing, on every core.
    chunks.par_iter().rev().for_each(|&(row, col)| {
        let chunk = OwnedImage::from_fn(256, 256, |x, y| {
            value(0, 0, col * 256 + x as u64, row * 256 + y as u64)
        });
        image
            .write_chunk(0, (0, 0, 0), (row, col), chunk.view())
            .unwrap();
    });
    let all = image
        .read_region::<u16>(0, (0, 0, 0), RectPx::new(0, 0, 2048, 2048))
        .unwrap();
    for (y, row) in all.view().rows().enumerate() {
        assert!(
            row.iter()
                .enumerate()
                .all(|(x, &p)| p == value(0, 0, x as u64, y as u64))
        );
    }
}

#[test]
fn a_whole_plane_is_written_at_once_and_reads_back_as_given() {
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("plane.ome.zarr");
    let size = Size5 {
        t: 1,
        c: 2,
        z: 1,
        y: 700,
        x: 1100,
    };
    let image = ZarrImage::create(&path, info(size, SampleType::U16, 256, 1)).unwrap();
    for c in 0..2 {
        let plane = OwnedImage::from_fn(1100, 700, |x, y| value(c, 0, x as u64, y as u64));
        image.write_plane(0, (0, c, 0), plane.view()).unwrap();
    }
    let wrong = OwnedImage::from_fn(1000, 700, |_, _| 0u16);
    assert!(image.write_plane(0, (0, 0, 0), wrong.view()).is_err());

    let image = ZarrImage::open(&path).unwrap();
    for c in 0..2 {
        let whole = image
            .read_region::<u16>(0, (0, c, 0), RectPx::new(0, 0, 1100, 700))
            .unwrap();
        for (k, &pixel) in whole.pixels.iter().enumerate() {
            let (x, y) = (k as u64 % 1100, k as u64 / 1100);
            assert_eq!(pixel, value(c, 0, x, y), "channel {c} at ({x}, {y})");
        }
    }
}
