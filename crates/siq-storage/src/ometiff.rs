//! OME-TIFF export (ADR-0046): an OME-Zarr image as one BigTIFF file, which Fiji, QuPath and other
//! tools open through Bio-Formats.
//!
//! - **One IFD per plane,** with level 0, in the order `XYZCT`: `z` fastest, then `c`, then `t`.
//! - **The other levels are each plane's SubIFDs,** finest first, marked reduced-resolution.
//! - **Tiles** of 512 × 512, padded with zeros at the edges, deflated or not compressed.
//! - **The OME-XML** is in the first IFD's ImageDescription, in UTF-8: the image's size, type,
//!   pixel size and channels, and what the dataset's manifest says of it.
//!
//! The tiles are read from the levels, encoded in parallel on the calling rayon pool, and written
//! in order, a batch at a time: the memory held is a batch of tiles, whatever the image's size.
//! The file is written as `<name>.part` beside its name, and renamed once it is whole.

use std::fmt::Write as _;
use std::fs::File;
use std::io::{BufWriter, Seek, Write};
use std::path::{Path, PathBuf};

use chrono::SecondsFormat;
use flate2::write::ZlibEncoder;
use rayon::prelude::*;
use siq_types::RectPx;
use tiff::encoder::{DirectoryEncoder, Ifd8, TiffEncoder, TiffKindBig};
use tiff::tags::{Tag, Type};

use crate::zarr::{ImageInfo, SampleType, ZarrImage, ZarrSample};
use crate::{Control, Manifest, StorageError, io_at};

/// A tile's width and height.
pub const TILE: u32 = 512;
/// The tiles encoded at once, in parallel, before they are written.
const BATCH: usize = 64;

/// How the tiles are compressed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TiffCompression {
    /// Not at all: the largest file, the fastest to write and to read.
    None,
    /// Deflate (code 8) at zlib's default level: lossless, and read by every TIFF reader.
    #[default]
    Deflate,
}

/// The memory an export of `info` holds at most, in bytes, with `threads` encoding.
pub fn estimate(info: &ImageInfo, threads: usize) -> u64 {
    let tile = u64::from(TILE * TILE) * sample_bytes(info.sample);
    // A batch of encoded tiles, and each thread's region, tile and encoder.
    (BATCH as u64 + 3 * threads as u64) * tile
}

/// Writes `image` to `out` as OME-TIFF: every plane, with its other levels as SubIFDs, and the
/// OME-XML of [`ome_xml`]. Reports to `control`, and stops if it is cancelled, leaving no file.
pub fn export(
    image: &ZarrImage,
    manifest: Option<&Manifest>,
    out: &Path,
    compression: TiffCompression,
    control: &dyn Control,
) -> Result<(), StorageError> {
    let mut part = out.as_os_str().to_owned();
    part.push(".part");
    let part = PathBuf::from(part);
    let info = image.info();
    let tiles: u64 = (0..info.levels).map(|level| tiles(info, level).0).sum();
    let mut export = Export {
        image,
        compression,
        control,
        part: &part,
        done: 0,
        total: tiles * info.size.t * info.size.c * info.size.z,
    };
    let xml = ome_xml(info, manifest);
    let written = match info.sample {
        SampleType::U16 => export.write::<u16>(&xml),
        SampleType::F32 => export.write::<f32>(&xml),
    };
    match written {
        Ok(()) => std::fs::rename(&part, out).map_err(io_at(out)),
        Err(error) => {
            let _ = std::fs::remove_file(&part);
            Err(error)
        }
    }
}

/// An export under way.
struct Export<'a> {
    image: &'a ZarrImage,
    compression: TiffCompression,
    control: &'a dyn Control,
    /// The file being written.
    part: &'a Path,
    /// The tiles written, of every plane and level.
    done: u64,
    total: u64,
}

impl Export<'_> {
    fn write<T: Sample>(&mut self, xml: &str) -> Result<(), StorageError> {
        let size = self.image.info().size;
        let file = File::create(self.part).map_err(io_at(self.part))?;
        let mut file = BufWriter::with_capacity(1 << 20, file);
        let mut tiff = TiffEncoder::new_big(&mut file).map_err(self.tiff())?;
        let mut description = Some(xml);
        for (t, c, z) in (0..size.t)
            .flat_map(|t| (0..size.c).flat_map(move |c| (0..size.z).map(move |z| (t, c, z))))
        {
            // The SubIFDs first, so that the plane's IFD can point to them.
            let mut subifds = Vec::new();
            for level in 1..self.image.info().levels {
                let mut ifd = tiff.extra_directory().map_err(self.tiff())?;
                self.level::<T, _>(&mut ifd, (t, c, z), level)?;
                subifds.push(Ifd8(ifd.finish_with_offsets().map_err(self.tiff())?.offset));
            }
            let mut ifd = tiff.image_directory().map_err(self.tiff())?;
            self.level::<T, _>(&mut ifd, (t, c, z), 0)?;
            if let Some(xml) = description.take() {
                // In UTF-8, as OME-TIFF readers take it: the encoder's text is ASCII only.
                let text = [xml.as_bytes(), &[0]].concat();
                let entry = ifd
                    .write_entry_bytes(Type::ASCII, &text)
                    .map_err(self.tiff())?;
                let mut description = tiff::Directory::empty();
                description.extend([(Tag::ImageDescription, entry)]);
                ifd.extend_from(&description);
                align(&mut ifd).map_err(self.tiff())?;
            }
            if !subifds.is_empty() {
                ifd.write_tag(Tag::SubIfd, subifds.as_slice())
                    .map_err(self.tiff())?;
            }
            ifd.finish().map_err(self.tiff())?;
        }
        let file = file
            .into_inner()
            .map_err(|error| io_at(self.part)(error.into_error()))?;
        file.sync_all().map_err(io_at(self.part))
    }

    /// Writes `plane`'s `level` in `ifd`: its tiles, and the tags that describe them.
    fn level<T: Sample, W: Write + Seek>(
        &mut self,
        ifd: &mut DirectoryEncoder<'_, W, TiffKindBig>,
        plane: (u64, u64, u64),
        level: usize,
    ) -> Result<(), StorageError> {
        let size = self.image.info().size(level);
        let (count, cols) = tiles(self.image.info(), level);
        let (mut offsets, mut counts) = (Vec::new(), Vec::new());
        let at: Vec<u64> = (0..count).collect();
        for batch in at.chunks(BATCH) {
            if self.control.cancelled() {
                return Err(StorageError::Cancelled);
            }
            let encoded = batch
                .par_iter()
                .map(|k| self.tile::<T>(plane, level, (k / cols, k % cols)))
                .collect::<Result<Vec<_>, _>>()?;
            for bytes in encoded {
                offsets.push(ifd.write_data(bytes.as_slice()).map_err(self.tiff())?);
                counts.push(bytes.len() as u64);
            }
            self.done += batch.len() as u64;
            self.control.progress(self.done as f64 / self.total as f64);
        }
        // The arrays of offsets, and the IFD after them, start on a word boundary.
        align(ifd).map_err(self.tiff())?;
        let code = match self.compression {
            TiffCompression::None => 1,
            TiffCompression::Deflate => 8,
        };
        let bits = 8 * sample_bytes(T::TYPE) as u16;
        // Grey, one sample a pixel, black at zero.
        for (tag, value) in [
            (Tag::BitsPerSample, bits),
            (Tag::Compression, code),
            (Tag::PhotometricInterpretation, 1),
            (Tag::SamplesPerPixel, 1),
            (Tag::PlanarConfiguration, 1),
            (Tag::SampleFormat, T::FORMAT),
        ] {
            ifd.write_tag(tag, value).map_err(self.tiff())?;
        }
        let long =
            |n: u64| u32::try_from(n).map_err(|_| self.tiff()(tiff::TiffError::IntSizeError));
        for (tag, value) in [
            (Tag::NewSubfileType, u32::from(level > 0)),
            (Tag::ImageWidth, long(size.x)?),
            (Tag::ImageLength, long(size.y)?),
            (Tag::TileWidth, TILE),
            (Tag::TileLength, TILE),
        ] {
            ifd.write_tag(tag, value).map_err(self.tiff())?;
        }
        ifd.write_tag(Tag::TileOffsets, offsets.as_slice())
            .map_err(self.tiff())?;
        ifd.write_tag(Tag::TileByteCounts, counts.as_slice())
            .map_err(self.tiff())
    }

    /// The tile at (`row`, `col`) of `plane`'s `level`, padded and compressed.
    fn tile<T: Sample>(
        &self,
        plane: (u64, u64, u64),
        level: usize,
        (row, col): (u64, u64),
    ) -> Result<Vec<u8>, StorageError> {
        let size = self.image.info().size(level);
        let (x, y, tile) = (
            col * u64::from(TILE),
            row * u64::from(TILE),
            u64::from(TILE),
        );
        let rect = RectPx::new(
            x as u32,
            y as u32,
            tile.min(size.x - x) as u32,
            tile.min(size.y - y) as u32,
        );
        let region = self.image.read_region::<T>(level, plane, rect)?;
        let bytes = sample_bytes(T::TYPE) as usize;
        let whole = TILE as usize * TILE as usize * bytes;
        let mut raw = Vec::with_capacity(whole);
        for line in region.view().rows() {
            line.iter().for_each(|&sample| sample.put(&mut raw));
            raw.resize(raw.len() + (TILE - rect.width) as usize * bytes, 0);
        }
        raw.resize(whole, 0);
        match self.compression {
            TiffCompression::None => Ok(raw),
            TiffCompression::Deflate => {
                let mut deflate = ZlibEncoder::new(Vec::new(), flate2::Compression::default());
                deflate
                    .write_all(&raw)
                    .and_then(|()| deflate.finish())
                    .map_err(io_at(self.part))
            }
        }
    }

    /// A TIFF error, as a [`StorageError`] of the file.
    fn tiff(&self) -> impl Fn(tiff::TiffError) -> StorageError + '_ {
        |error| StorageError::Tiff {
            path: self.part.to_owned(),
            why: error.to_string(),
        }
    }
}

/// Pads the file to a word boundary, where TIFF has IFDs and their values start. Tiles are of any
/// length, and the encoder pads only when an IFD is begun.
fn align<W: Write + Seek>(ifd: &mut DirectoryEncoder<'_, W, TiffKindBig>) -> tiff::TiffResult<()> {
    if ifd.write_data(&[] as &[u8])? % 2 == 1 {
        ifd.write_data([0u8])?;
    }
    Ok(())
}

/// A level's tiles, and how many there are across.
fn tiles(info: &ImageInfo, level: usize) -> (u64, u64) {
    let size = info.size(level);
    let (rows, cols) = (
        size.y.div_ceil(u64::from(TILE)),
        size.x.div_ceil(u64::from(TILE)),
    );
    (rows * cols, cols)
}

fn sample_bytes(sample: SampleType) -> u64 {
    match sample {
        SampleType::U16 => 2,
        SampleType::F32 => 4,
    }
}

/// A sample as TIFF holds it.
trait Sample: ZarrSample {
    /// Its SampleFormat: 1 unsigned, 3 floating point.
    const FORMAT: u16;
    /// Appends its bytes, in the file's byte order: this machine's, as the encoder writes.
    fn put(self, out: &mut Vec<u8>);
}

impl Sample for u16 {
    const FORMAT: u16 = 1;

    fn put(self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.to_ne_bytes());
    }
}

impl Sample for f32 {
    const FORMAT: u16 = 3;

    fn put(self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.to_ne_bytes());
    }
}

/// The OME-XML (the 2016-06 schema) of `info`, as the export writes it, with what `manifest`
/// says of the dataset: its name, kind and date, and its provenance as a map annotation.
///
/// Planes are listed in the file's order, each with its IFD and its position: that of its first
/// pixel's centre, in µm. Characters beyond ASCII are written as they are: Bio-Formats drops
/// numeric character references.
pub fn ome_xml(info: &ImageInfo, manifest: Option<&Manifest>) -> String {
    let size = info.size;
    let [dz, dy, dx] = info.pixel_size;
    let [z0, y0, x0] = info.origin;
    let schema = "http://www.openmicroscopy.org/Schemas/OME/2016-06";
    let mut xml = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<OME xmlns=\"{schema}\" \
         xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" \
         xsi:schemaLocation=\"{schema} {schema}/ome.xsd\" Creator=\"SpectrumIQ {}\">\n",
        env!("CARGO_PKG_VERSION")
    );
    let _ = writeln!(
        xml,
        "  <Image ID=\"Image:0\" Name=\"{}\">",
        escape(&info.name)
    );
    if let Some(manifest) = manifest {
        let created = manifest.created.to_rfc3339_opts(SecondsFormat::Secs, true);
        let _ = writeln!(xml, "    <AcquisitionDate>{created}</AcquisitionDate>");
        let _ = writeln!(
            xml,
            "    <Description>{} ({} dataset {})</Description>",
            escape(&manifest.name),
            manifest.kind.name(),
            manifest.id
        );
    }
    let kind = match info.sample {
        SampleType::U16 => "uint16",
        SampleType::F32 => "float",
    };
    let _ = writeln!(
        xml,
        "    <Pixels ID=\"Pixels:0\" DimensionOrder=\"XYZCT\" Type=\"{kind}\" SizeX=\"{}\" \
         SizeY=\"{}\" SizeZ=\"{}\" SizeC=\"{}\" SizeT=\"{}\" PhysicalSizeX=\"{dx}\" \
         PhysicalSizeY=\"{dy}\" PhysicalSizeZ=\"{dz}\" BigEndian=\"{}\" Interleaved=\"false\">",
        size.x,
        size.y,
        size.z,
        size.c,
        size.t,
        cfg!(target_endian = "big")
    );
    for (c, channel) in info.channels.iter().enumerate() {
        let [r, g, b] = channel.colour;
        let _ = writeln!(
            xml,
            "      <Channel ID=\"Channel:0:{c}\" Name=\"{}\" SamplesPerPixel=\"1\" Color=\"{}\"/>",
            escape(&channel.label),
            i32::from_be_bytes([r, g, b, 255])
        );
    }
    let planes: Vec<(u64, u64, u64)> = (0..size.t)
        .flat_map(|t| (0..size.c).flat_map(move |c| (0..size.z).map(move |z| (t, c, z))))
        .collect();
    for (ifd, (t, c, z)) in planes.iter().enumerate() {
        let _ = writeln!(
            xml,
            "      <TiffData IFD=\"{ifd}\" FirstZ=\"{z}\" FirstC=\"{c}\" FirstT=\"{t}\" \
             PlaneCount=\"1\"/>"
        );
    }
    for (t, c, z) in planes {
        let _ = writeln!(
            xml,
            "      <Plane TheZ=\"{z}\" TheC=\"{c}\" TheT=\"{t}\" PositionX=\"{x0}\" \
             PositionXUnit=\"µm\" PositionY=\"{y0}\" PositionYUnit=\"µm\" \
             PositionZ=\"{}\" PositionZUnit=\"µm\"/>",
            z0 + z as f64 * dz
        );
    }
    xml.push_str("    </Pixels>\n");
    if let Some(manifest) = manifest {
        let provenance = &manifest.provenance;
        xml.push_str("    <AnnotationRef ID=\"Annotation:0\"/>\n  </Image>\n");
        xml.push_str("  <StructuredAnnotations>\n");
        xml.push_str("    <MapAnnotation ID=\"Annotation:0\" Namespace=\"spectrumiq.dataset\">\n");
        xml.push_str("      <Value>\n");
        for (key, value) in [
            ("dataset", manifest.id.to_string()),
            ("kind", manifest.kind.name().to_owned()),
            ("user", manifest.user.clone()),
            ("software", provenance.software.clone()),
            ("version", provenance.version.clone()),
            ("host", provenance.host.clone()),
        ] {
            let _ = writeln!(xml, "        <M K=\"{key}\">{}</M>", escape(&value));
        }
        xml.push_str("      </Value>\n    </MapAnnotation>\n  </StructuredAnnotations>\n");
    } else {
        xml.push_str("  </Image>\n");
    }
    xml.push_str("</OME>\n");
    xml
}

/// `text` for XML's attributes and text. Control characters become spaces: XML 1.0 forbids most of
/// them, and attributes fold the others.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            ch if ch.is_control() => out.push(' '),
            ch => out.push(ch),
        }
    }
    out
}
