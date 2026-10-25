//! OME-Zarr images (ADR-0042): NGFF 0.5, a group of Zarr v3 arrays, one per resolution level, with
//! the axes `tczyx`. Chunks are 1 × 1 × 1 × 512 × 512 pixels by default, compressed with zstd.
//!
//! - The group's `zarr.json` holds the NGFF metadata under `ome`: `multiscales` (the axes, with
//!   space in µm; each level's scale and translation) and `omero` (each channel's label, colour and
//!   window).
//! - Level `l` halves the one before it in `y` and `x`, rounding up, so its pixel is 2ˡ of level
//!   0's. Its translation centres it on the level-0 pixels it covers.
//! - The writer stores one chunk at a time, in any order and from any thread
//!   ([`ZarrImage::write_chunk`]). Chunks at the right and bottom edges are given clipped, and
//!   padded with zeros on disk, as Zarr stores them.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{Map, Value, json};
use siq_types::{ImageView, RectPx};
use zarrs::array::codec::ZstdCodec;
use zarrs::array::{Array, ArrayBuilder, ArrayMetadataOptions, ArraySubset, data_type};
use zarrs::filesystem::FilesystemStore;
use zarrs::group::{Group, GroupBuilder};

use crate::StorageError;

/// The NGFF version written.
pub const NGFF_VERSION: &str = "0.5";
/// The default chunk's width and height.
pub const CHUNK: u64 = 512;
/// The zstd level: fast, for chunks written live and by jobs.
const ZSTD_LEVEL: i32 = 3;
/// The axes, in order.
const AXES: [&str; 5] = ["t", "c", "z", "y", "x"];

/// What an image's pixels are.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SampleType {
    /// Unsigned 16-bit: raw and stitched frames.
    U16,
    /// 32-bit float: processed results.
    F32,
}

/// A pixel type an image can hold: `u16` or `f32`.
pub trait ZarrSample:
    zarrs::array::Element + zarrs::array::ElementOwned + Copy + Default + Send + Sync + 'static
{
    /// Its [`SampleType`].
    const TYPE: SampleType;
}

impl ZarrSample for u16 {
    const TYPE: SampleType = SampleType::U16;
}

impl ZarrSample for f32 {
    const TYPE: SampleType = SampleType::F32;
}

/// A channel's label, and how viewers show it (NGFF's `omero`).
#[derive(Debug, Clone, PartialEq)]
pub struct ChannelInfo {
    /// Its label, such as `DAPI`.
    pub label: String,
    /// Its display colour.
    pub colour: [u8; 3],
    /// The values shown black and white: the window's start and end.
    pub window: (f64, f64),
}

/// An image's size: time points, channels, planes, rows and columns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Size5 {
    /// Time points.
    pub t: u64,
    /// Channels.
    pub c: u64,
    /// Focus planes.
    pub z: u64,
    /// Rows.
    pub y: u64,
    /// Columns.
    pub x: u64,
}

impl Size5 {
    fn to_vec(self) -> Vec<u64> {
        vec![self.t, self.c, self.z, self.y, self.x]
    }
}

/// What an image is: its metadata.
#[derive(Debug, Clone, PartialEq)]
pub struct ImageInfo {
    /// Its name.
    pub name: String,
    /// Its pixels' type.
    pub sample: SampleType,
    /// Level 0's size.
    pub size: Size5,
    /// The resolution levels, at least 1.
    pub levels: usize,
    /// A chunk's width and height.
    pub chunk: u64,
    /// Level 0's pixel size, in µm: `z`, `y`, `x`.
    pub pixel_size: [f64; 3],
    /// Where level 0's first pixel's centre is, in µm: `z`, `y`, `x`.
    pub origin: [f64; 3],
    /// The channels, one per `c`.
    pub channels: Vec<ChannelInfo>,
}

impl ImageInfo {
    /// Level `level`'s size: level 0 halved `level` times in `y` and `x`, rounding up.
    pub fn size(&self, level: usize) -> Size5 {
        let halve = |n: u64| n.div_ceil(1 << level).max(1);
        Size5 {
            y: halve(self.size.y),
            x: halve(self.size.x),
            ..self.size
        }
    }

    /// Level `level`'s chunks, down and across.
    pub fn chunks(&self, level: usize) -> (u64, u64) {
        let size = self.size(level);
        (size.y.div_ceil(self.chunk), size.x.div_ceil(self.chunk))
    }

    /// The NGFF metadata of the group's attributes.
    fn attributes(&self) -> Map<String, Value> {
        let [dz, dy, dx] = self.pixel_size;
        let [z0, y0, x0] = self.origin;
        let datasets: Vec<Value> = (0..self.levels)
            .map(|level| {
                let factor = f64::from(1u32 << level);
                // A level's pixel covers `factor` of level 0's: its centre is half that, less half
                // a level-0 pixel, along.
                let shift = (factor - 1.0) / 2.0;
                json!({
                    "path": level.to_string(),
                    "coordinateTransformations": [
                        { "type": "scale", "scale": [1.0, 1.0, dz, dy * factor, dx * factor] },
                        {
                            "type": "translation",
                            "translation": [0.0, 0.0, z0, y0 + shift * dy, x0 + shift * dx],
                        },
                    ],
                })
            })
            .collect();
        let channels: Vec<Value> = self
            .channels
            .iter()
            .map(|channel| {
                let [r, g, b] = channel.colour;
                let (start, end) = channel.window;
                json!({
                    "label": channel.label,
                    "color": format!("{r:02X}{g:02X}{b:02X}"),
                    "window": { "start": start, "end": end, "min": 0.0, "max": self.full_scale() },
                    "active": true,
                })
            })
            .collect();
        let ome = json!({
            "version": NGFF_VERSION,
            "multiscales": [{
                "name": self.name,
                "axes": [
                    { "name": "t", "type": "time", "unit": "second" },
                    { "name": "c", "type": "channel" },
                    { "name": "z", "type": "space", "unit": "micrometer" },
                    { "name": "y", "type": "space", "unit": "micrometer" },
                    { "name": "x", "type": "space", "unit": "micrometer" },
                ],
                "datasets": datasets,
                "type": "mean",
            }],
            "omero": { "channels": channels, "rdefs": { "model": "color" } },
        });
        Map::from_iter([("ome".to_owned(), ome)])
    }

    fn full_scale(&self) -> f64 {
        match self.sample {
            SampleType::U16 => f64::from(u16::MAX),
            SampleType::F32 => 1.0,
        }
    }
}

/// A region read from an image: its rectangle, and its pixels, row by row with no gaps.
#[derive(Debug, Clone, PartialEq)]
pub struct Region<T> {
    /// Where it is, in the level's pixels.
    pub rect: RectPx,
    /// Its pixels.
    pub pixels: Vec<T>,
}

impl<T> Region<T> {
    /// Its pixels as an image.
    pub fn view(&self) -> ImageView<'_, T> {
        let width = self.rect.width as usize;
        ImageView::new(&self.pixels, width, self.rect.height as usize, width)
            .expect("a region holds its rectangle's pixels")
    }
}

/// An OME-Zarr image, on disk.
pub struct ZarrImage {
    path: PathBuf,
    info: ImageInfo,
    levels: Vec<Array<FilesystemStore>>,
}

impl ZarrImage {
    /// Makes the image at `path`, a new folder such as `zarr/mosaic.ome.zarr`, with its metadata
    /// and an empty array for each level.
    pub fn create(path: &Path, info: ImageInfo) -> Result<Self, StorageError> {
        let wrong = |why: &str| zarr_error(path, why);
        if info.levels == 0 || info.chunk == 0 || info.channels.len() as u64 != info.size.c {
            return Err(wrong(
                "an image needs a level, a chunk size, and a channel per `c`",
            ));
        }
        std::fs::create_dir_all(path).map_err(crate::io_at(path))?;
        let store = store(path)?;
        let group = GroupBuilder::new()
            .attributes(info.attributes())
            .build(store.clone(), "/")
            .map_err(|error| wrong(&error.to_string()))?;
        group
            .store_metadata()
            .map_err(|error| wrong(&error.to_string()))?;
        let levels = (0..info.levels)
            .map(|level| {
                let chunk = vec![1, 1, 1, info.chunk, info.chunk];
                let shape = info.size(level).to_vec();
                let mut builder = match info.sample {
                    SampleType::U16 => ArrayBuilder::new(shape, chunk, data_type::uint16(), 0u16),
                    SampleType::F32 => ArrayBuilder::new(shape, chunk, data_type::float32(), 0f32),
                };
                let array = builder
                    .bytes_to_bytes_codecs(vec![Arc::new(ZstdCodec::new(ZSTD_LEVEL, false))])
                    .dimension_names(Some(AXES))
                    .build(store.clone(), &format!("/{level}"))
                    .map_err(|error| wrong(&error.to_string()))?;
                // Without zarrs's own attribute: the metadata is NGFF's and Zarr's only.
                let options = ArrayMetadataOptions::default().with_include_zarrs_metadata(false);
                array
                    .store_metadata_opt(&options)
                    .map_err(|error| wrong(&error.to_string()))?;
                Ok(array)
            })
            .collect::<Result<_, StorageError>>()?;
        Ok(Self {
            path: path.to_owned(),
            info,
            levels,
        })
    }

    /// Opens the image at `path`: an NGFF 0.5 multiscale image with the axes `tczyx`.
    pub fn open(path: &Path) -> Result<Self, StorageError> {
        let store = store(path)?;
        let group =
            Group::open(store.clone(), "/").map_err(|e| zarr_error(path, &e.to_string()))?;
        let info = read_info(group.attributes()).map_err(|why| zarr_error(path, &why))?;
        let levels: Vec<_> = (0..info.levels)
            .map(|level| Array::open(store.clone(), &format!("/{level}")))
            .collect::<Result<_, _>>()
            .map_err(|error| zarr_error(path, &error.to_string()))?;
        let sample = if levels[0].data_type() == &data_type::uint16() {
            SampleType::U16
        } else if levels[0].data_type() == &data_type::float32() {
            SampleType::F32
        } else {
            return Err(zarr_error(
                path,
                "its pixels are neither uint16 nor float32",
            ));
        };
        let size = levels[0].shape();
        let size = Size5 {
            t: size[0],
            c: size[1],
            z: size[2],
            y: size[3],
            x: size[4],
        };
        let chunk = levels[0]
            .chunk_shape(&[0; 5])
            .map_or(CHUNK, |shape| shape[4].get());
        Ok(Self {
            path: path.to_owned(),
            info: ImageInfo {
                sample,
                size,
                chunk,
                ..info
            },
            levels,
        })
    }

    /// Its folder.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// What it is.
    pub fn info(&self) -> &ImageInfo {
        &self.info
    }

    /// Stores chunk (`row`, `col`) of plane (`t`, `c`, `z`) at `level`: `pixels`, the chunk clipped
    /// to the level's edges. Chunks may be written in any order, from any thread, but each by one
    /// writer at a time.
    pub fn write_chunk<T: ZarrSample>(
        &self,
        level: usize,
        (t, c, z): (u64, u64, u64),
        (row, col): (u64, u64),
        pixels: ImageView<'_, T>,
    ) -> Result<(), StorageError> {
        let array = self.level::<T>(level)?;
        let chunk = self.info.chunk;
        let size = self.info.size(level);
        let (rows, cols) = (
            chunk.min(size.y.saturating_sub(row * chunk)),
            chunk.min(size.x.saturating_sub(col * chunk)),
        );
        if rows == 0 || cols == 0 || pixels.height() as u64 != rows || pixels.width() as u64 != cols
        {
            return Err(zarr_error(
                &self.path,
                &format!(
                    "chunk ({row}, {col}) of level {level} is {rows} × {cols}, not {} × {}",
                    pixels.height(),
                    pixels.width()
                ),
            ));
        }
        // On disk a chunk is always whole; past the edges it holds zeros.
        let mut whole = vec![T::default(); (chunk * chunk) as usize];
        for (y, source) in pixels.rows().enumerate() {
            whole[y * chunk as usize..][..cols as usize].copy_from_slice(source);
        }
        array
            .store_chunk(&[t, c, z, row, col], whole.as_slice())
            .map_err(|error| zarr_error(&self.path, &error.to_string()))
    }

    /// Reads `rect` of plane (`t`, `c`, `z`) at `level`, in the level's pixels.
    pub fn read_region<T: ZarrSample>(
        &self,
        level: usize,
        (t, c, z): (u64, u64, u64),
        rect: RectPx,
    ) -> Result<Region<T>, StorageError> {
        let array = self.level::<T>(level)?;
        let size = self.info.size(level);
        if rect.right() > size.x
            || rect.bottom() > size.y
            || t >= size.t
            || c >= size.c
            || z >= size.z
        {
            let why = format!("{rect:?} of plane ({t}, {c}, {z}) isn't in level {level}");
            return Err(zarr_error(&self.path, &why));
        }
        let (y, x) = (u64::from(rect.y), u64::from(rect.x));
        let subset = ArraySubset::new_with_ranges(&[
            t..t + 1,
            c..c + 1,
            z..z + 1,
            y..y + u64::from(rect.height),
            x..x + u64::from(rect.width),
        ]);
        let pixels = array
            .retrieve_array_subset::<Vec<T>>(&subset)
            .map_err(|error| zarr_error(&self.path, &error.to_string()))?;
        Ok(Region { rect, pixels })
    }

    fn level<T: ZarrSample>(&self, level: usize) -> Result<&Array<FilesystemStore>, StorageError> {
        if T::TYPE != self.info.sample {
            return Err(zarr_error(&self.path, "its pixels aren't of that type"));
        }
        self.levels
            .get(level)
            .ok_or_else(|| zarr_error(&self.path, &format!("it has no level {level}")))
    }
}

impl std::fmt::Debug for ZarrImage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ZarrImage")
            .field("path", &self.path)
            .field("info", &self.info)
            .finish_non_exhaustive()
    }
}

fn store(path: &Path) -> Result<Arc<FilesystemStore>, StorageError> {
    FilesystemStore::new(path)
        .map(Arc::new)
        .map_err(|error| zarr_error(path, &error.to_string()))
}

fn zarr_error(path: &Path, why: &str) -> StorageError {
    StorageError::Zarr {
        path: path.to_owned(),
        why: why.to_owned(),
    }
}

/// The metadata of an NGFF 0.5 group's attributes, but the sample type, size and chunk, which come
/// from level 0's array.
fn read_info(attributes: &Map<String, Value>) -> Result<ImageInfo, String> {
    let ome = &attributes.get("ome").ok_or("it has no `ome` metadata")?;
    if ome["version"] != NGFF_VERSION {
        return Err(format!(
            "its NGFF version is {}, not {NGFF_VERSION}",
            ome["version"]
        ));
    }
    let multiscale = &ome["multiscales"][0];
    let axes: Vec<&str> = multiscale["axes"]
        .as_array()
        .ok_or("it has no axes")?
        .iter()
        .filter_map(|axis| axis["name"].as_str())
        .collect();
    if axes != AXES {
        return Err(format!("its axes are {axes:?}, not tczyx"));
    }
    let datasets = multiscale["datasets"]
        .as_array()
        .ok_or("it has no levels")?;
    let transform = |kind: &str| -> Result<[f64; 3], String> {
        let values = datasets[0]["coordinateTransformations"]
            .as_array()
            .and_then(|all| all.iter().find(|each| each["type"] == kind))
            .and_then(|each| each[kind].as_array())
            .ok_or(format!("level 0 has no {kind}"))?;
        let value = |i: usize| values.get(i).and_then(Value::as_f64).unwrap_or(0.0);
        Ok([value(2), value(3), value(4)])
    };
    let channels = ome["omero"]["channels"]
        .as_array()
        .map(|channels| channels.iter().map(channel_info).collect())
        .unwrap_or_default();
    Ok(ImageInfo {
        name: multiscale["name"].as_str().unwrap_or_default().to_owned(),
        sample: SampleType::U16,
        size: Size5 {
            t: 0,
            c: 0,
            z: 0,
            y: 0,
            x: 0,
        },
        levels: datasets.len(),
        chunk: CHUNK,
        pixel_size: transform("scale")?,
        origin: transform("translation").unwrap_or([0.0; 3]),
        channels,
    })
}

fn channel_info(channel: &Value) -> ChannelInfo {
    let colour =
        u32::from_str_radix(channel["color"].as_str().unwrap_or("FFFFFF"), 16).unwrap_or(0xFF_FFFF);
    let [_, r, g, b] = colour.to_be_bytes();
    let window = &channel["window"];
    ChannelInfo {
        label: channel["label"].as_str().unwrap_or_default().to_owned(),
        colour: [r, g, b],
        window: (
            window["start"].as_f64().unwrap_or(0.0),
            window["end"].as_f64().unwrap_or(0.0),
        ),
    }
}
