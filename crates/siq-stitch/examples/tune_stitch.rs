//! The stitching defaults tuned (P8.5): the search's reach and size, Huber's k and the feather,
//! each varied about its default with the others at theirs, over two `siq-datagen` scans of 5 × 4
//! tiles of 2048 × 1536, 15 % overlapping, the stage off by 5 µm: grains in the camera's usual
//! light, and cells in a tenth of it. For the search and Huber's k, the tiles' placement against
//! the truth, the pairs trusted, and the time taken; for the feather, the composite against the
//! ideal mosaic: its PSNR, and the steps of what is left between neighbouring blocks of 16 × 16
//! pixels, where a seam shows, through the simulator's flat and through none. The pairs are
//! registered through the simulator's flat too, as the service registers them through the flat in
//! force, a calibration's, not one estimated from the scan itself.
//!
//! ```text
//! cargo run --release -p siq-stitch --example tune_stitch
//! ```
//!
//! Smoke mode (`SIQ_SMOKE=1`) uses 3 × 3 tiles of 512 × 384.

use std::error::Error;
use std::time::Instant;

use serde_json::Value;
use siq_calib::shading::FlatField;
use siq_datagen::Output;
use siq_datagen::tilescan::{TileScanArgs, tilescan};
use siq_stitch::composite::{self, Bounds, CompositeOptions, Placed, Plane};
use siq_stitch::pairs::{self, PairOptions};
use siq_stitch::solve::{self, SolveOptions};
use siq_stitch::{Edge, MappedTiles, Mosaic};
use siq_storage::zarr::{CHUNK, ChannelInfo, ImageInfo, SampleType, Size5, ZarrImage};
use siq_storage::{Dataset, DatasetReader};
use siq_types::{Affine2, OwnedImage, RectPx, Roi};

/// The simulator's 10× pixel, in µm, and its sensor's dark level.
const PIXEL_UM: f64 = 0.376;
const DARK: f32 = 100.0;

/// The side of the blocks a seam is looked for between.
const BLOCK: usize = 16;

type Result<T> = std::result::Result<T, Box<dyn Error>>;

/// The simulator's vignetting on a sensor of `width` × `height`, fitted as a flat.
fn true_flat(width: u32, height: u32) -> Result<FlatField> {
    let profile = siq_sim::optics::Vignetting::Cos4 { corner: 0.8 };
    let (cx, cy) = (f64::from(width) / 2.0, f64::from(height) / 2.0);
    let light = OwnedImage::from_fn(width as usize, height as usize, |x, y| {
        let (dx, dy) = (x as f64 + 0.5 - cx, y as f64 + 0.5 - cy);
        (1000.0 * profile.at((dx * dx + dy * dy) / (cx * cx + cy * cy))) as f32
    });
    let roi = Roi::full(width, height);
    Ok(FlatField::fit(&light.view(), roi, [width, height], 0.0)?)
}

/// A scan with its ideal mosaic, each tile's true corner, and the simulator's flat.
struct Scan {
    folder: tempfile::TempDir,
    dataset: Dataset,
    source: MappedTiles,
    mosaic: Mosaic,
    truth: Vec<[f64; 2]>,
    flat: FlatField,
}

impl Scan {
    fn new(world: &str, light: f64, grid: (u32, u32), tile: (u32, u32), seed: u64) -> Result<Self> {
        let folder = tempfile::tempdir()?;
        let args = TileScanArgs {
            world: world.to_owned(),
            columns: grid.0,
            rows: grid.1,
            width: tile.0,
            height: tile.1,
            overlap: 0.15,
            stage_error: 5.0,
            light,
            no_mosaic: false,
        };
        let out = Output {
            data_root: folder.path().to_owned(),
            smoke: false,
            seed,
        };
        let dataset = Dataset::open(tilescan(&args, &out)?)?;
        let text = std::fs::read_to_string(dataset.path().join("truth/tilescan.json"))?;
        let truth: Value = serde_json::from_str(&text)?;
        let reader = DatasetReader::open(&dataset)?;
        let (source, frames) = MappedTiles::open(&reader, |_| true)?;
        let source = source.with_capacity(4 * rayon::current_num_threads());
        let mosaic = Mosaic::place(&frames, &Affine2::scale(1.0 / PIXEL_UM, 1.0 / PIXEL_UM))?;
        let truth = mosaic
            .tiles
            .iter()
            .map(|tile| {
                let at = &truth["tiles"][tile.key.p as usize];
                [0, 1].map(|axis| {
                    let of = |key: &str| at[key][axis].as_f64().unwrap_or(f64::NAN);
                    tile.nominal[axis] + (of("true") - of("nominal")) / PIXEL_UM
                })
            })
            .collect();
        Ok(Self {
            folder,
            dataset,
            source,
            mosaic,
            truth,
            flat: true_flat(tile.0, tile.1)?,
        })
    }

    /// The pairs registered with `options`, and the seconds that took.
    fn edges(&self, options: &PairOptions) -> Result<(Vec<Edge>, f64)> {
        let began = Instant::now();
        let found = self
            .mosaic
            .overlaps(options.min_overlap_px, options.min_overlap_share);
        let threads = rayon::current_num_threads();
        let shading = Some((DARK, &self.flat));
        let edges = pairs::register(
            &self.source,
            &self.mosaic,
            &found,
            shading,
            options,
            threads,
        )?;
        Ok((edges, began.elapsed().as_secs_f64()))
    }

    /// How far `positions` are from the truth's, by axis, on average.
    fn offset(&self, positions: &[[f64; 2]]) -> [f64; 2] {
        let n = positions.len() as f64;
        [0, 1].map(|axis| {
            let sum: f64 = (positions.iter().zip(&self.truth))
                .map(|(at, truth)| at[axis] - truth[axis])
                .sum();
            sum / n
        })
    }

    /// `positions` against the truth once their means agree: the RMS and the largest distance.
    fn misplaced(&self, positions: &[[f64; 2]]) -> (f64, f64) {
        let mean = self.offset(positions);
        let errors: Vec<f64> = (positions.iter().zip(&self.truth))
            .map(|(at, truth)| (at[0] - truth[0] - mean[0]).hypot(at[1] - truth[1] - mean[1]))
            .collect();
        let rms = (errors.iter().map(|e| e * e).sum::<f64>() / errors.len() as f64).sqrt();
        (rms, errors.iter().copied().fold(0.0, f64::max))
    }

    /// The scan composited at `positions`, moved to the truth's on average, with `options`, through
    /// the flat if `flat`; then against the ideal mosaic, as `got ≈ a·ideal + b`: its PSNR, the RMS
    /// of the steps of what is left between neighbouring blocks, and the seconds the composite
    /// took.
    fn composite(
        &self,
        positions: &[[f64; 2]],
        options: &CompositeOptions,
        flat: bool,
    ) -> Result<[f64; 3]> {
        let offset = self.offset(positions);
        let corners: Vec<[f64; 2]> = positions
            .iter()
            .map(|at| [at[0] - offset[0], at[1] - offset[1]])
            .collect();
        let sizes: Vec<[u32; 2]> = self.mosaic.tiles.iter().map(|tile| tile.size).collect();
        let bounds = Bounds::of(&corners, &sizes);
        let info = ImageInfo {
            name: "tuned".to_owned(),
            sample: SampleType::U16,
            size: Size5 {
                t: 1,
                c: 1,
                z: 1,
                y: bounds.size[1],
                x: bounds.size[0],
            },
            levels: 1,
            chunk: CHUNK,
            pixel_size: [1.0; 3],
            origin: [0.0; 3],
            channels: vec![ChannelInfo {
                label: "tuned".to_owned(),
                colour: [255; 3],
                window: (0.0, 16_383.0),
            }],
            cfa: None,
        };
        let path = self.folder.path().join("tuned.ome.zarr");
        let _ = std::fs::remove_dir_all(&path);
        let image = ZarrImage::create(&path, info)?;
        let tiles = (0..self.mosaic.tiles.len())
            .map(|k| Placed {
                source: k,
                tile: self.mosaic.tiles[k],
                corner: corners[k],
            })
            .collect();
        let plane = Plane {
            at: (0, 0, 0),
            tiles,
        };
        let began = Instant::now();
        let shading = flat.then_some((DARK, &self.flat));
        composite::composite(
            &self.source,
            &[plane],
            &bounds,
            &image,
            shading,
            options,
            &(),
        )?;
        let took = began.elapsed().as_secs_f64();

        // The ideal's pixel is the image's plus `k`: it starts at the first tile's stage corner.
        let ideal = ZarrImage::open(&self.dataset.zarr_dir().join("truth.ome.zarr"))?;
        let [width, height] = sizes[0];
        let k = [
            bounds.origin[0] + i64::from(width / 2),
            bounds.origin[1] + i64::from(height / 2),
        ];
        let ideal_size = [ideal.info().size.x as i64, ideal.info().size.y as i64];
        let margin = 64;
        let first = [0, 1].map(|axis| margin.max(margin - k[axis]));
        let last = [0, 1].map(|axis| {
            (bounds.size[axis] as i64 - margin).min(ideal_size[axis] - margin - k[axis])
        });
        let blocks = [0, 1].map(|axis| ((last[axis] - first[axis]).max(0) as usize) / BLOCK);
        let size = blocks.map(|n| (n * BLOCK) as u32);
        let rect = |at: [i64; 2]| RectPx::new(at[0] as u32, at[1] as u32, size[0], size[1]);
        let got = image.read_region::<u16>(0, (0, 0, 0), rect(first))?.pixels;
        let there = [first[0] + k[0], first[1] + k[1]];
        let want = ideal.read_region::<u16>(0, (0, 0, 0), rect(there))?.pixels;

        // The fit, then what is left: over all, and by block.
        let n = got.len() as f64;
        let mean = |values: &[u16]| values.iter().map(|&v| f64::from(v)).sum::<f64>() / n;
        let (mg, mw) = (mean(&got), mean(&want));
        let (mut gw, mut ww) = (0.0, 0.0);
        for (&g, &w) in got.iter().zip(&want) {
            let (g, w) = (f64::from(g) - mg, f64::from(w) - mw);
            (gw, ww) = (gw + g * w, ww + w * w);
        }
        let a = gw / ww;
        let mut left = 0.0;
        let mut by_block = vec![0.0; blocks[0] * blocks[1]];
        for (at, (&g, &w)) in got.iter().zip(&want).enumerate() {
            let r = f64::from(g) - mg - a * (f64::from(w) - mw);
            left += r * r;
            let (x, y) = (at % size[0] as usize, at / size[0] as usize);
            by_block[(y / BLOCK) * blocks[0] + x / BLOCK] += r / (BLOCK * BLOCK) as f64;
        }
        let range = got.iter().max().copied().unwrap_or(0) - got.iter().min().copied().unwrap_or(0);
        let psnr = 20.0 * (f64::from(range) / (left / n).sqrt()).log10();
        let mut steps = Vec::new();
        for y in 0..blocks[1] {
            for x in 0..blocks[0] {
                let here = by_block[y * blocks[0] + x];
                if x + 1 < blocks[0] {
                    steps.push(by_block[y * blocks[0] + x + 1] - here);
                }
                if y + 1 < blocks[1] {
                    steps.push(by_block[(y + 1) * blocks[0] + x] - here);
                }
            }
        }
        let steps = (steps.iter().map(|s| s * s).sum::<f64>() / steps.len().max(1) as f64).sqrt();
        Ok([psnr, steps, took])
    }
}

fn main() -> Result<()> {
    let smoke = std::env::var_os("SIQ_SMOKE").is_some();
    let (grid, tile) = match smoke {
        true => ((3, 3), (512, 384)),
        false => ((5, 4), (2048, 1536)),
    };
    for (world, light, seed) in [("grains", 1.0, 31), ("cells", 0.1, 32)] {
        let began = Instant::now();
        let scan = Scan::new(world, light, grid, tile, seed)?;
        println!(
            "{world} in {light} of the light: {} × {} tiles of {} × {}, made in {:.1?}",
            grid.0,
            grid.1,
            tile.0,
            tile.1,
            began.elapsed()
        );
        let placed = |edges: &[Edge], options: &SolveOptions| {
            let solution = solve::solve(&scan.mosaic, edges, options);
            let (rms, worst) = scan.misplaced(&solution.positions);
            let trusted = edges.iter().filter(|edge| edge.confident).count();
            format!(
                "{rms:.3} px RMS, {worst:.3} at worst; {trusted} of {} pairs trusted",
                edges.len()
            )
        };
        let solving = SolveOptions::default();

        // The search: how far it reaches, and its size.
        for reach_sigmas in [2.0, 3.0, 4.0, 6.0] {
            let options = PairOptions {
                reach_sigmas,
                ..PairOptions::default()
            };
            let (edges, took) = scan.edges(&options)?;
            println!(
                "  reach {reach_sigmas} σ:        {}; {took:.2} s",
                placed(&edges, &solving)
            );
        }
        for search_px in [256, 512, 1024, 2048] {
            let options = PairOptions {
                search_px,
                ..PairOptions::default()
            };
            let (edges, took) = scan.edges(&options)?;
            println!(
                "  search {search_px:4} px:    {}; {took:.2} s",
                placed(&edges, &solving)
            );
        }

        // Huber's k, with every fifth edge 0.8 px wrong: under the rejection's pixel.
        let (mut edges, _) = scan.edges(&PairOptions::default())?;
        for edge in edges.iter_mut().step_by(5) {
            edge.shift = [edge.shift[0] + 0.6, edge.shift[1] - 0.53];
        }
        for huber_k in [0.5, 1.0, 1.345, 2.0, 1e6] {
            let options = SolveOptions {
                huber_k,
                ..SolveOptions::default()
            };
            let k = match huber_k > 1e5 {
                true => "∞".to_owned(),
                false => huber_k.to_string(),
            };
            println!("  Huber's k {k:<5}:  {}", placed(&edges, &options));
        }

        // The feather, the tiles where the solve put them.
        let (edges, _) = scan.edges(&PairOptions::default())?;
        let solution = solve::solve(&scan.mosaic, &edges, &solving);
        for feather_px in [32.0, 64.0, 128.0, 256.0] {
            let options = CompositeOptions {
                feather_px,
                ..CompositeOptions::default()
            };
            let [psnr, steps, took] = scan.composite(&solution.positions, &options, true)?;
            let [bare, bare_steps, _] = scan.composite(&solution.positions, &options, false)?;
            println!(
                "  feather {feather_px:3} px:   PSNR {psnr:.2} dB, steps {steps:.3} ADU RMS; {took:.2} s; without the flat, {bare:.2} dB and {bare_steps:.3} ADU"
            );
        }
    }
    Ok(())
}
