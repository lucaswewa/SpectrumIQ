# siq-storage

Datasets on disk (ADR-0041): the folder layout of architecture §8.1 and its manifest, with full
provenance; the raw frame recorder, writing the frames marked to save with direct I/O straight
from the frame pool; the frames read back, memory-mapped; the catalog of a data root; and OME-Zarr
images (ADR-0042), the master format of everything derived, with their OME-TIFF export
(ADR-0046) for Fiji, QuPath and other tools.

## Place in the architecture

- **Layer:** L1 Core.
- **Depends on:** `siq-types`, `siq-buffer`; `zarrs` (with zstd), `tiff` (its low-level BigTIFF
  encoder), `flate2`, `rayon`, `memmap2`, `sha2`, `serde_json`, `chrono`, `windows-sys`.
- **Used by:** the service (P4.7), `siq-jobs` (P4.3), `siq-datagen` (P4.9).
- **Phase:** P4.1; P4.2 added `zarr`, P4.6 `ometiff`, P4.7 `write_plane` and `Catalog::delete`, and
  P4.9 `write_frame`.

## API

| Type | What |
|---|---|
| `Dataset` | A dataset's folder: `create(data_root, NewDataset)` and `open(path)`; `raw_dir`, `zarr_dir`, `exports_dir`, `qa_dir`; `update(…)` writes the manifest atomically |
| `Manifest` | `schema` and `schema_version`, id, kind (`snap`, `recording`, `tile_scan`, `z_stack`, `sequence` for any other plan's, `synthetic`, `mosaic` for a live mosaic saved), name, created, user, provenance, the instrument and calibration `Snapshot`s with their SHA-256, the plan, and the processing history: each step's kind, version, parameters, outputs, state and `results`. `read` refuses another format or a newer version; `write` is atomic |
| `DatasetWriter` | `start(dataset, subscriber, WriterOptions)` records every frame of a lossless subscriber on a thread of its own; `stats`, `is_finished`, `finish` → `WriterReport` |
| `WriterOptions` | `min_free` (the bytes to keep free), each frame's `FrameKey`, the `FrameWrite` (`DirectIo`), the free-space query, and a `Catalog` to update |
| `DatasetReader` | `open(dataset)` lists the frames with a sidecar, in key order; `RawFrame::map` → `MappedFrame`, with `image_u16`, `image_u8`, `as_bytes` and `meta` |
| `write_frame(dataset, key, meta, pixels)` | A frame made without the camera, such as `siq-datagen`'s, written as the recorder writes one: its raw file, then its sidecar |
| `Catalog` | `scan(data_root)`, `refresh`, `record(path)`, `list` (newest first), `get(id)`, `delete(id)` (the folder, then the entry) |
| `free_space(path)` | The bytes this process may still write on a path's disk |
| `zarr::ZarrImage` | An OME-Zarr image, NGFF 0.5: `create(path, ImageInfo)` and `open(path)`; `set_levels(n)` adds levels, for the pyramid (P4.4); `write_chunk(level, (t, c, z), (row, col), view)` from any thread, edge chunks clipped; `write_plane(level, (t, c, z), view)`, its chunks in parallel; `read_region(level, (t, c, z), rect)` → `Region` |
| `zarr::ImageInfo` | The name, `SampleType` (`U16`, `F32`), level 0's `Size5`, the levels, the chunk, the pixel size and origin in µm, and the `ChannelInfo`s (label, colour, window); `size(level)` and `chunks(level)` |
| `ometiff::export(image, manifest, out, compression, control)` | The image as one OME-TIFF file: BigTIFF, an IFD per plane in `XYZCT` order with its other levels as SubIFDs, tiles of 512, and the OME-XML. Tiles are encoded in parallel on the calling rayon pool, in batches; the file is written as `<out>.part` and renamed once whole |
| `ometiff::ome_xml(info, manifest)` | The OME-XML (2016-06) written: the image, its channels and planes, and the manifest's date, description and provenance |
| `ometiff::TiffCompression` | `Deflate` (the default) or `None` |
| `ometiff::estimate(info, threads)` | The most memory an export holds |
| `Control` | Where long work reports its progress, and learns whether to stop: an export's, and the pyramid builder's (`siq-pyramid` re-exports it) |

A frame's files are `raw/t0000_c00_z000_p00012.raw` (its pixels, little-endian, padded to whole
pages) and `.json` (its `Sidecar`: key, length and `FrameMeta`), written after the pixels.

**Unsafe code** (ADR-0007): `disk` calls `GetDiskFreeSpaceExW`, and `raw::map` maps a frame
read-only (ADR-0041 gives the argument). Each block has its `SAFETY` comment.

## Examples

| Example | What it shows | Run |
|---|---|---|
| `record_sim` | 20 frames of the simulated 100 MP camera recorded through the acquisition engine, the throughput, and the frames read back | `cargo run --release -p siq-storage --example record_sim` |
| `zarr_write_read` | A synthetic 3-channel OME-Zarr image written in parallel, and a region read back; `--out <folder>` keeps it, to open in napari | `cargo run --release -p siq-storage --example zarr_write_read` |

Every example also runs in smoke mode (`SIQ_SMOKE=1`) in `cargo xtask ci`.

## Features

None.

## Testing

`cargo test -p siq-storage`:

- the manifest: its round trip, refusing another format or a newer version, the snapshot's hash,
  and an interrupted write leaving the previous manifest intact;
- the dataset's folder, name and manifest, and an update kept;
- recording: pixels and metadata round trip, files of whole pages mapped page-aligned, a full disk
  stopping the recording cleanly, and the free space kept;
- the catalog: newest first, frames counted, broken folders left out, updates and refresh;
- OME-Zarr: odd sizes read back anywhere, edge chunks, `f32` levels, the NGFF metadata
  (snapshots), and disjoint chunks written from many threads;
- OME-TIFF, through a structural reader of our own: the header, the IFDs in `XYZCT` order, each
  plane's SubIFDs, every tile's pixels and padding, the OME-XML (snapshot), level 0 decoded by the
  `tiff` crate, `f32` uncompressed, and a cancelled export leaving no file.

## Limitations

- Raw frames go in the dataset's folder, on the data root's disk. The `scratch` setting isn't
  used yet.
- Windows only, like the rest of the service: the direct-I/O flags and the free-space query are
  Windows's.
- The catalog learns of new datasets when told (`record`), or on `refresh`; it doesn't watch the
  folder.
- An OME-TIFF export holds one image, at the levels it has. There's no predictor, and no other
  compression than deflate.
