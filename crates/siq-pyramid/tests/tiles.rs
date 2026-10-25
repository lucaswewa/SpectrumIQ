//! Deep-zoom tiles (ADR-0045): the DZI geometry against OpenSeadragon's, tiles rendered and
//! decoded back, levels beyond the top, the cache's eviction and ETags, concurrent requests, and
//! the value probe. REQ-DATA-005, REQ-NET-003, REQ-VIS-001.

use std::sync::Arc;

use proptest::prelude::*;
use siq_imgproc::Colormap;
use siq_pyramid::build;
use siq_pyramid::tiles::*;
use siq_storage::zarr::{ChannelInfo, ImageInfo, SampleType, Size5, ZarrImage};
use siq_types::{OwnedImage, RectPx};

/// OpenSeadragon's `DziTileSource`, as its JavaScript computes it, written apart from ours.
mod openseadragon {
    pub fn max_level(width: u64, height: u64) -> u32 {
        (width.max(height) as f64).log2().ceil() as u32
    }

    pub fn level_size(width: u64, height: u64, level: u32) -> (u64, u64) {
        let scale = 0.5f64.powi((max_level(width, height) - level) as i32);
        (
            (width as f64 * scale).ceil() as u64,
            (height as f64 * scale).ceil() as u64,
        )
    }

    /// `getTileBounds`, in pixels: x, y, width, height.
    pub fn tile_bounds(
        level_width: u64,
        level_height: u64,
        x: u64,
        y: u64,
    ) -> (u64, u64, u64, u64) {
        let (tile, overlap) = (254, 1);
        let px = if x == 0 { 0 } else { tile * x - overlap };
        let py = if y == 0 { 0 } else { tile * y - overlap };
        let sx = (tile + if x == 0 { 1 } else { 2 } * overlap).min(level_width - px);
        let sy = (tile + if y == 0 { 1 } else { 2 } * overlap).min(level_height - py);
        (px, py, sx, sy)
    }
}

proptest! {
    #[test]
    fn the_geometry_is_openseadragon_s(width in 2u64..300_000, height in 2u64..300_000) {
        let dzi = DziGeometry { width, height };
        prop_assert_eq!(dzi.max_level(), openseadragon::max_level(width, height));
        prop_assert_eq!(dzi.level_size(dzi.max_level()), (width, height));
        prop_assert_eq!(dzi.level_size(0), (1, 1));
        for level in [0, dzi.max_level() / 2, dzi.max_level().saturating_sub(1), dzi.max_level()] {
            let (lw, lh) = dzi.level_size(level);
            prop_assert_eq!((lw, lh), openseadragon::level_size(width, height, level));
            let (cols, rows) = dzi.tiles(level);
            prop_assert_eq!((cols, rows), (lw.div_ceil(254), lh.div_ceil(254)));
            // The first, a middle and the last tile, edges included.
            for (col, row) in [(0, 0), (cols / 2, rows / 2), (cols - 1, rows - 1)] {
                let rect = dzi.tile_rect(level, col, row).unwrap();
                let (x, y, w, h) = openseadragon::tile_bounds(lw, lh, col, row);
                prop_assert_eq!((u64::from(rect.x), u64::from(rect.y)), (x, y));
                prop_assert_eq!((u64::from(rect.width), u64::from(rect.height)), (w, h));
                prop_assert!(rect.right() <= lw && rect.bottom() <= lh);
            }
            // The last tile reaches the level's edge; there is none after it.
            prop_assert_eq!(dzi.tile_rect(level, cols - 1, 0).unwrap().right(), lw);
            prop_assert!(dzi.tile_rect(level, cols, 0).is_none());
        }
    }
}

#[test]
fn neighbouring_tiles_share_two_pixels_and_levels_match_zarr_s() {
    let dzi = DziGeometry {
        width: 1000,
        height: 600,
    };
    assert_eq!(dzi.max_level(), 10);
    let (a, b) = (
        dzi.tile_rect(10, 0, 0).unwrap(),
        dzi.tile_rect(10, 1, 0).unwrap(),
    );
    assert_eq!((a.x, a.width, b.x, b.width), (0, 255, 253, 256));
    assert_eq!(a.right() - u64::from(b.x), 2);
    // DZI level 10 − k is Zarr level k, which halves rounding up as DZI does.
    assert_eq!(dzi.level_size(9), (500, 300));
    assert_eq!(dzi.level_size(8), (250, 150));
    let descriptor = dzi.descriptor(TileFormat::Jpeg);
    assert!(
        descriptor.contains(r#"TileSize="254" Overlap="1" Format="jpeg""#),
        "{descriptor}"
    );
    assert!(descriptor.contains(r#"<Size Width="1000" Height="600"/>"#));
}

/// A 1000 × 600 image, two channels, its pyramid built: channel 1 is `x + y`.
fn image(folder: &std::path::Path) -> ZarrImage {
    let path = folder.join("tiles.ome.zarr");
    let channel = |label: &str, colour| ChannelInfo {
        label: label.to_owned(),
        colour,
        window: (0.0, 2000.0),
    };
    let info = ImageInfo {
        name: "tiles".to_owned(),
        sample: SampleType::U16,
        size: Size5 {
            t: 1,
            c: 2,
            z: 1,
            y: 600,
            x: 1000,
        },
        levels: 1,
        chunk: 256,
        pixel_size: [1.0, 0.5, 0.5],
        origin: [0.0; 3],
        channels: vec![
            channel("white", [255, 255, 255]),
            channel("green", [0, 255, 0]),
        ],
    };
    let mut image = ZarrImage::create(&path, info).unwrap();
    for c in 0..2u64 {
        for (row, col) in (0..3).flat_map(|row| (0..4).map(move |col| (row, col))) {
            let (x0, y0) = (col * 256, row * 256);
            let (w, h) = (256.min(1000 - x0), 256.min(600 - y0));
            let chunk = OwnedImage::from_fn(w as usize, h as usize, |x, y| {
                ((x0 + x as u64 + y0 + y as u64) * c) as u16
            });
            image
                .write_chunk(0, (0, c, 0), (row, col), chunk.view())
                .unwrap();
        }
    }
    build(&mut image, 256, &()).unwrap();
    image
}

/// A PNG's pixels and colour type.
fn decode(png_bytes: &[u8]) -> (Vec<u8>, png::ColorType, u32, u32) {
    let mut reader = png::Decoder::new(std::io::Cursor::new(png_bytes))
        .read_info()
        .unwrap();
    let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
    let frame = reader.next_frame(&mut pixels).unwrap();
    pixels.truncate(frame.buffer_size());
    (pixels, frame.color_type, frame.width, frame.height)
}

#[test]
fn a_tile_is_its_level_s_region_through_the_lut() {
    let folder = tempfile::tempdir().unwrap();
    let image = image(folder.path());
    let lut = TileLut {
        min: 0.0,
        max: 1000.0,
        gamma: 1.0,
        colormap: Colormap::Grey,
    };
    // DZI level 9 is Zarr level 1, 500 × 300: tile (1, 0) starts at x = 253, 247 wide.
    let at = TileAt {
        plane: (0, 1, 0),
        level: 9,
        col: 1,
        row: 0,
    };
    let (pixels, colour, width, height) =
        decode(&render(&image, at, Some(lut), TileFormat::Png).unwrap());
    assert_eq!(
        (colour, width, height),
        (png::ColorType::Grayscale, 247, 255)
    );
    let region = image
        .read_region::<u16>(1, (0, 1, 0), RectPx::new(253, 0, 247, 255))
        .unwrap();
    let expected: Vec<u8> = region
        .pixels
        .iter()
        .map(|&v| ((f64::from(v) / 1000.0).min(1.0) * 255.0).round() as u8)
        .collect();
    assert_eq!(pixels, expected);

    // Without a LUT, the channel's own: its window, in green.
    let (rgb, colour, ..) = decode(&render(&image, at, None, TileFormat::Png).unwrap());
    assert_eq!(colour, png::ColorType::Rgb);
    let first = ((f64::from(region.pixels[0]) / 2000.0) * 255.0).round() as u32;
    assert_eq!(rgb[..3], [0, ((255 * first + 127) / 255) as u8, 0]);

    // JPEG, opaque, for the viewer.
    let jpeg = render(&image, at, Some(lut), TileFormat::Jpeg).unwrap();
    assert_eq!(
        (jpeg[..2].to_vec(), jpeg[jpeg.len() - 2..].to_vec()),
        (vec![0xFF, 0xD8], vec![0xFF, 0xD9])
    );
}

#[test]
fn levels_beyond_the_top_are_halved_from_it_down_to_one_pixel() {
    let folder = tempfile::tempdir().unwrap();
    let image = image(folder.path());
    assert_eq!(image.info().levels, 3, "1000, 500, 250");
    let lut = TileLut {
        min: 0.0,
        max: 2000.0,
        gamma: 1.0,
        colormap: Colormap::Grey,
    };
    // DZI level 7 is Zarr level 3, a halving of the top: 125 × 75.
    let at = TileAt {
        plane: (0, 1, 0),
        level: 7,
        col: 0,
        row: 0,
    };
    let (pixels, _, width, height) =
        decode(&render(&image, at, Some(lut), TileFormat::Png).unwrap());
    assert_eq!((width, height), (125, 75));
    // Pixel (0, 0) of level 3 is the mean of level 0's top-left 8 × 8: x + y from 0 to 14, mean 7.
    assert_eq!(pixels[0], ((7.0 / 2000.0) * 255.0f64).round() as u8);
    let one = TileAt {
        plane: (0, 1, 0),
        level: 0,
        col: 0,
        row: 0,
    };
    assert_eq!(
        decode(&render(&image, one, Some(lut), TileFormat::Png).unwrap()).2,
        1
    );
}

#[test]
fn bad_requests_and_missing_tiles_are_told_apart() {
    let folder = tempfile::tempdir().unwrap();
    let image = image(folder.path());
    let at = TileAt {
        plane: (0, 0, 0),
        level: 9,
        col: 0,
        row: 0,
    };
    let lut = |min, max, gamma| {
        Some(TileLut {
            min,
            max,
            gamma,
            colormap: Colormap::Grey,
        })
    };
    for bad in [
        lut(10.0, 10.0, 1.0),
        lut(0.0, f64::NAN, 1.0),
        lut(0.0, 1.0, 0.0),
        lut(0.0, 1.0, 50.0),
    ] {
        assert!(
            matches!(
                render(&image, at, bad, TileFormat::Jpeg),
                Err(TileError::BadRequest(_))
            ),
            "{bad:?}"
        );
    }
    for missing in [
        TileAt { level: 11, ..at },
        TileAt { col: 2, ..at },
        TileAt {
            plane: (0, 2, 0),
            ..at
        },
        TileAt {
            plane: (1, 0, 0),
            ..at
        },
    ] {
        assert!(
            matches!(
                render(&image, missing, None, TileFormat::Jpeg),
                Err(TileError::NotFound(_))
            ),
            "{missing:?}"
        );
    }
}

#[test]
fn the_value_probe_gives_the_true_pixel() {
    let folder = tempfile::tempdir().unwrap();
    let image = image(folder.path());
    assert_eq!(value(&image, (0, 1, 0), 0, 999, 599).unwrap(), 1598.0);
    // Level 1's pixel (0, 0): the mean of x + y over 2 × 2, (0 + 1 + 1 + 2) / 4 = 1.
    assert_eq!(value(&image, (0, 1, 0), 1, 0, 0).unwrap(), 1.0);
    assert!(matches!(
        value(&image, (0, 1, 0), 0, 1000, 0),
        Err(TileError::NotFound(_))
    ));
    assert!(matches!(
        value(&image, (0, 1, 0), 3, 0, 0),
        Err(TileError::NotFound(_))
    ));
}

#[test]
fn the_cache_keeps_the_most_recent_tiles_within_its_budget() {
    let folder = tempfile::tempdir().unwrap();
    let image = image(folder.path());
    let path = image.path().to_owned();
    let tile = |col| TileAt {
        plane: (0, 1, 0),
        level: 10,
        col,
        row: 0,
    };
    let size = render(&image, tile(0), None, TileFormat::Jpeg)
        .unwrap()
        .len();
    // Room for about two tiles.
    let service = TileService::new(size * 5 / 2);
    let first = service
        .tile(&path, tile(0), None, TileFormat::Jpeg)
        .unwrap();
    service
        .tile(&path, tile(1), None, TileFormat::Jpeg)
        .unwrap();
    assert_eq!(
        service
            .tile(&path, tile(0), None, TileFormat::Jpeg)
            .unwrap(),
        first,
        "a hit"
    );
    service
        .tile(&path, tile(2), None, TileFormat::Jpeg)
        .unwrap();
    let stats = service.stats();
    assert_eq!(
        (stats.hits, stats.misses, stats.evictions, stats.tiles),
        (1, 3, 1, 2)
    );
    assert!(stats.bytes <= size * 5 / 2);
    // Tile 1 was the least recently used: it is rendered again.
    service
        .tile(&path, tile(1), None, TileFormat::Jpeg)
        .unwrap();
    assert_eq!(service.stats().misses, 4);

    // The ETag follows the request, and the image.
    let lut = TileLut {
        min: 0.0,
        max: 100.0,
        gamma: 1.0,
        colormap: Colormap::Grey,
    };
    let other = service
        .tile(&path, tile(0), Some(lut), TileFormat::Jpeg)
        .unwrap();
    assert_ne!(other.etag, first.etag);
    std::thread::sleep(std::time::Duration::from_millis(20));
    ZarrImage::open(&path).unwrap().set_levels(4).unwrap();
    let rebuilt = service
        .tile(&path, tile(0), None, TileFormat::Jpeg)
        .unwrap();
    assert_ne!(rebuilt.etag, first.etag, "the image changed");
}

#[test]
fn tiles_are_served_to_many_at_once() {
    let folder = tempfile::tempdir().unwrap();
    let image = image(folder.path());
    let path = image.path().to_owned();
    let service = Arc::new(TileService::new(64 << 20));
    let tiles: Vec<Vec<Arc<Tile>>> = std::thread::scope(|scope| {
        let workers: Vec<_> = (0..8)
            .map(|_| {
                let (service, path) = (Arc::clone(&service), path.clone());
                scope.spawn(move || {
                    (0..4)
                        .map(|col| {
                            service
                                .tile(
                                    &path,
                                    TileAt {
                                        plane: (0, 0, 0),
                                        level: 10,
                                        col,
                                        row: 1,
                                    },
                                    None,
                                    TileFormat::Jpeg,
                                )
                                .unwrap()
                        })
                        .collect()
                })
            })
            .collect();
        workers.into_iter().map(|w| w.join().unwrap()).collect()
    });
    for each in &tiles[1..] {
        for (a, b) in each.iter().zip(&tiles[0]) {
            assert_eq!(a.bytes, b.bytes);
        }
    }
    let stats = service.stats();
    assert_eq!(stats.hits + stats.misses, 32);
    assert_eq!(stats.tiles, 4);
}

#[test]
fn the_thumbnail_adds_every_channel_in_its_colour_at_the_top_level() {
    let folder = tempfile::tempdir().unwrap();
    let image = image(folder.path());
    // The top level is 250 × 150: channel 0 is 0 everywhere, so the thumbnail is channel 1's
    // green, x + y through 0 to 2000.
    let (rgb, width, height) = composite(&image, (0, 0), 256).unwrap();
    assert_eq!((width, height), (250, 150));
    let top = image
        .read_region::<u16>(2, (0, 1, 0), RectPx::new(0, 0, 250, 150))
        .unwrap();
    for (pixel, &value) in rgb.chunks(3).zip(&top.pixels) {
        let green = (f64::from(value) / 2000.0 * 255.0).round().min(255.0) as u8;
        assert_eq!(pixel, [0, green, 0], "{value}");
    }
    // Fitting 100 pixels halves the top level twice more: 63 × 38.
    let (_, width, height) = composite(&image, (0, 0), 100).unwrap();
    assert_eq!((width, height), (63, 38));

    let jpeg = thumbnail(&image, (0, 0)).unwrap();
    assert_eq!(&jpeg[..2], [0xFF, 0xD8]);
    let sof = jpeg.windows(2).position(|w| w == [0xFF, 0xC0]).unwrap();
    let size = |at: usize| u16::from_be_bytes([jpeg[at], jpeg[at + 1]]);
    assert_eq!(
        (size(sof + 7), size(sof + 5)),
        (250, 150),
        "the JPEG's size"
    );
    assert!(matches!(
        thumbnail(&image, (0, 1)),
        Err(TileError::NotFound(_))
    ));
}
