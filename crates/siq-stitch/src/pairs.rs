//! Pairwise registration (ADR-0061): for each pair of overlapping tiles, how far one truly is from
//! the other, by phase correlation of their nominal overlaps, flat-corrected, searched only as far
//! as the stage's uncertainty reaches, and how sure that is.

use rayon::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use siq_calib::shading::FlatField;
use siq_register::{Options, Registrar, Registration};
use siq_types::{ImageView, OwnedImage, RectPx};

use crate::StitchError;
use crate::tiles::{Mosaic, Tile, TileSource, overlap};

/// The smallest overlap registered each way, in pixels: the registrar's least.
const MIN_SIDE: u32 = 16;

/// How pairs are found and registered.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct PairOptions {
    /// The least overlap two tiles must have each way to be registered, in pixels.
    pub min_overlap_px: f64,
    /// The least overlap's area, as a share of a tile's: a scan's corners, a few percent, fall
    /// under it.
    pub min_overlap_share: f64,
    /// The stage's uncertainty: its error's standard deviation per axis, in µm.
    pub stage_sigma_um: f64,
    /// How far the search reaches each way, in standard deviations of a pair's offset.
    pub reach_sigmas: f64,
    /// The least share of the overlap that a registration's shift may leave in common: a match in
    /// a sliver is chance.
    pub min_shared: f64,
    /// The least peak-to-correlation energy of a trusted registration: a floor, as an overlap the
    /// stage shifted far registers rightly with little energy.
    pub min_pce: f64,
    /// The least correlation of a trusted registration's overlap.
    pub min_ncc: f64,
    /// The least margin of a trusted registration's peak over the next.
    pub min_margin: f64,
}

impl Default for PairOptions {
    /// 32 pixels and 5 % of a tile; a stage of 5 µm, searched 4 standard deviations of a pair's
    /// offset out, leaving a quarter of the overlap; and a correlation of 0.8, a margin of 0.3 and
    /// an energy of 50 to trust.
    fn default() -> Self {
        Self {
            min_overlap_px: 32.0,
            min_overlap_share: 0.05,
            stage_sigma_um: 5.0,
            reach_sigmas: 4.0,
            min_shared: 0.25,
            min_pce: 50.0,
            min_ncc: 0.8,
            min_margin: 0.3,
        }
    }
}

impl PairOptions {
    /// How far the search reaches each way, in mosaic pixels: a pair's offset is the difference of
    /// two tiles' errors, so √2 of the stage's, and a pixel more for the overlaps' rounding.
    pub fn reach_px(&self, px_per_um: f64) -> f64 {
        self.reach_sigmas * std::f64::consts::SQRT_2 * self.stage_sigma_um * px_per_um + 1.0
    }
}

/// A pair of tiles, registered: the overlap graph's edge.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Edge {
    /// The first tile, by its index in the mosaic.
    pub i: usize,
    /// The second, after the first.
    pub j: usize,
    /// Where tile `j`'s corner is from tile `i`'s, in mosaic pixels, as measured; the nominal
    /// offset if the registration failed.
    pub shift: [f64; 2],
    /// The registration's peak-to-correlation energy: 0 if it failed.
    pub pce: f64,
    /// How far its refined peak stands above the next, from 0 to 1.
    pub margin: f64,
    /// Its overlap's correlation, from −1 to 1.
    pub ncc: f64,
    /// The overlap registered, width and height, in pixels.
    pub overlap_px: [u32; 2],
    /// Whether it is trusted: registered within the reach, and over the correlation, margin and
    /// energy thresholds.
    pub confident: bool,
}

/// Where two tiles' nominal overlap lies in each: the first's corner, the second's, and its size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Crops {
    a: [u32; 2],
    b: [u32; 2],
    size: [u32; 2],
}

impl Crops {
    /// The overlap of `a` and `b`, as whole pixels of each: `None` if it is under 16 pixels.
    fn of(a: &Tile, b: &Tile) -> Option<Self> {
        let (corner, size) = overlap(a, b)?;
        let into = |tile: &Tile, axis: usize| {
            let at = (corner[axis] - tile.nominal[axis]).round();
            at.clamp(0.0, f64::from(tile.size[axis])) as u32
        };
        let (oa, ob) = ([into(a, 0), into(a, 1)], [into(b, 0), into(b, 1)]);
        let side = |axis: usize| {
            (size[axis].floor() as u32)
                .min(a.size[axis] - oa[axis])
                .min(b.size[axis] - ob[axis])
        };
        let size = [side(0), side(1)];
        (size.iter().all(|&s| s >= MIN_SIDE)).then_some(Self { a: oa, b: ob, size })
    }

    fn rect(corner: [u32; 2], size: [u32; 2]) -> RectPx {
        RectPx::new(corner[0], corner[1], size[0], size[1])
    }
}

/// The memory one pair's registration holds at most, in bytes, given the largest overlap among
/// `pairs`: the overlaps flat-corrected as `f32`, the registrar's copies of them, and its FFTs and
/// refinement windows of up to 512 pixels a side.
pub fn bytes_per_pair(mosaic: &Mosaic, pairs: &[(usize, usize)]) -> u64 {
    let largest = pairs
        .iter()
        .filter_map(|&(i, j)| Crops::of(&mosaic.tiles[i], &mosaic.tiles[j]))
        .map(|crops| u64::from(crops.size[0]) * u64::from(crops.size[1]))
        .max()
        .unwrap_or(0);
    let fft = 512 * 512 * 4;
    16 * largest + 12 * fft
}

/// Registers each of `pairs` of `mosaic`'s tiles, their pixels from `source`, corrected by
/// `shading` (the dark level and the flat) if given: their edges, in order. At most `threads`
/// pairs are registered at once, on the calling rayon pool, so that a job's memory stays within
/// `threads` × [`bytes_per_pair`]. A pair that doesn't register is an edge that isn't confident;
/// a tile that can't be read is an error.
pub fn register(
    source: &(impl TileSource + ?Sized),
    mosaic: &Mosaic,
    pairs: &[(usize, usize)],
    shading: Option<(f32, &FlatField)>,
    options: &PairOptions,
    threads: usize,
) -> Result<Vec<Edge>, StitchError> {
    let reach = options.reach_px(mosaic.px_per_um);
    let registrar = Registrar::new(Options {
        reach: Some([reach; 2]),
        min_overlap: options.min_shared,
        ..Options::default()
    });
    let chunk = pairs.len().div_ceil(threads.max(1)).max(1);
    let edges = pairs
        .par_chunks(chunk)
        .map(|chunk| {
            chunk
                .iter()
                .map(|&pair| one(&registrar, source, mosaic, pair, shading, options))
                .collect::<Result<Vec<_>, _>>()
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(edges.into_iter().flatten().collect())
}

/// Registers the pair `(i, j)`.
fn one(
    registrar: &Registrar,
    source: &(impl TileSource + ?Sized),
    mosaic: &Mosaic,
    (i, j): (usize, usize),
    shading: Option<(f32, &FlatField)>,
    options: &PairOptions,
) -> Result<Edge, StitchError> {
    let (a, b) = (&mosaic.tiles[i], &mosaic.tiles[j]);
    let nominal = [b.nominal[0] - a.nominal[0], b.nominal[1] - a.nominal[1]];
    let failed = |overlap_px| Edge {
        i,
        j,
        shift: nominal,
        pce: 0.0,
        margin: 0.0,
        ncc: 0.0,
        overlap_px,
        confident: false,
    };
    let Some(crops) = Crops::of(a, b) else {
        return Ok(failed([0, 0]));
    };
    let (pa, pb) = (source.pixels(i)?, source.pixels(j)?);
    let (va, vb) = (
        pa.sub_view(Crops::rect(crops.a, crops.size))?,
        pb.sub_view(Crops::rect(crops.b, crops.size))?,
    );
    let found = match shading {
        None => registrar.register(va, vb),
        Some((dark, flat)) => {
            let (ca, cb) = (
                corrected(&va, a, crops.a, dark, flat),
                corrected(&vb, b, crops.b, dark, flat),
            );
            registrar.register(ca.view(), cb.view())
        }
    };
    let Ok(Registration {
        shift,
        pce,
        margin,
        ncc,
    }) = found
    else {
        return Ok(failed(crops.size));
    };
    // `b`'s crop shows at `x` what `a`'s shows at `x − shift`: their corners differ by the crops'
    // corners less the shift.
    let offset = |axis: usize| f64::from(crops.a[axis]) - f64::from(crops.b[axis]) - shift[axis];
    Ok(Edge {
        i,
        j,
        shift: [offset(0), offset(1)],
        pce,
        margin,
        ncc,
        overlap_px: crops.size,
        confident: pce >= options.min_pce && ncc >= options.min_ncc && margin >= options.min_margin,
    })
}

/// `view`, the part of `tile` from `corner`, as `(value − dark) × gain` by `flat`.
fn corrected(
    view: &ImageView<'_, u16>,
    tile: &Tile,
    corner: [u32; 2],
    dark: f32,
    flat: &FlatField,
) -> OwnedImage<f32> {
    let b = f64::from(tile.binning);
    let origin = [0, 1].map(|axis| f64::from(tile.sensor[axis]) + f64::from(corner[axis]) * b);
    let gains = flat.gains_on(origin, [b, b], (view.width(), view.height()));
    let gains = gains.view();
    OwnedImage::from_fn(view.width(), view.height(), |x, y| {
        (f32::from(view.row(y)[x]) - dark).max(0.0) * gains.row(y)[x]
    })
}
