//! Out-of-core compositing (ADR-0063): the mosaic written into OME-Zarr a block at a time, each
//! block from the tiles that cover it at their solved corners: sampled with their sub-pixel shift,
//! corrected by the flat, weighted by ramps that fall to the tiles' edges, and normalised. Only the
//! blocks being made, and the tiles' pages they read, are held.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};

use rayon::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use siq_calib::shading::FlatField;
use siq_storage::Control;
use siq_storage::zarr::ZarrImage;
use siq_types::OwnedImage;

use crate::StitchError;
use crate::tiles::{Tile, TileSource};

/// The flat's gains are kept on a grid this many pixels apart, between which they are
/// interpolated: the flat is smooth.
const GAIN_STEP: usize = 16;
/// A tile's least weight, at its very edge, so that a pixel it alone covers is still its.
const LEAST_WEIGHT: f32 = 1e-3;

/// How a mosaic is composited.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct CompositeOptions {
    /// How far in from a tile's edge its weight rises to 1, in pixels: at most half its smaller
    /// side.
    pub feather_px: f64,
    /// Whole pixels only: each tile's corner rounded, and its pixels copied, not resampled.
    pub integer: bool,
    /// A block's side, in the image's chunks.
    pub block_chunks: u64,
}

impl Default for CompositeOptions {
    /// A feather of 128 pixels; bilinear resampling; blocks of 2 × 2 chunks.
    fn default() -> Self {
        Self {
            feather_px: 128.0,
            integer: false,
            block_chunks: 2,
        }
    }
}

/// Where a mosaic lies, in mosaic pixels: its first pixel, and its width and height.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Bounds {
    /// The mosaic pixel at the image's first pixel.
    pub origin: [i64; 2],
    /// The image's width and height.
    pub size: [u64; 2],
}

impl Bounds {
    /// The box of tiles of `sizes` with their corners at `corners`, to whole pixels.
    pub fn of(corners: &[[f64; 2]], sizes: &[[u32; 2]]) -> Self {
        let mut low = [f64::INFINITY; 2];
        let mut high = [f64::NEG_INFINITY; 2];
        for (corner, size) in corners.iter().zip(sizes) {
            for axis in 0..2 {
                low[axis] = low[axis].min(corner[axis]);
                high[axis] = high[axis].max(corner[axis] + f64::from(size[axis]));
            }
        }
        if corners.is_empty() {
            return Self {
                origin: [0, 0],
                size: [0, 0],
            };
        }
        let origin = low.map(|at| at.round() as i64);
        let size = [0, 1].map(|axis| (high[axis].round() as i64 - origin[axis]).max(0) as u64);
        Self { origin, size }
    }
}

/// A tile of a plane, placed: which of the source's, the tile, and its corner on the mosaic.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placed {
    /// Its index in the source.
    pub source: usize,
    /// The tile: its size, and its place on the sensor for the flat.
    pub tile: Tile,
    /// Its corner, in mosaic pixels: the solution's.
    pub corner: [f64; 2],
}

/// A plane of the image, `(t, c, z)`, and its tiles.
#[derive(Debug, Clone, PartialEq)]
pub struct Plane {
    /// Its time point, channel and focus plane in the image.
    pub at: (u64, u64, u64),
    /// Its tiles.
    pub tiles: Vec<Placed>,
}

/// The memory a composite with blocks of `options` holds at most with `threads` working, in
/// bytes, for an image of `chunk` pixels a side: each block's sums and weights, each chunk as
/// filled, copied whole and encoded, and the encoder's context.
pub fn bytes(chunk: u64, options: &CompositeOptions, threads: usize) -> u64 {
    let side = options.block_chunks.max(1) * chunk;
    let encoder = 4 << 20;
    threads.max(1) as u64 * (side * side * 8 + 3 * chunk * chunk * 2 + encoder)
}

/// A flat's gains over a tile, on a grid [`GAIN_STEP`] pixels apart.
struct Gains {
    grid: OwnedImage<f32>,
}

impl Gains {
    fn of(flat: &FlatField, tile: &Tile) -> Self {
        let (b, step) = (f64::from(tile.binning), GAIN_STEP as f64);
        let nodes = |side: u32| (side as usize).saturating_sub(1).div_ceil(GAIN_STEP) + 2;
        // Node k at the centre of the tile's pixel k × GAIN_STEP.
        let origin = [0, 1].map(|axis| f64::from(tile.sensor[axis]) + 0.5 * b - 0.5 * step * b);
        let grid = flat.gains_on(
            origin,
            [step * b; 2],
            (nodes(tile.size[0]), nodes(tile.size[1])),
        );
        Self { grid }
    }

    /// The gain at the tile's pixel `(u, v)`, between the nodes.
    fn at(&self, u: f32, v: f32) -> f32 {
        let grid = self.grid.view();
        let (fu, fv) = (u.max(0.0) / GAIN_STEP as f32, v.max(0.0) / GAIN_STEP as f32);
        let (k, l) = (fu as usize, fv as usize);
        let (k, l) = (k.min(grid.width() - 2), l.min(grid.height() - 2));
        let (s, t) = (fu - k as f32, fv - l as f32);
        let (top, bottom) = (grid.row(l), grid.row(l + 1));
        let upper = top[k] * (1.0 - s) + top[k + 1] * s;
        let lower = bottom[k] * (1.0 - s) + bottom[k + 1] * s;
        upper * (1.0 - t) + lower * t
    }
}

/// The tiles of a plane, by the cells of a grid as large as its largest tile, for a block to find
/// those that cover it.
struct Index {
    cell: f64,
    cells: HashMap<[i64; 2], Vec<usize>>,
}

impl Index {
    fn of(tiles: &[Placed]) -> Self {
        let cell = tiles
            .iter()
            .map(|placed| f64::from(placed.tile.size[0].max(placed.tile.size[1])))
            .fold(1.0, f64::max);
        let mut cells: HashMap<[i64; 2], Vec<usize>> = HashMap::new();
        for (k, placed) in tiles.iter().enumerate() {
            let [x0, y0] = placed.corner;
            let [w, h] = placed.tile.size.map(f64::from);
            for cy in (y0 / cell).floor() as i64..=((y0 + h) / cell).floor() as i64 {
                for cx in (x0 / cell).floor() as i64..=((x0 + w) / cell).floor() as i64 {
                    cells.entry([cx, cy]).or_default().push(k);
                }
            }
        }
        Self { cell, cells }
    }

    /// The tiles whose cells meet the box from `low` to `high`, each once.
    fn meeting(&self, low: [f64; 2], high: [f64; 2]) -> Vec<usize> {
        let mut found = Vec::new();
        for cy in (low[1] / self.cell).floor() as i64..=(high[1] / self.cell).floor() as i64 {
            for cx in (low[0] / self.cell).floor() as i64..=(high[0] / self.cell).floor() as i64 {
                found.extend(self.cells.get(&[cx, cy]).into_iter().flatten());
            }
        }
        found.sort_unstable();
        found.dedup();
        found
    }
}

/// A block of the image: its plane, and its first pixel and size in the image.
#[derive(Debug, Clone, Copy)]
struct Block {
    plane: usize,
    at: [u64; 2],
    size: [u64; 2],
}

/// Writes `planes` into level 0 of `image`, whose size is `bounds`', a block at a time on the
/// calling rayon pool, each pixel from the tiles that cover it, corrected by `shading` (the dark
/// level and the flat) if given. A pixel no tile covers is 0. `control` hears the share of blocks
/// done, and may stop it.
pub fn composite(
    source: &(impl TileSource + ?Sized),
    planes: &[Plane],
    bounds: &Bounds,
    image: &ZarrImage,
    shading: Option<(f32, &FlatField)>,
    options: &CompositeOptions,
    control: &dyn Control,
) -> Result<(), StitchError> {
    let chunk = image.info().chunk;
    let side = options.block_chunks.max(1) * chunk;
    let indexes: Vec<Index> = planes.iter().map(|plane| Index::of(&plane.tiles)).collect();
    // The flat's gains, once for each place on the sensor and size of tile.
    let mut gains: HashMap<([u32; 2], [u32; 2], u32), Gains> = HashMap::new();
    if let Some((_, flat)) = shading {
        for placed in planes.iter().flat_map(|plane| &plane.tiles) {
            let tile = &placed.tile;
            let key = (tile.sensor, tile.size, tile.binning);
            gains.entry(key).or_insert_with(|| Gains::of(flat, tile));
        }
    }
    let [width, height] = bounds.size;
    let blocks: Vec<Block> = (0..planes.len())
        .flat_map(|plane| {
            (0..height.div_ceil(side)).flat_map(move |row| {
                (0..width.div_ceil(side)).map(move |column| {
                    let at = [column * side, row * side];
                    let size = [side.min(width - at[0]), side.min(height - at[1])];
                    Block { plane, at, size }
                })
            })
        })
        .collect();
    let done = AtomicUsize::new(0);
    blocks.par_iter().try_for_each(|block| {
        if control.cancelled() {
            return Err(StitchError::Cancelled);
        }
        let plane = &planes[block.plane];
        let (sums, weights) = fill(
            source,
            plane,
            &indexes[block.plane],
            bounds,
            block,
            shading.map(|(dark, _)| (dark, &gains)),
            options,
        )?;
        write(image, plane.at, block, chunk, &sums, &weights)?;
        let finished = done.fetch_add(1, Ordering::Relaxed) + 1;
        control.progress(finished as f64 / blocks.len() as f64);
        Ok(())
    })
}

type GainMaps = HashMap<([u32; 2], [u32; 2], u32), Gains>;

/// The weighted sums of `block`'s pixels over the tiles that cover it, and their weights.
fn fill(
    source: &(impl TileSource + ?Sized),
    plane: &Plane,
    index: &Index,
    bounds: &Bounds,
    block: &Block,
    shading: Option<(f32, &GainMaps)>,
    options: &CompositeOptions,
) -> Result<(Vec<f32>, Vec<f32>), StitchError> {
    let [bw, bh] = block.size.map(|side| side as usize);
    let (mut sums, mut weights) = (vec![0.0f32; bw * bh], vec![0.0f32; bw * bh]);
    // The block's first pixel on the mosaic.
    let x0 = bounds.origin[0] + block.at[0] as i64;
    let y0 = bounds.origin[1] + block.at[1] as i64;
    let low = [x0 as f64, y0 as f64];
    let high = [low[0] + bw as f64, low[1] + bh as f64];
    for k in index.meeting(low, high) {
        let placed = &plane.tiles[k];
        let [tw, th] = placed.tile.size;
        let corner = match options.integer {
            true => placed.corner.map(f64::round),
            false => placed.corner,
        };
        // The mosaic pixels whose centres the tile covers, `X + 0.5` from `corner` to `corner +
        // size`, within the block.
        let span = |from: f64, side: u32, start: i64, length: usize| {
            let first = (from - 0.5).ceil() as i64;
            let last = (from + f64::from(side) - 0.5).floor() as i64;
            (first.max(start), last.min(start + length as i64 - 1))
        };
        let (xs, ys) = (span(corner[0], tw, x0, bw), span(corner[1], th, y0, bh));
        if xs.0 > xs.1 || ys.0 > ys.1 {
            continue;
        }
        let pixels = source.pixels(placed.source)?;
        let view = pixels.view()?;
        let gains = shading.map(|(dark, maps)| {
            let tile = &placed.tile;
            (dark, &maps[&(tile.sensor, tile.size, tile.binning)])
        });
        let feather = options.feather_px.min(f64::from(tw.min(th)) / 2.0).max(1.0) as f32;
        let ramp = |u: f32, side: u32| {
            ((u + 0.5).min(side as f32 - 0.5 - u) / feather).clamp(LEAST_WEIGHT, 1.0)
        };
        let (wmax, hmax) = (tw as usize - 1, th as usize - 1);
        for y in ys.0..=ys.1 {
            let v = (y as f64 - corner[1]) as f32;
            let row = (y - y0) as usize * bw;
            let wy = ramp(v, th);
            // Within half a pixel of an edge, the edge's pixel.
            let vc = v.clamp(0.0, hmax as f32);
            let (v0, t) = (vc as usize, vc - vc.floor());
            let (r0, r1) = (view.row(v0.min(hmax)), view.row((v0 + 1).min(hmax)));
            for x in xs.0..=xs.1 {
                let u = (x as f64 - corner[0]) as f32;
                let value = if options.integer {
                    f32::from(r0[(u as usize).min(wmax)])
                } else {
                    let uc = u.clamp(0.0, wmax as f32);
                    let (u0, s) = (uc as usize, uc - uc.floor());
                    let (a, b) = (u0.min(wmax), (u0 + 1).min(wmax));
                    let top = f32::from(r0[a]) * (1.0 - s) + f32::from(r0[b]) * s;
                    let bottom = f32::from(r1[a]) * (1.0 - s) + f32::from(r1[b]) * s;
                    top * (1.0 - t) + bottom * t
                };
                let value = match gains {
                    Some((dark, map)) => (value - dark).max(0.0) * map.at(u, v),
                    None => value,
                };
                let weight = ramp(u, tw) * wy;
                let at = row + (x - x0) as usize;
                sums[at] += weight * value;
                weights[at] += weight;
            }
        }
    }
    Ok((sums, weights))
}

/// `block`'s chunks of plane `at` of `image`, normalised from `sums` and `weights`.
fn write(
    image: &ZarrImage,
    at: (u64, u64, u64),
    block: &Block,
    chunk: u64,
    sums: &[f32],
    weights: &[f32],
) -> Result<(), StitchError> {
    let bw = block.size[0] as usize;
    for cy in (0..block.size[1]).step_by(chunk as usize) {
        for cx in (0..block.size[0]).step_by(chunk as usize) {
            let (w, h) = (
                chunk.min(block.size[0] - cx) as usize,
                chunk.min(block.size[1] - cy) as usize,
            );
            let pixels = OwnedImage::from_fn(w, h, |x, y| {
                let k = (cy as usize + y) * bw + cx as usize + x;
                match weights[k] > 0.0 {
                    true => (sums[k] / weights[k]).round().clamp(0.0, 65_535.0) as u16,
                    false => 0,
                }
            });
            let cell = ((block.at[1] + cy) / chunk, (block.at[0] + cx) / chunk);
            image.write_chunk(0, at, cell, pixels.view())?;
        }
    }
    Ok(())
}
