//! Deep-zoom tiles, rendered on demand from an OME-Zarr image's levels (ADR-0045), for
//! OpenSeadragon.
//!
//! - **The geometry** is DZI's, as OpenSeadragon reads it: tiles of [`TILE`] pixels with an
//!   [`OVERLAP`] of 1. Level `max` is the image; each level below halves it, rounding up, down to
//!   level 0, one pixel. DZI level `l` is Zarr level `max − l`, which has the same size. Levels
//!   coarser than the image's top level are halved from it.
//! - **A tile** is the region read at its level, mapped through a [`TileLut`] (window, gamma,
//!   colours), and encoded as JPEG or PNG.
//! - **The [`TileService`]** caches tiles, bounded by bytes, least recently used first out, each
//!   with an ETag that changes when the image does.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::SystemTime;

use jpeg_encoder::{ColorType, Encoder};
use lru::LruCache;
use siq_imgproc::{Colormap, DisplayMapping, Lut};
use siq_storage::StorageError;
use siq_storage::zarr::{SampleType, ZarrImage};
use siq_types::{ImageView, RectPx};

use crate::mean_2x2;

/// A tile's size, in pixels, without its overlap.
pub const TILE: u64 = 254;
/// The pixels a tile shares with each neighbour.
pub const OVERLAP: u64 = 1;
/// The JPEG quality of tiles.
const QUALITY: u8 = 85;

/// The DZI levels and tiles of an image of `width` × `height`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DziGeometry {
    /// The image's width.
    pub width: u64,
    /// The image's height.
    pub height: u64,
}

impl DziGeometry {
    /// The full-resolution level: ⌈log₂ of the longer side⌉.
    pub fn max_level(&self) -> u32 {
        let side = self.width.max(self.height).max(1);
        u64::BITS - (side - 1).leading_zeros()
    }

    /// Level `level`'s size: the image halved `max − level` times, rounding up.
    pub fn level_size(&self, level: u32) -> (u64, u64) {
        let halvings = self.max_level().saturating_sub(level);
        let halve = |n: u64| n.div_ceil(1 << halvings).max(1);
        (halve(self.width), halve(self.height))
    }

    /// Level `level`'s tiles, across and down.
    pub fn tiles(&self, level: u32) -> (u64, u64) {
        let (width, height) = self.level_size(level);
        (width.div_ceil(TILE), height.div_ceil(TILE))
    }

    /// Tile (`col`, `row`)'s pixels at `level`, its overlap included, as OpenSeadragon's
    /// `DziTileSource` bounds them; `None` if there is no such tile.
    pub fn tile_rect(&self, level: u32, col: u64, row: u64) -> Option<RectPx> {
        let (cols, rows) = self.tiles(level);
        if level > self.max_level() || col >= cols || row >= rows {
            return None;
        }
        let (width, height) = self.level_size(level);
        let start = |i: u64| if i == 0 { 0 } else { TILE * i - OVERLAP };
        let length = |i: u64| TILE + if i == 0 { 1 } else { 2 } * OVERLAP;
        let (x, y) = (start(col), start(row));
        let (w, h) = (length(col).min(width - x), length(row).min(height - y));
        Some(RectPx::new(x as u32, y as u32, w as u32, h as u32))
    }

    /// The image's `.dzi` descriptor.
    pub fn descriptor(&self, format: TileFormat) -> String {
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Image \
             xmlns=\"http://schemas.microsoft.com/deepzoom/2008\" TileSize=\"{TILE}\" \
             Overlap=\"{OVERLAP}\" Format=\"{}\"><Size Width=\"{}\" Height=\"{}\"/></Image>\n",
            format.extension(),
            self.width,
            self.height
        )
    }
}

/// How tiles are encoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TileFormat {
    /// JPEG: small, opaque, lossy.
    Jpeg,
    /// PNG: lossless.
    Png,
}

impl TileFormat {
    /// The file extension, as DZI's `Format` names it.
    pub fn extension(self) -> &'static str {
        match self {
            Self::Jpeg => "jpeg",
            Self::Png => "png",
        }
    }

    /// The media type.
    pub fn content_type(self) -> &'static str {
        match self {
            Self::Jpeg => "image/jpeg",
            Self::Png => "image/png",
        }
    }

    /// The format of an extension: `jpeg`, `jpg` or `png`.
    pub fn from_extension(extension: &str) -> Option<Self> {
        match extension {
            "jpeg" | "jpg" => Some(Self::Jpeg),
            "png" => Some(Self::Png),
            _ => None,
        }
    }
}

/// How a tile's values become colours: the window shown black to full, a gamma (above 1
/// brightens the shadows, as `display.lut`'s), and the colours.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TileLut {
    /// The value shown black.
    pub min: f64,
    /// The value shown full.
    pub max: f64,
    /// The gamma.
    pub gamma: f64,
    /// The colours.
    pub colormap: Colormap,
}

impl TileLut {
    /// Checks the mapping: a window with `min < max`, and a gamma from 0.05 to 20.
    pub fn check(&self) -> Result<(), TileError> {
        let finite = self.min.is_finite() && self.max.is_finite();
        if !finite || self.min >= self.max {
            return Err(TileError::BadRequest(
                "the window needs min < max".to_owned(),
            ));
        }
        if !(0.05..=20.0).contains(&self.gamma) {
            return Err(TileError::BadRequest(
                "the gamma must be from 0.05 to 20".to_owned(),
            ));
        }
        Ok(())
    }

    fn key(&self) -> [u64; 4] {
        let colours = match self.colormap {
            Colormap::Grey => 0,
            Colormap::Inverted => 1,
            Colormap::Colour([r, g, b]) => {
                2 | u64::from(r) << 8 | u64::from(g) << 16 | u64::from(b) << 24
            }
        };
        [
            self.min.to_bits(),
            self.max.to_bits(),
            self.gamma.to_bits(),
            colours,
        ]
    }
}

/// Why a tile wasn't given.
#[derive(Debug, thiserror::Error)]
pub enum TileError {
    /// There is no such image, plane, level or tile.
    #[error("{0}")]
    NotFound(String),
    /// The request's parameters aren't valid.
    #[error("{0}")]
    BadRequest(String),
    /// The image couldn't be read.
    #[error(transparent)]
    Storage(#[from] StorageError),
    /// The tile couldn't be encoded.
    #[error("the tile couldn't be encoded: {0}")]
    Encode(String),
}

/// An encoded tile.
#[derive(Debug, Clone, PartialEq)]
pub struct Tile {
    /// Its bytes.
    pub bytes: Vec<u8>,
    /// Its format.
    pub format: TileFormat,
    /// Its ETag: the same while neither the image nor the request changes.
    pub etag: String,
}

/// Where a tile is: its image's plane (`t`, `c`, `z`), DZI level, column and row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TileAt {
    /// The plane: time point, channel and focus plane.
    pub plane: (u64, u64, u64),
    /// The DZI level.
    pub level: u32,
    /// The column.
    pub col: u64,
    /// The row.
    pub row: u64,
}

/// Renders tile `at` of `image`, mapped through `lut`: the channel's window and colour if none is
/// given.
pub fn render(
    image: &ZarrImage,
    at: TileAt,
    lut: Option<TileLut>,
    format: TileFormat,
) -> Result<Vec<u8>, TileError> {
    let info = image.info();
    let (t, c, z) = at.plane;
    if t >= info.size.t || c >= info.size.c || z >= info.size.z {
        return Err(TileError::NotFound(format!(
            "the image has no plane ({t}, {c}, {z})"
        )));
    }
    let geometry = DziGeometry {
        width: info.size.x,
        height: info.size.y,
    };
    let rect = geometry
        .tile_rect(at.level, at.col, at.row)
        .ok_or_else(|| {
            TileError::NotFound(format!(
                "level {} has no tile ({}, {})",
                at.level, at.col, at.row
            ))
        })?;
    let lut = lut.unwrap_or_else(|| channel_lut(image, c));
    lut.check()?;
    let zarr_level = (geometry.max_level() - at.level) as usize;
    let rgb = match info.sample {
        SampleType::U16 => {
            let (pixels, width) = read::<u16>(image, zarr_level, at.plane, rect)?;
            encode_rgb(&map_u16(&pixels, &lut)?, width, &lut, format)?
        }
        SampleType::F32 => {
            let (pixels, width) = read::<f32>(image, zarr_level, at.plane, rect)?;
            encode_rgb(&map_f32(&pixels, &lut), width, &lut, format)?
        }
    };
    Ok(rgb)
}

/// The value at (`x`, `y`) of Zarr level `level`'s plane: the true pixel, not a shown one.
pub fn value(
    image: &ZarrImage,
    plane: (u64, u64, u64),
    level: usize,
    x: u32,
    y: u32,
) -> Result<f64, TileError> {
    if level >= image.info().levels {
        return Err(TileError::NotFound(format!(
            "the image has no level {level}"
        )));
    }
    let rect = RectPx::new(x, y, 1, 1);
    let found = match image.info().sample {
        SampleType::U16 => image
            .read_region::<u16>(level, plane, rect)
            .map(|r| f64::from(r.pixels[0])),
        SampleType::F32 => image
            .read_region::<f32>(level, plane, rect)
            .map(|r| f64::from(r.pixels[0])),
    };
    found.map_err(|_| {
        TileError::NotFound(format!(
            "({x}, {y}) of plane {plane:?} isn't in level {level}"
        ))
    })
}

/// Channel `c`'s own window and colour, from the image's `omero` metadata: what a tile is shown
/// with unless told otherwise.
pub fn channel_lut(image: &ZarrImage, c: u64) -> TileLut {
    let channel = image.info().channels.get(c as usize);
    let (min, max) = channel.map_or((0.0, 65535.0), |channel| channel.window);
    let colormap = match channel.map(|channel| channel.colour) {
        None | Some([255, 255, 255]) => Colormap::Grey,
        Some(colour) => Colormap::Colour(colour),
    };
    TileLut {
        min,
        max,
        gamma: 1.0,
        colormap,
    }
}

/// `rect` of `plane` at a Zarr level, which may be coarser than the image's top level: then the
/// top level is halved down to it. The pixels, and the width.
fn read<T: crate::Mean>(
    image: &ZarrImage,
    level: usize,
    plane: (u64, u64, u64),
    rect: RectPx,
) -> Result<(Vec<T>, usize), TileError> {
    let top = image.info().levels - 1;
    if level <= top {
        let region = image.read_region::<T>(level, plane, rect)?;
        return Ok((region.pixels, rect.width as usize));
    }
    // A level beyond the top fits a tile or two: it is made whole, then cut.
    let size = image.info().size(top);
    let whole = RectPx::new(0, 0, size.x as u32, size.y as u32);
    let mut region = image.read_region::<T>(top, plane, whole)?;
    let mut width = size.x as usize;
    for _ in top..level {
        let height = region.pixels.len() / width;
        let view = ImageView::new(&region.pixels, width, height, width).expect("a whole level");
        let (pixels, w, _) = mean_2x2(view);
        (region.pixels, width) = (pixels, w);
    }
    let (x0, y0, w) = (rect.x as usize, rect.y as usize, rect.width as usize);
    let pixels = (y0..y0 + rect.height as usize)
        .flat_map(|y| region.pixels[y * width + x0..][..w].iter().copied())
        .collect();
    Ok((pixels, w))
}

/// Display levels of `u16` values, through `siq-imgproc`'s tables.
fn map_u16(pixels: &[u16], lut: &TileLut) -> Result<Vec<u8>, TileError> {
    let clamp = |v: f64| v.round().clamp(0.0, 65535.0) as u16;
    let (min, max) = (clamp(lut.min), clamp(lut.max));
    let mapping = DisplayMapping {
        min,
        max: max.max(min.saturating_add(1)),
        gamma: lut.gamma as f32,
        colormap: lut.colormap,
    };
    let table = Lut::new(&mapping).map_err(|error| TileError::BadRequest(error.to_string()))?;
    Ok(pixels.iter().map(|&p| table.level(p)).collect())
}

/// Display levels of `f32` values, by the same formula.
fn map_f32(pixels: &[f32], lut: &TileLut) -> Vec<u8> {
    let span = lut.max - lut.min;
    pixels
        .iter()
        .map(|&p| {
            let t = ((f64::from(p) - lut.min) / span).clamp(0.0, 1.0);
            let shown = (t.powf(1.0 / lut.gamma) * 255.0).round() as u8;
            if lut.colormap == Colormap::Inverted {
                255 - shown
            } else {
                shown
            }
        })
        .collect()
}

/// Encodes display levels: grey, or in the colormap's colour.
fn encode_rgb(
    levels: &[u8],
    width: usize,
    lut: &TileLut,
    format: TileFormat,
) -> Result<Vec<u8>, TileError> {
    let height = levels.len() / width.max(1);
    let (bytes, colour) = match lut.colormap {
        Colormap::Colour(colour) => {
            let rgb = levels
                .iter()
                .flat_map(|&level| {
                    colour.map(|c| ((u32::from(c) * u32::from(level) + 127) / 255) as u8)
                })
                .collect();
            (rgb, true)
        }
        _ => (levels.to_vec(), false),
    };
    let encode_error = |error: &dyn std::fmt::Display| TileError::Encode(error.to_string());
    let mut out = Vec::new();
    match format {
        TileFormat::Jpeg => {
            let kind = if colour {
                ColorType::Rgb
            } else {
                ColorType::Luma
            };
            Encoder::new(&mut out, QUALITY)
                .encode(&bytes, width as u16, height as u16, kind)
                .map_err(|e| encode_error(&e))?;
        }
        TileFormat::Png => {
            let mut encoder = png::Encoder::new(&mut out, width as u32, height as u32);
            encoder.set_color(if colour {
                png::ColorType::Rgb
            } else {
                png::ColorType::Grayscale
            });
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().map_err(|e| encode_error(&e))?;
            writer
                .write_image_data(&bytes)
                .map_err(|e| encode_error(&e))?;
        }
    }
    Ok(out)
}

/// What the cache holds and has done.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CacheStats {
    /// Tiles held.
    pub tiles: usize,
    /// Their bytes.
    pub bytes: usize,
    /// Tiles given from the cache.
    pub hits: u64,
    /// Tiles rendered.
    pub misses: u64,
    /// Tiles let go to stay within the budget.
    pub evictions: u64,
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct TileKey {
    image: PathBuf,
    generation: u64,
    at: TileAt,
    lut: Option<[u64; 4]>,
    format: TileFormat,
}

struct Cache {
    tiles: LruCache<TileKey, Arc<Tile>>,
    stats: CacheStats,
}

/// An image opened, and its generation: when its metadata last changed.
struct Opened {
    image: Arc<ZarrImage>,
    generation: u64,
}

/// Tiles of OME-Zarr images, rendered on demand and kept within a budget of bytes.
pub struct TileService {
    budget: usize,
    cache: Mutex<Cache>,
    images: Mutex<HashMap<PathBuf, Opened>>,
}

impl TileService {
    /// A service keeping up to `budget` bytes of tiles.
    pub fn new(budget: usize) -> Self {
        Self {
            budget,
            cache: Mutex::new(Cache {
                tiles: LruCache::unbounded(),
                stats: CacheStats::default(),
            }),
            images: Mutex::new(HashMap::new()),
        }
    }

    /// The image at `path`, opened once and again whenever its metadata changes, as when its
    /// pyramid is built.
    pub fn image(&self, path: &Path) -> Result<(Arc<ZarrImage>, u64), TileError> {
        let metadata = path.join("zarr.json");
        let modified = std::fs::metadata(&metadata)
            .and_then(|m| m.modified())
            .map_err(|_| {
                TileError::NotFound(format!("{} isn't an OME-Zarr image", path.display()))
            })?;
        let generation = modified
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos() as u64);
        let mut images = self.images.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(opened) = images
            .get(path)
            .filter(|opened| opened.generation == generation)
        {
            return Ok((Arc::clone(&opened.image), generation));
        }
        let image = Arc::new(ZarrImage::open(path)?);
        images.insert(
            path.to_owned(),
            Opened {
                image: Arc::clone(&image),
                generation,
            },
        );
        Ok((image, generation))
    }

    /// The `.dzi` descriptor of the image at `path`.
    pub fn descriptor(&self, path: &Path, format: TileFormat) -> Result<String, TileError> {
        let (image, _) = self.image(path)?;
        let size = image.info().size;
        Ok(DziGeometry {
            width: size.x,
            height: size.y,
        }
        .descriptor(format))
    }

    /// Tile `at` of the image at `path`, from the cache or rendered.
    pub fn tile(
        &self,
        path: &Path,
        at: TileAt,
        lut: Option<TileLut>,
        format: TileFormat,
    ) -> Result<Arc<Tile>, TileError> {
        let (image, generation) = self.image(path)?;
        let key = TileKey {
            image: path.to_owned(),
            generation,
            at,
            lut: lut.map(|l| l.key()),
            format,
        };
        {
            let mut cache = self.lock();
            if let Some(tile) = cache.tiles.get(&key).cloned() {
                cache.stats.hits += 1;
                return Ok(tile);
            }
        }
        let bytes = render(&image, at, lut, format)?;
        let tile = Arc::new(Tile {
            etag: etag(&key),
            bytes,
            format,
        });
        let mut cache = self.lock();
        cache.stats.misses += 1;
        if cache.tiles.put(key, Arc::clone(&tile)).is_none() {
            cache.stats.bytes += tile.bytes.len();
            cache.stats.tiles += 1;
        }
        while cache.stats.bytes > self.budget {
            let Some((_, old)) = cache.tiles.pop_lru() else {
                break;
            };
            cache.stats.bytes -= old.bytes.len();
            cache.stats.tiles -= 1;
            cache.stats.evictions += 1;
        }
        Ok(tile)
    }

    /// The cache's contents and counts.
    pub fn stats(&self) -> CacheStats {
        self.lock().stats
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Cache> {
        self.cache.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl std::fmt::Debug for TileService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TileService")
            .field("budget", &self.budget)
            .field("cache", &self.stats())
            .finish()
    }
}

/// A tile's ETag: an FNV-1a hash of its key, stable across restarts.
fn etag(key: &TileKey) -> String {
    struct Fnv(u64);
    impl Hasher for Fnv {
        fn finish(&self) -> u64 {
            self.0
        }
        fn write(&mut self, bytes: &[u8]) {
            for &byte in bytes {
                self.0 = (self.0 ^ u64::from(byte)).wrapping_mul(0x100_0000_01b3);
            }
        }
    }
    let mut hasher = Fnv(0xcbf2_9ce4_8422_2325);
    key.hash(&mut hasher);
    format!("\"{:016x}\"", hasher.finish())
}
