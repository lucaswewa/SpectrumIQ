//! A real image as the world (REQ-SIM-005): a PNG or TIFF laid out at a pixel size, centred on
//! the sample origin.

use std::fs::File;
use std::io::{BufReader, Read, Seek};
use std::path::{Path, PathBuf};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use siq_types::{ImageView, Micrometers, OwnedImage, Vec2Um};

use crate::noise::mix_values as mix;
use crate::{SynthError, World, WorldInfo};

/// The parameters of [`ImageWorld`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ImageParams {
    /// The image: a PNG or a TIFF, grey or colour, 8 or 16 bits, or float for TIFF. Colour is
    /// reduced to luminance.
    pub path: PathBuf,
    /// The size of one image pixel on the sample.
    pub pixel_size: Micrometers,
}

/// An image centred on the sample origin, with column 0 at the lowest x and row 0 at the lowest
/// y. Outside it, the world is dark.
///
/// The image is loaded whole, as `f32`, with a pyramid of 2 × 2 means for coarse samplings: a
/// 20 000 × 20 000 image takes about 2 GB.
#[derive(Debug, Clone)]
pub struct ImageWorld {
    params: ImageParams,
    /// The image, then each level half the last.
    levels: Vec<OwnedImage<f32>>,
}

impl ImageWorld {
    /// The image at `params.path`.
    pub fn open(params: ImageParams) -> Result<Self, SynthError> {
        let image = read(&params.path)?;
        Ok(Self::from_image(params, image))
    }

    /// A world of `image`, whose intensities are from 0 to 1. `params.path` only names it.
    pub fn from_image(params: ImageParams, image: OwnedImage<f32>) -> Self {
        let mut levels = vec![image];
        while let Some(last) = levels
            .last()
            .filter(|level| level.width() > 1 || level.height() > 1)
        {
            let half = halve(&last.view());
            levels.push(half);
        }
        Self { params, levels }
    }

    /// The image's width and height, in pixels.
    pub fn size(&self) -> (usize, usize) {
        (self.levels[0].width(), self.levels[0].height())
    }
}

impl World for ImageWorld {
    /// Bilinear within a pyramid level, and linear between the two levels around the footprint.
    fn intensity(&self, at: Vec2Um, footprint: Micrometers) -> f32 {
        let pixel = self.params.pixel_size.0;
        let (width, height) = self.size();
        let corner = [
            -(width as f64) * pixel / 2.0,
            -(height as f64) * pixel / 2.0,
        ];
        let [x, y] = at.to_um();
        let at_level = |level: usize| {
            let scale = pixel * f64::from(1u32 << level);
            let (u, v) = ((x - corner[0]) / scale - 0.5, (y - corner[1]) / scale - 0.5);
            bilinear(&self.levels[level].view(), u, v)
        };
        let depth = (footprint.0 / pixel).max(1.0).log2();
        let level = (depth.floor() as usize).min(self.levels.len() - 1);
        let value = match self.levels.get(level + 1) {
            Some(_) => mix(at_level(level), at_level(level + 1), depth - level as f64),
            None => at_level(level),
        };
        value as f32
    }

    fn surface_z(&self, _at: Vec2Um) -> Option<Micrometers> {
        Some(Micrometers::ZERO)
    }

    fn info(&self) -> WorldInfo {
        WorldInfo::Image(self.params.clone())
    }
}

/// The image at `(u, v)`, in pixel-centre units, from its four nearest pixels. Pixels outside
/// the image are 0.
fn bilinear(image: &ImageView<'_, f32>, u: f64, v: f64) -> f64 {
    let (i, j) = (u.floor(), v.floor());
    let tap = |di: f64, dj: f64| {
        let (x, y) = (i + di, j + dj);
        let inside = x >= 0.0 && y >= 0.0;
        let pixel = inside.then(|| image.get(x as usize, y as usize)).flatten();
        pixel.map_or(0.0, |&p| f64::from(p))
    };
    let (fu, fv) = (u - i, v - j);
    mix(
        mix(tap(0.0, 0.0), tap(1.0, 0.0), fu),
        mix(tap(0.0, 1.0), tap(1.0, 1.0), fu),
        fv,
    )
}

/// The next pyramid level: each pixel the mean of the 2 × 2 block, or of what is left of it at
/// an odd edge.
fn halve(image: &ImageView<'_, f32>) -> OwnedImage<f32> {
    let (width, height) = (image.width().div_ceil(2), image.height().div_ceil(2));
    OwnedImage::from_fn(width, height, |x, y| {
        let block =
            [(0, 0), (1, 0), (0, 1), (1, 1)].map(|(dx, dy)| image.get(2 * x + dx, 2 * y + dy));
        let (sum, count) = block
            .iter()
            .flatten()
            .fold((0.0, 0.0), |(s, n), &&p| (s + p, n + 1.0));
        sum / count
    })
}

/// Decodes a PNG or a TIFF, by its first bytes.
fn read(path: &Path) -> Result<OwnedImage<f32>, SynthError> {
    let io = |source| SynthError::Io {
        path: path.to_owned(),
        source,
    };
    let bad = |message: String| SynthError::Image {
        path: path.to_owned(),
        message,
    };
    let mut file = BufReader::new(File::open(path).map_err(io)?);
    let mut magic = [0; 4];
    file.read_exact(&mut magic).map_err(io)?;
    file.rewind().map_err(io)?;
    match magic {
        [0x89, b'P', b'N', b'G'] => read_png(file).map_err(bad),
        [b'I', b'I', 42 | 43, 0] | [b'M', b'M', 0, 42 | 43] => read_tiff(file).map_err(bad),
        _ => Err(bad("it isn't a PNG or a TIFF".into())),
    }
}

fn read_png(file: BufReader<File>) -> Result<OwnedImage<f32>, String> {
    let limits = png::Limits { bytes: usize::MAX };
    let mut decoder = png::Decoder::new_with_limits(file, limits);
    decoder.set_transformations(png::Transformations::EXPAND);
    let mut reader = decoder.read_info().map_err(|error| error.to_string())?;
    let size = reader
        .output_buffer_size()
        .ok_or("the image is too large")?;
    let mut buffer = vec![0; size];
    let frame = reader
        .next_frame(&mut buffer)
        .map_err(|error| error.to_string())?;
    let channels = frame.color_type.samples();
    Ok(match frame.bit_depth {
        png::BitDepth::Sixteen => luminance(frame.width, frame.height, channels, |n| {
            f32::from(u16::from_be_bytes([buffer[2 * n], buffer[2 * n + 1]])) / 65535.0
        }),
        _ => luminance(frame.width, frame.height, channels, |n| {
            f32::from(buffer[n]) / 255.0
        }),
    })
}

fn read_tiff(file: BufReader<File>) -> Result<OwnedImage<f32>, String> {
    use tiff::ColorType::{Gray, GrayA, RGB, RGBA};
    use tiff::decoder::{Decoder, DecodingResult, Limits};
    let error = |error: tiff::TiffError| error.to_string();
    let decoder = Decoder::new(file).map_err(error)?;
    let mut decoder = decoder.with_limits(Limits::unlimited());
    let (width, height) = decoder.dimensions().map_err(error)?;
    let channels = match decoder.colortype().map_err(error)? {
        Gray(_) => 1,
        GrayA(_) => 2,
        RGB(_) => 3,
        RGBA(_) => 4,
        other => return Err(format!("{other:?} images aren't supported")),
    };
    match decoder.read_image().map_err(error)? {
        DecodingResult::U8(v) => Ok(luminance(width, height, channels, |n| {
            f32::from(v[n]) / 255.0
        })),
        DecodingResult::U16(v) => Ok(luminance(width, height, channels, |n| {
            f32::from(v[n]) / 65535.0
        })),
        DecodingResult::F32(v) => Ok(luminance(width, height, channels, |n| v[n])),
        _ => Err("only 8-bit, 16-bit and 32-bit float samples are supported".into()),
    }
}

/// The luminance of an image whose samples, `channels` per pixel, `sample` gives by index.
fn luminance(
    width: u32,
    height: u32,
    channels: usize,
    sample: impl Fn(usize) -> f32,
) -> OwnedImage<f32> {
    let width = width as usize;
    OwnedImage::from_fn(width, height as usize, |x, y| {
        let first = (y * width + x) * channels;
        if channels >= 3 {
            0.2126 * sample(first) + 0.7152 * sample(first + 1) + 0.0722 * sample(first + 2)
        } else {
            sample(first)
        }
    })
}
