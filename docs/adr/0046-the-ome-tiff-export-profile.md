# ADR-0046: The OME-TIFF export profile

- **Status:** Accepted
- **Date:** 2026-10-25
- **Phase:** P4.6
- **Related:** ADR-0042, ADR-0043, ADR-0044, REQ-DATA-004; architecture §8

## Context

- **OME-Zarr is the master format** (ADR-0042), but many tools read only OME-TIFF:
  - Fiji, through Bio-Formats;
  - QuPath;
  - commercial software.
- **Images are large:**
  - 100 MP snaps and mosaics of 500 MP and more, with several channels, beyond TIFF's 4 GB;
  - each with its pyramid (ADR-0044).

  OME-TIFF holds pyramids as SubIFDs (Bio-Formats 6 and later).
- **The `tiff` crate (0.11.3) is already in the tree** (P1.4). Its encoder offers:
  - BigTIFF;
  - images by strips only (`ImageEncoder`), with no tiles and no SubIFDs;
  - a low-level `DirectoryEncoder`: raw data at an offset, any tag, unchained directories for
    SubIFDs, and `IFD8` arrays.
- **An export runs as a job** within the RAM budget (ADR-0043), whatever the image's size.

## Decision

- **We write through the `tiff` crate's low-level `DirectoryEncoder`,** not our own writer.
  - The crate does the file's structure: the header, the entries, the chain and its offsets.
  - We do the tiles and the tags.
- **The file:**
  - **BigTIFF,** in this machine's byte order (little-endian), named `<name>.ome.tif`.
  - **One IFD per plane,** holding level 0, in the order `XYZCT`: `z` fastest, then `c`, then
    `t`.
  - **Levels 1 and up are each plane's SubIFDs,** finest first, marked `NewSubfileType` 1. They
    are written before the plane's IFD, which lists them as `IFD8`. The SubIFDs aren't chained.
- **Tiles** are 512 × 512, padded with zeros at the right and bottom edges, and either:
  - **deflated** (Compression 8, zlib's default level), the default; or
  - **not compressed** (1).

  There's no predictor.
- **Tags:**
  - `BitsPerSample` 16 or 32, and `SampleFormat` 1 (unsigned) or 3 (float);
  - `PhotometricInterpretation` 1, one sample per pixel, `PlanarConfiguration` 1;
  - `TileOffsets` and `TileByteCounts` as `LONG8`.
  - IFDs and their values start on a word boundary.
- **The OME-XML** (the 2016-06 schema) is in the first IFD's `ImageDescription`, in UTF-8:
  - **`Image`:** its name. From the manifest: `AcquisitionDate` (the dataset's creation) and a
    `Description` (the dataset's name, kind and id).
  - **`Pixels`:**
    - `DimensionOrder` `XYZCT`, the type (`uint16` or `float`), the sizes;
    - `PhysicalSize` X, Y and Z in µm, the default unit.
  - **`Channel`:** each with its name, and its colour as a signed RGBA integer, opaque.
  - **`TiffData`:** one per plane, giving its IFD and `FirstZ`, `FirstC`, `FirstT`.
  - **`Plane`:** one per plane, positioned at its first pixel's centre, in µm.
  - **A `MapAnnotation`** in the namespace `spectrumiq.dataset`, from the manifest: the dataset,
    kind, user, software, version and host.
  - **No UUID,** as the export is one file.
  - **No numeric character references:** characters beyond ASCII are written as they are.
    Bio-Formats' sanitiser turns every `&#` into ` #`, so `&#181;m` failed its parsing (see
    Verification).
- **Streaming:**
  - tiles are read from the Zarr levels and encoded in parallel on the job's pool, in batches of
    64, then written in order;
  - the memory held is (64 + 3 × threads) tiles, whatever the image's size (`ometiff::estimate`).
- **The file is written as `<name>.part`,** and renamed once it is whole. A cancelled or failed
  export removes it.
- **The job** (`siq_jobs::kinds::OmeTiffExportJob`, kind `export_ome_tiff`):
  - takes the OME-XML's manifest from its dataset;
  - records the file as its output.

## Alternatives considered

- **A BigTIFF writer of our own:** the brief's fallback if the crate fell short. The crate does
  the header, the entries and the chain correctly. Writing them would add about 150 lines.
- **Strips, through `ImageEncoder`:** no tiles. Readers of large images, and pyramids, need tiles.
- **Levels as further IFDs in the main chain:** Bio-Formats would read them as planes. SubIFDs are
  OME-TIFF's pyramid.
- **Other compressions:**
  - LZW: slower, and larger than deflate for 16-bit data;
  - zstd: not read by every TIFF reader;
  - JPEG: lossy.

  Deflate is read by libtiff, Bio-Formats and tifffile.
- **The horizontal predictor:** smaller files for smooth images. Left for later, as an option.
- **ASCII with numeric character references,** to keep TIFF's text 7-bit: Bio-Formats can't read
  them (above). UTF-8 is what OME-TIFF writers, Bio-Formats among them, put there.

## Consequences

- **Fiji opens the export as OME-TIFF** (Bio-Formats 8.1.1 and 8.5.0):
  - every resolution;
  - the channels, with their names and colours;
  - the pixel size in µm;
  - the dataset's annotation.
- **QuPath opens it too** (0.7.0, through Bio-Formats): its resolutions as downsamples 1 to 64,
  the channels' colours, and the pixel size.
- **A reduced resolution opened on its own** (Fiji's series 2 and up) has default colours and no
  pixel size: Bio-Formats gives sub-resolutions no OME metadata of their own.
- **Strict TIFF readers see bytes beyond 7 bits** in an `ImageDescription` typed `ASCII`, as in
  every OME-TIFF with non-ASCII text.
- **The OME-XML's `Creator` carries the version,** so its snapshot changes with each release.

## Verification

- `crates/siq-storage/tests/ometiff.rs`, with a structural reader of our own:
  - the BigTIFF header;
  - the IFD chain in `XYZCT` order (2 × 2 × 2 planes);
  - every tag, and its values on word boundaries;
  - each plane's two SubIFDs (as `IFD8`, unchained);
  - every tile of every level, inflated, with its pixels and padding;
  - the OME-XML in the first IFD only.

  Also:
  - the `tiff` crate decodes level 0 of every plane;
  - a `float` image with one level and no compression;
  - the OME-XML snapshot, with no `&#` in it;
  - a cancelled export leaves no file.
- `crates/siq-jobs/tests/kinds.rs`: the job through the scheduler, decoded by the `tiff` crate,
  with its SubIFDs and the dataset in its OME-XML, and recorded.
- **tifffile 2026.9.20:** the `export_ometiff` file (12 000 × 8000, two channels, seven levels).
  - Every level equals the OME-Zarr's.
  - ome-types validates the OME-XML against the schema.
- **Fiji,** headless: the stable build (March 2026, Bio-Formats 8.1.1) and the latest (October
  2026, Bio-Formats 8.5.0), alike.
  - **`OMETiffReader`** takes the file: one series, seven resolutions, tiles of 512, the channels,
    the pixel size, the annotation, and the right pixels at resolutions 0 and 3.
  - **The importer's code path** (`BF.openImagePlus`) opens a 16-bit composite image,
    12 000 × 8000 × 2 at 0.376 µm, green and magenta.
  - **The first attempt** used `&#181;m` for the planes' units. Bio-Formats read the file as
    plain TIFF, and the reason was its sanitiser (above).
- **QuPath 0.7.0,** in its console mode: a Bio-Formats server, 12 000 × 8000, two `UINT16`
  channels in green and magenta, seven resolutions, 0.376 µm, the right pixels, and the whole
  image read at downsample 8.
