//! Out-of-core compositing (ADR-0063): a `siq-datagen` scan's tiles, through the simulator's flat,
//! composited at their true corners match the ideal mosaic (PSNR), and at the stage's corners they
//! don't; two neighbours composited apart agree where they overlap, so leave no seam, where
//! without the flat they don't; whole pixels are copied as they are; and a pixel no tile covers is
//! 0. REQ-STI-002, REQ-STI-003.

use serde_json::Value;
use siq_calib::shading::FlatField;
use siq_datagen::Output;
use siq_datagen::tilescan::{TileScanArgs, tilescan};
use siq_stitch::composite::{self, Bounds, CompositeOptions, Placed, Plane};
use siq_stitch::{MappedTiles, Mosaic, Tile};
use siq_storage::zarr::{ChannelInfo, ImageInfo, SampleType, Size5, ZarrImage};
use siq_storage::{Dataset, DatasetReader, FrameKey};
use siq_types::{Affine2, OwnedImage, RectPx, Roi};

/// The generator's camera: 3.76 µm pixels behind a 10× objective, a dark offset of 100.
const PIXEL_UM: f64 = 0.376;
const DARK: f32 = 100.0;

/// An image of `bounds` with `planes` channels in `folder`, in chunks of 256.
fn image(folder: &std::path::Path, bounds: &Bounds, planes: u64) -> ZarrImage {
    let channel = |c: u64| ChannelInfo {
        label: format!("plane {c}"),
        colour: [255; 3],
        window: (0.0, 16_383.0),
    };
    let info = ImageInfo {
        name: "mosaic".to_owned(),
        sample: SampleType::U16,
        size: Size5 {
            t: 1,
            c: planes,
            z: 1,
            y: bounds.size[1],
            x: bounds.size[0],
        },
        levels: 1,
        chunk: 256,
        pixel_size: [1.0, PIXEL_UM, PIXEL_UM],
        origin: [0.0, 0.0, 0.0],
        channels: (0..planes).map(channel).collect(),
        cfa: None,
    };
    ZarrImage::create(&folder.join("mosaic.ome.zarr"), info).unwrap()
}

fn read(image: &ZarrImage, c: u64, rect: RectPx) -> Vec<f64> {
    let region = image.read_region::<u16>(0, (0, c, 0), rect).unwrap();
    region.pixels.iter().map(|&v| f64::from(v)).collect()
}

/// The simulator's vignetting on a sensor of `width` × `height`, fitted as a flat.
fn true_flat(width: u32, height: u32) -> FlatField {
    let profile = siq_sim::optics::Vignetting::Cos4 { corner: 0.8 };
    let (cx, cy) = (f64::from(width) / 2.0, f64::from(height) / 2.0);
    let light = OwnedImage::from_fn(width as usize, height as usize, |x, y| {
        let (dx, dy) = (x as f64 + 0.5 - cx, y as f64 + 0.5 - cy);
        (1000.0 * profile.at((dx * dx + dy * dy) / (cx * cx + cy * cy))) as f32
    });
    FlatField::fit(
        &light.view(),
        Roi::full(width, height),
        [width, height],
        0.0,
    )
    .unwrap()
}

fn rms(values: &[f64]) -> f64 {
    (values.iter().map(|v| v * v).sum::<f64>() / values.len().max(1) as f64).sqrt()
}

/// A `siq-datagen` scan of 3 × 3 tiles of 512 × 384 over grains, with its ideal mosaic.
struct Scan {
    _folder: tempfile::TempDir,
    dataset: Dataset,
    source: MappedTiles,
    mosaic: Mosaic,
    truth: Value,
}

impl Scan {
    fn new() -> Self {
        let folder = tempfile::tempdir().unwrap();
        let args = TileScanArgs {
            world: "grains".to_owned(),
            columns: 3,
            rows: 3,
            width: 512,
            height: 384,
            overlap: 0.2,
            stage_error: 5.0,
            light: 1.0,
            no_mosaic: false,
        };
        let out = Output {
            data_root: folder.path().to_owned(),
            smoke: false,
            seed: 6,
        };
        let dataset = Dataset::open(tilescan(&args, &out).unwrap()).unwrap();
        let text = std::fs::read_to_string(dataset.path().join("truth/tilescan.json")).unwrap();
        let reader = DatasetReader::open(&dataset).unwrap();
        let (source, frames) = MappedTiles::open(&reader, |_| true).unwrap();
        let l = Affine2::scale(1.0 / PIXEL_UM, 1.0 / PIXEL_UM);
        Self {
            _folder: folder,
            mosaic: Mosaic::place(&frames, &l).unwrap(),
            source: source.with_capacity(3),
            dataset,
            truth: serde_json::from_str(&text).unwrap(),
        }
    }

    /// Tile `p`'s corner where the truth puts it (`"true"`), or the stage (`"nominal"`).
    fn corner(&self, p: usize, which: &str) -> [f64; 2] {
        let at =
            |key: &str, axis: usize| self.truth["tiles"][p][key][axis].as_f64().unwrap() / PIXEL_UM;
        [0, 1]
            .map(|axis| self.mosaic.tiles[p].nominal[axis] + at(which, axis) - at("nominal", axis))
    }

    fn placed(&self, p: usize, which: &str) -> Placed {
        Placed {
            source: p,
            tile: self.mosaic.tiles[p],
            corner: self.corner(p, which),
        }
    }
}

#[test]
fn a_scan_composited_at_its_true_places_matches_the_ideal_mosaic() {
    let scan = Scan::new();
    let flat = true_flat(512, 384);
    let ideal = ZarrImage::open(&scan.dataset.zarr_dir().join("truth.ome.zarr")).unwrap();
    let mut scores = Vec::new();
    for which in ["true", "nominal"] {
        let tiles: Vec<Placed> = (0..9).map(|p| scan.placed(p, which)).collect();
        let corners: Vec<[f64; 2]> = tiles.iter().map(|placed| placed.corner).collect();
        let bounds = Bounds::of(&corners, &[[512, 384]; 9]);
        let place = tempfile::tempdir().unwrap();
        let made = image(place.path(), &bounds, 1);
        let plane = Plane {
            at: (0, 0, 0),
            tiles,
        };
        let options = CompositeOptions::default();
        let shading = Some((DARK, &flat));
        composite::composite(
            &scan.source,
            &[plane],
            &bounds,
            &made,
            shading,
            &options,
            &(),
        )
        .unwrap();
        // The inside of the ideal mosaic, whose pixel `i` is the mosaic's `i − size / 2`.
        let (w, h) = (
            ideal.info().size.x as i64 - 32,
            ideal.info().size.y as i64 - 32,
        );
        let from = [16 - 256 - bounds.origin[0], 16 - 192 - bounds.origin[1]];
        let rect = |at: [i64; 2]| RectPx::new(at[0] as u32, at[1] as u32, w as u32, h as u32);
        let (got, want) = (read(&made, 0, rect(from)), read(&ideal, 0, rect([16, 16])));
        // The residual against the best fit `a · ideal + b`: the flat leaves the level free.
        let n = got.len() as f64;
        let (mg, mw) = (got.iter().sum::<f64>() / n, want.iter().sum::<f64>() / n);
        let (mut gw, mut ww) = (0.0, 0.0);
        for (g, w) in got.iter().zip(&want) {
            (gw, ww) = (gw + (g - mg) * (w - mw), ww + (w - mw) * (w - mw));
        }
        let a = gw / ww;
        let off: Vec<f64> = got
            .iter()
            .zip(&want)
            .map(|(g, w)| g - (a * (w - mw) + mg))
            .collect();
        let peak = got.iter().copied().fold(f64::MIN, f64::max)
            - got.iter().copied().fold(f64::MAX, f64::min);
        let psnr = 20.0 * (peak / rms(&off)).log10();
        eprintln!("at the {which} corners: PSNR {psnr:.1} dB against the ideal mosaic");
        scores.push(psnr);
    }
    // The ideal has neither the optics' blur nor the noise of the tiles: they set the ceiling.
    assert!(
        scores[0] >= 25.0 && scores[0] > scores[1] + 6.0,
        "{scores:?}"
    );
}

#[test]
fn neighbours_composited_apart_agree_where_they_overlap() {
    let scan = Scan::new();
    let flat = true_flat(512, 384);
    // The middle tile and the ones left of it and above it, each into a plane of its own.
    let mut steps = Vec::new();
    for shading in [Some((DARK, &flat)), None] {
        for (a, b) in [(3, 4), (1, 4)] {
            let (pa, pb) = (scan.placed(a, "true"), scan.placed(b, "true"));
            let bounds = Bounds::of(&[pa.corner, pb.corner], &[[512, 384]; 2]);
            let place = tempfile::tempdir().unwrap();
            let made = image(place.path(), &bounds, 2);
            let planes = [
                Plane {
                    at: (0, 0, 0),
                    tiles: vec![pa],
                },
                Plane {
                    at: (0, 1, 0),
                    tiles: vec![pb],
                },
            ];
            let options = CompositeOptions::default();
            composite::composite(
                &scan.source,
                &planes,
                &bounds,
                &made,
                shading,
                &options,
                &(),
            )
            .unwrap();
            // Their overlap, 4 pixels in: the difference of the two, by blocks of 16 × 16, so
            // that noise averages away and a step in brightness stays.
            let origin = bounds.origin.map(|at| at as f64);
            let size = [512.0, 384.0];
            let low = [0, 1].map(|axis| pa.corner[axis].max(pb.corner[axis]) - origin[axis] + 4.0);
            let high = [0, 1].map(|axis| {
                let end = (pa.corner[axis] + size[axis]).min(pb.corner[axis] + size[axis]);
                end - origin[axis] - 4.0
            });
            let [x, y] = low.map(|v| v.ceil() as u32);
            let [w, h] = [0, 1].map(|axis| ((high[axis] - low[axis]) as u32 / 16) * 16);
            let rect = RectPx::new(x, y, w, h);
            let (one, other) = (read(&made, 0, rect), read(&made, 1, rect));
            let level = one.iter().sum::<f64>() / one.len() as f64;
            let mut blocks = Vec::new();
            for by in 0..h / 16 {
                for bx in 0..w / 16 {
                    let mut sum = 0.0;
                    for k in 0..256 {
                        let at = ((by * 16 + k / 16) * w + bx * 16 + k % 16) as usize;
                        sum += one[at] - other[at];
                    }
                    blocks.push(sum / 256.0 / level);
                }
            }
            steps.push(rms(&blocks));
        }
    }
    eprintln!("the steps where the neighbours overlap, flat-corrected then not: {steps:.4?}");
    // Through the flat, under half a percent; without it, the vignetting's several percent.
    assert!(steps[..2].iter().all(|&step| step < 0.005), "{steps:?}");
    assert!(steps[2..].iter().all(|&step| step > 0.02), "{steps:?}");
}

#[test]
fn whole_pixels_are_copied_and_uncovered_ones_are_0() {
    // Two tiles of a ramp, side by side 3 pixels apart, at fractional corners.
    let ramp = |seed: u16| OwnedImage::from_fn(64, 48, move |x, y| seed + (x + 64 * y) as u16);
    let tiles = [ramp(1000), ramp(9000)];
    let tile = |p: u32| Tile {
        key: FrameKey {
            t: 0,
            c: 0,
            z: 0,
            p,
        },
        size: [64, 48],
        nominal: [0.0, 0.0],
        sensor: [0, 0],
        binning: 1,
    };
    let corners = [[0.3, -0.2], [66.6, 10.4]];
    let placed = (0..2)
        .map(|p| Placed {
            source: p,
            tile: tile(p as u32),
            corner: corners[p],
        })
        .collect();
    let bounds = Bounds::of(&corners, &[[64, 48], [64, 48]]);
    assert_eq!(
        bounds,
        Bounds {
            origin: [0, 0],
            size: [131, 58]
        }
    );
    let folder = tempfile::tempdir().unwrap();
    let made = image(folder.path(), &bounds, 1);
    let options = CompositeOptions {
        integer: true,
        ..CompositeOptions::default()
    };
    let plane = Plane {
        at: (0, 0, 0),
        tiles: placed,
    };
    composite::composite(&tiles[..], &[plane], &bounds, &made, None, &options, &()).unwrap();
    let all = read(&made, 0, RectPx::new(0, 0, 131, 58));
    let at = |x: usize, y: usize| all[y * 131 + x] as u16;
    // The first tile at (0, 0), the second at (67, 10): their pixels as they are.
    assert_eq!(at(0, 0), 1000);
    assert_eq!(at(10, 20), 1000 + 10 + 64 * 20);
    assert_eq!(at(67 + 5, 10 + 7), 9000 + 5 + 64 * 7);
    // Between them, and below the first, nothing.
    assert_eq!(at(65, 5), 0);
    assert_eq!(at(10, 50), 0);
}
