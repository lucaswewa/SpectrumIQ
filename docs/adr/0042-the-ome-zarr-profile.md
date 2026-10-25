# ADR-0042: The OME-Zarr profile

- **Status:** Accepted
- **Date:** 2026-10-25
- **Phase:** P4.2
- **Related:** ADR-0007, ADR-0041, REQ-DATA-003; architecture §8.1

## Context

- **OME-Zarr is the master format** of everything derived from the frames: stitched mosaics,
  depth maps, volumes (architecture §8). Pyramids (P4.4), the tile service (P4.5) and exports
  (P4.6) read it.
- **The ecosystem must open it:** napari, through `napari-ome-zarr`, and Fiji.
- **NGFF has two current versions:**
  - **0.4** on Zarr v2, which everything reads;
  - **0.5** on Zarr v3, which the recent readers support. The brief picks 0.5 unless napari or
    Fiji fails on it.
- **The writers are parallel:** stitching and pyramid jobs write many chunks at once, in an order
  of their own.
- **`zarrs`** is the Rust implementation of Zarr v3, with zstd (whose C code ADR-0007 already
  allows).

## Decision

- **NGFF 0.5, on Zarr v3,** through `zarrs` 0.23, with only its `filesystem` and `zstd`
  features. napari read it (see Verification).
- **An image is a group,** such as `zarr/mosaic.ome.zarr/`, with one array per resolution level,
  `0`, `1`, ….
  - Level `l` halves level `l − 1` in `y` and `x`, rounding up, so its pixel is 2ˡ of level 0's.
  - `t`, `c` and `z` are never downsampled.
- **The axes are `tczyx`,** always all five:
  - `t` (time, in seconds) and `c` (channel);
  - `z`, `y`, `x` (space, in micrometres).

  The arrays' `dimension_names` say the same.
- **Pixels:** `uint16` for frames and mosaics, `float32` for processed results. The fill value is
  0.
- **Chunks** are 1 × 1 × 1 × 512 × 512 by default, the size of a deep-zoom tile's
  neighbourhood.
  - The chunk key encoding is Zarr v3's default, with `/`.
  - No sharding for now: a 100 MP plane is 400 chunks, which file systems handle.
- **Codecs:** `bytes` (little-endian), then `zstd` at level 3, without its checksum. Level 3 writes
  a 151 MB image at 700 MB/s on this PC's 16 threads, 4.3× smaller.
- **The group's `zarr.json`** holds NGFF's metadata under `ome`:
  - `version`: `"0.5"`;
  - `multiscales[0]`:
    - `name`;
    - the five `axes`, with their types and units;
    - each level's `datasets` entry, with a `scale` (level 0's pixel size × 2ˡ in `y` and `x`)
      and a `translation`;
    - `type: "mean"`.
  - **`omero`:** each channel's `label`, `color` (hex RGB), and `window` (`start` and `end`
    shown, `min` and `max` the type's range), all `active`, and `rdefs.model: "color"`.
- **A level's translation centres its pixels** on the level-0 pixels they cover: level 0's origin
  plus (2ˡ − 1) / 2 of level 0's pixel. Readers that use only level 0's transform lose nothing.
- **Nothing else is written:** `zarrs`' own `_zarrs` attribute is left out.
- **The API (`siq_storage::zarr`):**
  - `ZarrImage::create(path, ImageInfo)` writes the metadata and an empty array per level;
  - `open(path)` reads it back, refusing another NGFF version or other axes;
  - `write_chunk(level, (t, c, z), (row, col), view)` stores one chunk:
    - from any thread, in any order;
    - at the right and bottom edges, given clipped, and padded with zeros on disk;
  - `read_region(level, (t, c, z), rect)` reads any rectangle of a level, refusing one outside it.

## Alternatives considered

- **NGFF 0.4 (Zarr v2):** read by older tools, but superseded. It is the fallback if a reader we
  need can't open 0.5.
- **Sharding** (many chunks per file): fewer files for very large mosaics. It can come later
  behind the same API, since readers that support v3 support sharding.
- **Blosc or LZ4:** zstd alone, at a low level, is fast enough, and needs one C library, not two.
- **Chunks of 256 or 1024:** 256 means four times the files; 1024 means a tile read decodes 4 MB.
- **`ndarray`:** `zarrs` offers it, but `ImageView`s and `Vec`s are what the rest of the code
  uses.

## Consequences

- **napari opens the images** with its OME-Zarr reader:
  - a layer per channel, named and coloured from `omero`, with its window as contrast;
  - the physical pixel size.
- **Fiji's reading of NGFF 0.5 isn't checked yet:** Fiji isn't installed on this PC. P4.6 installs
  it for the OME-TIFF check, and tries the OME-Zarr then: see the amendment.
- **`zarrs` lists `libz-sys` as a build-dependency,** which its build script doesn't use. zlib is
  compiled for it, and linked into nothing we build.
- **A level's array is made empty;** P4.4's pyramid builder fills levels 1 and up.

## Verification

- `crates/siq-storage/tests/zarr.rs`:
  - an odd-sized image (1000 × 777, chunks of 256) read back whole, across chunks, and in a
    partial corner chunk;
  - edge chunks given clipped, a whole one refused there, zeros where nothing was written, and
    regions outside refused;
  - `float32` with four levels, halved rounding up;
  - the NGFF metadata, snapshotted (`snapshots/zarr__group.snap`, `zarr__level_1.snap`);
  - 64 disjoint chunks written at once by rayon, in reverse order.
- **`zarr_write_read`:** 3 channels of 6144 × 4096 written and a region read back, with no pixel
  wrong.
- **napari 0.9.2, napari-ome-zarr 0.11.0** (ome-zarr 0.21.0, zarr 3.4.1). This PC's service
  session has no OpenGL, so the reader napari calls for an `.ome.zarr` was run headless, and napari
  layers were made from its output:
  - three layers, `synthetic: rings`, `synthetic: gradient` and `synthetic: checkers`;
  - each of shape (1, 1, 4096, 6144), `uint16`, at 0.376 µm a pixel;
  - colourmaps magenta, green and the custom blue, with contrast 0–16000;
  - pixels as written.

## Amendment (P4.6, 2026-10-25): Fiji reads NGFF 0.5 with `n5-zarr` 2

P4.6 installed Fiji, for the OME-TIFF check, and tried this profile's images in it, through the
N5 libraries behind File › Import › HDF5/N5/Zarr/OME-NGFF. Fiji was run headless, as Groovy
scripts, since this PC's service session has no display.

- **The stable build** (March 2026, `n5-zarr` 1.5.1) **doesn't read them:** it knows Zarr v2
  only, and finds no container at an image's folder.
- **The latest build** (23 October 2026, `n5-zarr` 2.0.1, `n5-universe` 3.0.2) **reads them**
  (`export_ometiff`'s image, 12 000 × 8000 × 2, seven levels):
  - through `ZarrV3KeyValueReader`;
  - its metadata discovered as OME-NGFF, with the seven scales;
  - level 0 as a 16-bit ImagePlus;
  - the right pixels.

**The decision stands:** NGFF 0.5. A Fiji without `n5-zarr` 2, such as the March 2026 stable
build, needs updating to read the images, or opens their OME-TIFF export (ADR-0046), which both
builds read.
