//! OME-TIFF export (ADR-0046), checked by a structural reader of our own: the BigTIFF header, the
//! IFD chain in `XYZCT` order, each plane's SubIFDs, every tile's pixels and padding, and the
//! OME-XML. The `tiff` crate decodes level 0. REQ-DATA-004.

use std::collections::BTreeMap;
use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};

use siq_storage::ometiff::{TILE, TiffCompression, export, ome_xml};
use siq_storage::zarr::*;
use siq_storage::{Control, Dataset, DatasetKind, NewDataset, StorageError};
use siq_types::OwnedImage;

fn info(size: Size5, sample: SampleType, levels: usize) -> ImageInfo {
    let channels = (0..size.c)
        .map(|c| ChannelInfo {
            label: ["DAPI", "GFP"][c as usize % 2].to_owned(),
            colour: [[0, 0, 255], [0, 255, 0]][c as usize % 2],
            window: (100.0, 16000.0),
        })
        .collect();
    ImageInfo {
        name: "kidney – section 3 <A&B>".to_owned(),
        sample,
        size,
        levels,
        chunk: 256,
        pixel_size: [2.0, 0.376, 0.376],
        origin: [10.0, -500.0, 250.0],
        channels,
        cfa: None,
    }
}

/// Plane (`t`, `c`, `z`)'s pixel at (`x`, `y`) of `level`: different everywhere.
fn value(level: usize, (t, c, z): (u64, u64, u64), x: u64, y: u64) -> u16 {
    ((level as u64 * 40_009 + t * 3001 + c * 7919 + z * 104_729 + y * 1000 + x) % 65_521) as u16
}

/// An image with every chunk of every level written, as `value` gives them, cast to `T`.
fn image<T: ZarrSample>(path: &std::path::Path, info: ImageInfo, cast: fn(u16) -> T) -> ZarrImage {
    let image = ZarrImage::create(path, info.clone()).unwrap();
    for level in 0..info.levels {
        let size = info.size(level);
        let (rows, cols) = info.chunks(level);
        for plane in planes(size) {
            for (row, col) in (0..rows).flat_map(|row| (0..cols).map(move |col| (row, col))) {
                let (y0, x0) = (row * info.chunk, col * info.chunk);
                let height = info.chunk.min(size.y - y0) as usize;
                let width = info.chunk.min(size.x - x0) as usize;
                let chunk = OwnedImage::from_fn(width, height, |x, y| {
                    cast(value(level, plane, x0 + x as u64, y0 + y as u64))
                });
                image
                    .write_chunk(level, plane, (row, col), chunk.view())
                    .unwrap();
            }
        }
    }
    image
}

/// The planes in `XYZCT` order: `z` fastest, then `c`, then `t`.
fn planes(size: Size5) -> Vec<(u64, u64, u64)> {
    (0..size.t)
        .flat_map(|t| (0..size.c).flat_map(move |c| (0..size.z).map(move |z| (t, c, z))))
        .collect()
}

/// An IFD as our reader sees it: each entry's type and values, the text of ASCII ones (UTF-8, as
/// OME-TIFF has it), and the next IFD's offset.
#[derive(Debug)]
struct Ifd {
    entries: BTreeMap<u16, (u16, Vec<u64>)>,
    text: BTreeMap<u16, String>,
    next: u64,
}

impl Ifd {
    fn one(&self, tag: u16) -> u64 {
        let (_, values) = &self.entries[&tag];
        assert_eq!(values.len(), 1, "tag {tag}");
        values[0]
    }
}

fn u16_at(file: &[u8], at: usize) -> u16 {
    u16::from_le_bytes(file[at..at + 2].try_into().unwrap())
}

fn u64_at(file: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(file[at..at + 8].try_into().unwrap())
}

/// Reads the BigTIFF IFD at `offset`: 8 bytes of count, entries of 20 bytes in ascending tag
/// order, then the next IFD's offset. Values of 8 bytes or less are in the entry.
fn read_ifd(file: &[u8], offset: u64) -> Ifd {
    let at = offset as usize;
    assert_eq!(at % 2, 0, "IFDs start on a word boundary");
    let count = u64_at(file, at) as usize;
    let (mut entries, mut text) = (BTreeMap::new(), BTreeMap::new());
    let mut previous = 0;
    for entry in (0..count).map(|i| at + 8 + i * 20) {
        let (tag, kind, n) = (
            u16_at(file, entry),
            u16_at(file, entry + 2),
            u64_at(file, entry + 4),
        );
        assert!(tag > previous, "tags ascend: {tag} after {previous}");
        previous = tag;
        let width = match kind {
            1 | 2 => 1,
            3 => 2,
            4 => 4,
            16 | 18 => 8,
            other => panic!("tag {tag} has the unexpected type {other}"),
        };
        let size = width * n as usize;
        let data = if size <= 8 {
            &file[entry + 12..entry + 12 + size]
        } else {
            let start = u64_at(file, entry + 12) as usize;
            assert_eq!(start % 2, 0, "tag {tag}'s values start on a word boundary");
            &file[start..start + size]
        };
        if kind == 2 {
            assert_eq!(data.last(), Some(&0), "ASCII ends with NUL");
            let utf8 = std::str::from_utf8(&data[..size - 1]).unwrap();
            text.insert(tag, utf8.to_owned());
        }
        let values = data
            .chunks(width)
            .map(|value| match width {
                1 => u64::from(value[0]),
                2 => u64::from(u16_at(value, 0)),
                4 => u64::from(u32::from_le_bytes(value.try_into().unwrap())),
                _ => u64_at(value, 0),
            })
            .collect();
        entries.insert(tag, (kind, values));
    }
    Ifd {
        entries,
        text,
        next: u64_at(file, at + 8 + count * 20),
    }
}

/// Checks that `ifd` holds `plane`'s `level` of `info`: its tags, and every tile, decompressed if
/// `deflated`, with the pixels of `value` cast to `T`'s bytes and zeros beyond the edges.
fn check_level<T: Copy>(
    file: &[u8],
    ifd: &Ifd,
    info: &ImageInfo,
    plane: (u64, u64, u64),
    level: usize,
    (deflated, format, cast): (bool, u64, fn(u16) -> T),
    bytes: fn(T) -> Vec<u8>,
) {
    let size = info.size(level);
    let sample = size_of::<T>();
    assert_eq!(ifd.one(254), u64::from(level > 0), "NewSubfileType");
    assert_eq!((ifd.one(256), ifd.one(257)), (size.x, size.y));
    assert_eq!(ifd.one(258), 8 * sample as u64, "BitsPerSample");
    assert_eq!(ifd.one(259), if deflated { 8 } else { 1 }, "Compression");
    assert_eq!((ifd.one(262), ifd.one(277), ifd.one(284)), (1, 1, 1));
    assert_eq!(
        (ifd.one(322), ifd.one(323)),
        (512, 512),
        "TileWidth, TileLength"
    );
    assert_eq!(ifd.one(339), format, "SampleFormat");
    let (cols, rows) = (size.x.div_ceil(512), size.y.div_ceil(512));
    let (offsets, counts) = (&ifd.entries[&324], &ifd.entries[&325]);
    assert_eq!((offsets.0, counts.0), (16, 16), "LONG8");
    assert_eq!(offsets.1.len() as u64, rows * cols);
    assert_eq!(counts.1.len() as u64, rows * cols);
    let tile = TILE as usize;
    for (k, (&offset, &count)) in offsets.1.iter().zip(&counts.1).enumerate() {
        let stored = &file[offset as usize..(offset + count) as usize];
        let raw = if deflated {
            let mut raw = Vec::new();
            flate2::read::ZlibDecoder::new(stored)
                .read_to_end(&mut raw)
                .unwrap();
            raw
        } else {
            stored.to_vec()
        };
        assert_eq!(raw.len(), tile * tile * sample, "a whole tile");
        let (x0, y0) = ((k as u64 % cols) * 512, (k as u64 / cols) * 512);
        for (y, line) in raw.chunks(tile * sample).enumerate() {
            for (x, pixel) in line.chunks(sample).enumerate() {
                let (x, y) = (x0 + x as u64, y0 + y as u64);
                let expected = if x < size.x && y < size.y {
                    bytes(cast(value(level, plane, x, y)))
                } else {
                    vec![0; sample]
                };
                assert_eq!(pixel, expected, "level {level} of {plane:?} at ({x}, {y})");
            }
        }
    }
}

#[test]
fn every_plane_and_level_is_in_the_file_as_tiles_with_its_subifds() {
    let folder = tempfile::tempdir().unwrap();
    let size = Size5 {
        t: 2,
        c: 2,
        z: 2,
        y: 1100,
        x: 1300,
    };
    let info = info(size, SampleType::U16, 3);
    let image = image(&folder.path().join("in.ome.zarr"), info.clone(), |v| v);
    let out = folder.path().join("out.ome.tif");
    let dataset =
        Dataset::create(folder.path(), NewDataset::new(DatasetKind::Snap, "kidney")).unwrap();
    let manifest = dataset.manifest();
    export(&image, Some(manifest), &out, TiffCompression::Deflate, &()).unwrap();
    let file = std::fs::read(&out).unwrap();

    // BigTIFF: little-endian, version 43, offsets of 8 bytes.
    assert_eq!(&file[..8], b"II\x2b\x00\x08\x00\x00\x00");
    let mut next = u64_at(&file, 8);
    let mut found = Vec::new();
    while next != 0 {
        let ifd = read_ifd(&file, next);
        found.push(ifd.next);
        let plane = planes(size)[found.len() - 1];
        let first = found.len() == 1;
        assert_eq!(
            ifd.text.get(&270).cloned(),
            first.then(|| ome_xml(&info, Some(manifest))),
            "the OME-XML, in the first IFD only"
        );
        let to_bytes = |v: u16| v.to_le_bytes().to_vec();
        check_level(&file, &ifd, &info, plane, 0, (true, 1, |v| v), to_bytes);
        let (kind, subifds) = &ifd.entries[&330];
        assert_eq!((*kind, subifds.len()), (18, 2), "two SubIFDs, as IFD8");
        for (level, &subifd) in (1..).zip(subifds) {
            let sub = read_ifd(&file, subifd);
            assert_eq!(sub.next, 0, "SubIFDs aren't chained");
            assert!(!sub.entries.contains_key(&330) && sub.text.is_empty());
            check_level(&file, &sub, &info, plane, level, (true, 1, |v| v), to_bytes);
        }
        next = ifd.next;
    }
    assert_eq!(found.len(), 8, "an IFD per plane");
    assert!(!folder.path().join("out.ome.tif.part").exists());

    // The tiff crate reads level 0 of every plane, in order.
    let mut decoder = tiff::decoder::Decoder::new(std::fs::File::open(&out).unwrap()).unwrap();
    for (index, plane) in planes(size).into_iter().enumerate() {
        if index > 0 {
            decoder.next_image().unwrap();
        }
        assert_eq!(decoder.dimensions().unwrap(), (1300, 1100));
        let tiff::decoder::DecodingResult::U16(pixels) = decoder.read_image().unwrap() else {
            panic!("not 16-bit");
        };
        for (k, &pixel) in pixels.iter().enumerate().step_by(997) {
            let (x, y) = (k as u64 % 1300, k as u64 / 1300);
            assert_eq!(pixel, value(0, plane, x, y), "{plane:?} at ({x}, {y})");
        }
    }
    assert!(!decoder.more_images());
}

#[test]
fn a_float_image_of_one_level_is_written_uncompressed() {
    let folder = tempfile::tempdir().unwrap();
    let size = Size5 {
        t: 1,
        c: 1,
        z: 1,
        y: 300,
        x: 700,
    };
    let info = info(size, SampleType::F32, 1);
    let cast: fn(u16) -> f32 = |v| f32::from(v) / 7.0;
    let image = image(&folder.path().join("in.ome.zarr"), info.clone(), cast);
    let out = folder.path().join("out.ome.tif");
    export(&image, None, &out, TiffCompression::None, &()).unwrap();
    let file = std::fs::read(&out).unwrap();

    let ifd = read_ifd(&file, u64_at(&file, 8));
    assert_eq!(ifd.next, 0, "one plane");
    assert!(!ifd.entries.contains_key(&330), "no SubIFDs");
    let to_bytes = |v: f32| v.to_le_bytes().to_vec();
    check_level(&file, &ifd, &info, (0, 0, 0), 0, (false, 3, cast), to_bytes);
    let xml = &ifd.text[&270];
    assert!(xml.contains(r#"Type="float""#) && !xml.contains("StructuredAnnotations"));
}

#[test]
fn the_ome_xml_describes_the_image_and_its_dataset() {
    let folder = tempfile::tempdir().unwrap();
    let dataset =
        Dataset::create(folder.path(), NewDataset::new(DatasetKind::Snap, "kidney")).unwrap();
    let mut manifest = dataset.manifest().clone();
    manifest.id = serde_json::from_str(r#""0199a3c2-7d1e-7b40-9a55-3f2e8c1d0b6a""#).unwrap();
    manifest.created = "2026-10-25T09:30:00Z".parse().unwrap();
    manifest.user = "operator".to_owned();
    manifest.provenance.version = "0.1.0".to_owned();
    manifest.provenance.host = "scope-1".to_owned();
    let size = Size5 {
        t: 1,
        c: 2,
        z: 2,
        y: 1100,
        x: 1300,
    };
    let xml = ome_xml(&info(size, SampleType::U16, 3), Some(&manifest));
    assert!(
        !xml.contains("&#"),
        "Bio-Formats drops numeric character references"
    );
    insta::assert_snapshot!(xml);
}

/// Cancels at the first progress it is told of.
struct CancelAtFirst(AtomicBool);

impl Control for CancelAtFirst {
    fn progress(&self, _: f64) {
        self.0.store(true, Ordering::Relaxed);
    }

    fn cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

#[test]
fn a_cancelled_export_leaves_no_file() {
    let folder = tempfile::tempdir().unwrap();
    let size = Size5 {
        t: 1,
        c: 1,
        z: 1,
        y: 6000,
        x: 6000,
    };
    let image = image(
        &folder.path().join("in.ome.zarr"),
        info(size, SampleType::U16, 1),
        |v| v,
    );
    let out = folder.path().join("out.ome.tif");
    let control = CancelAtFirst(AtomicBool::new(false));
    let result = export(&image, None, &out, TiffCompression::Deflate, &control);
    assert!(matches!(result, Err(StorageError::Cancelled)), "{result:?}");
    assert!(!out.exists());
    assert!(!folder.path().join("out.ome.tif.part").exists());
}
