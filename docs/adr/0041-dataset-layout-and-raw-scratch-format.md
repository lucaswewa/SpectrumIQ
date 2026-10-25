# ADR-0041: Dataset layout, and the raw scratch format

- **Status:** Accepted
- **Date:** 2026-10-25
- **Phase:** P4.1
- **Related:** ADR-0005, ADR-0007 (amended), ADR-0021, ADR-0022, REQ-DATA-001, REQ-DATA-002,
  REQ-DATA-006; architecture §4, §8

## Context

- **Every derived product traces back to its frames** (REQ-DATA-001):
  - what the instrument was, and its calibration;
  - the plan that was followed;
  - what processing ran, with which parameters.
- **Only the frames marked to save are recorded** (REQ-DATA-006). They come from the frame pool's
  page-aligned slots through a lossless subscriber (ADR-0022).
  - A 100 MP Mono14 frame is 204 MB, and live runs at 9 frames a second: 111 ms a frame.
  - Writing through the system's cache would copy every frame once more, and evict what the
    viewer and the jobs read.
- **Raw frames are scratch:** later phases turn them into OME-Zarr (P4.2), and Zarr is the master
  format.
- **Windows writes straight from user memory** with `FILE_FLAG_NO_BUFFERING`, if the buffer's
  address and length are whole sectors. A slot starts on a page and is whole pages. A frame's
  pixels needn't be: 11648 × 8742 × 2 bytes isn't.

## Decision

### The layout

```text
<data_root>/datasets/<yyyymmdd-hhmmss>_<slug>_<id8>/
  manifest.json   raw/   zarr/   exports/   qa/
```

- **The folder's name sorts by time, and reads by name.**
  - The time is the UTC creation time.
  - The slug is the name in lowercase letters and digits, with `-` between, at most 40
    characters; the kind if nothing is left.
  - `id8` is the id's last 8 hex digits, the random ones.

### The manifest (`manifest.json`)

- **It is versioned:** `"schema": "spectrumiq.dataset"` and `"schema_version": 1`.
  - A reader refuses another schema, or a newer version.
  - A newer version must read older ones.
- **What made the dataset:**
  - `id` (UUID v7), `kind`, `name`, `created` (UTC), and `user` (the computer's user until M9);
  - `provenance`: the program, its version, the computer.
- **What it was made with:** `instrument` and `calibration`, each a `Snapshot`.
  - A snapshot is the value itself and the SHA-256 of its compact JSON.
  - Datasets with the same hash were made with the same thing; the value says what that was.
- **`plan`:** the acquisition plan (M6), as JSON.
- **`history`:** processing records, oldest first. Each has an id, the kind, the version, the
  parameters, inputs and outputs as paths in the dataset, the state, the times and any error.
  P4.3's jobs append them.
- **It is written atomically:** to `manifest.json.tmp`, flushed to disk, then renamed over the
  manifest. A failure removes the temporary file and leaves the previous manifest as it was.

### The raw scratch format (`raw/`)

- **A frame is two files,** named by its key, `t0000_c00_z000_p00012` (time point, channel id,
  plane, stage position):
  - **`.raw`:** the pixels, as the pool holds them: little-endian, row-major, `u8` or `u16`,
    with no header. They are padded to whole pages with whatever the slot held after the image:
    zeros, or an earlier frame's pixels;
  - **`.json`:** the sidecar: the key, the pixels' length, and the frame's `FrameMeta`.
- **The sidecar is written last,** atomically. A frame without one wasn't finished, and isn't
  read.
- **Pixels are written with `FILE_FLAG_NO_BUFFERING | FILE_FLAG_WRITE_THROUGH`,** straight from the
  slot (`FrameLease::as_padded_bytes`). That's no copy, and nothing left in the system's cache.
- **The recorder (`DatasetWriter`)** runs on a thread of its own, taking a lossless subscriber
  whose filter takes the frames marked to save.
  - A frame's key comes from a function of its metadata: by default, its arrival order, on its
    channel. M6's plans give theirs.
  - It reports frames, bytes and the time spent writing.
- **Free space** (`[server] min_free_gb`, 20 by default):
  - the recorder won't start with less free;
  - it stops, cleanly, before a frame would take the disk below it;
  - a failed write, such as a full disk, stops it too: the partial file is removed, and the frames
    already written are kept.

  Stopping drops its subscriber, so nothing waits on it any more.
- **Reading:** `DatasetReader` lists the frames with a sidecar, in key order, and maps each
  read-only as an `ImageView`. While mapped, the file is open sharing reading only.

### The catalog

- **It indexes a data root in memory:** every folder under `datasets/` with a readable manifest,
  and its frame count, newest first.
- **It learns of a dataset** when told (`record`, which the recorder does when it finishes), or
  on `refresh`. A folder that isn't a dataset is left out, with a warning.

### Unsafe code (ADR-0007, amended)

- **`disk`** calls `GetDiskFreeSpaceExW`: a small Windows-API module, as ADR-0007 allows.
- **`raw::map`** maps a raw file with `memmap2`.
  - Mapping is sound only if nothing changes or truncates the file while it is mapped.
  - The file is opened sharing reading only, and kept open as long as the mapping. No handle can
    then be opened to write, truncate or delete it, and none was open to write it, or the open
    would have failed.
  - The recorder never opens a frame again once it is written.
  - The same module views the mapped bytes as `u16`, when they are aligned and an even number.

## Alternatives considered

- **One file per recording, frames appended:** fewer files, but a frame couldn't be found or
  deleted alone, and a failure part-way through would leave the whole file in doubt.
- **A header in each raw file:** the sidecar already holds the metadata, readable without opening
  the pixels, and a headerless file maps straight to an image.
- **Buffered writes:** one more copy of 204 MB a frame, and the cache churned.
- **Copying each frame into an aligned buffer of its padded size:** the slot already is one.
- **Raw frames in the `scratch` folder:** a second place to look for a dataset's frames. A data
  root on the fast disk does the same, and the setting stays for processing temporaries.
- **Reading frames into memory instead of mapping them:** a 204 MB copy for each look at a frame.

## Consequences

- **A recorded frame is self-describing:** its sidecar carries everything acquisition knew.
- **Recording runs at NVMe speed** with no copy. On this PC's Samsung 990 PRO it wrote 20 frames
  of 11648 × 8742 at 4.3 GB/s: 47 ms a frame, against the 111 ms budget.
- **A raw file is a few KB larger than its pixels**, and its padding may hold an earlier frame's
  pixels. The padding is the instrument's own data, and is never read as image.
- **A mapped frame can't be deleted** until it is unmapped. Deleting datasets (P4.7) must first
  close their readers.

## Verification

- `crates/siq-storage/src/manifest/tests.rs`: the round trip, refusing another format and a newer
  version, the snapshot's hash, and an interrupted write.
- `crates/siq-storage/tests/dataset.rs`: the folder, its name and the manifest's fields, an update
  kept, and the catalog.
- `crates/siq-storage/tests/raw.rs`:
  - the frames marked to save, recorded and mapped back, bytes and metadata;
  - files of whole pages, mapped page-aligned;
  - a full disk stopping the recording, the partial frame removed;
  - the free space kept;
  - the real free space.
- `crates/siq-buffer/tests/pool.rs`: the padded bytes are whole pages from the slot's start.
- **`record_sim`:** 20 frames of the simulated 100 MP camera.

## Amendment (P4.7, 2026-10-25): processing on snap, and the dataset API

**A snap becomes a dataset of kind `snap`,** through `datasets.snap_to_dataset(channels?, name?)`:

1. **The frames are recorded raw,** by the frame recorder on a lossless subscriber of the frames
   marked to save. There is one frame per channel given, each channel applied first (ADR-0035), or
   one in the channel in use. Frame `c` has the key `t0000_c<c>_z000_p00000`. The manifest keeps
   the instrument's configuration as its snapshot.
2. **A job, `zarr_from_raw`,** writes them as level 0 of the OME-Zarr image `zarr/snap.ome.zarr`
   (ADR-0042), a channel each, in the order snapped:
   - **the channels:** named and coloured as their channels, each in its frame's auto-contrast
     window (0.1 % to 99.9 %);
   - **the pixel size:** the sensor's, binned, through the objective's magnification;
   - **the origin:** the stage's position, taken as the frame's centre.

   The pixel size and the origin are approximations until the stage–camera calibration (M5).
3. **The pyramid job** (ADR-0044) is submitted.

The action returns once the pyramid is submitted: the dataset, its image, and the pyramid's job.
The image's tiles are served from then on. The dataset's history records both jobs. Frames must
be of 16-bit pixels.

**The `datasets` Thing:**
- **`summary(offset, limit)`:** a page of the catalog, newest first, at most 500. It is an action,
  since properties take no parameters. It gives each dataset's id, kind, name, date, user, raw
  frames and OME-Zarr images.
- **`export_ome_tiff(dataset, image)`:** `exports/<image>.ome.tif` (ADR-0046).
- **`export_dzi(dataset, image, c, z, t, format)`:** `exports/dzi/<image>_c<c>_z<z>_t<t>.dzi`
  (ADR-0045).

  Both exports run as background jobs, whose ids they give.
- **`delete(dataset)`:** removes the folder and its catalog entry.
  - It is refused while a job is queued or running on the dataset. In the service only jobs map
    raw frames, so no reader is open when it runs (see Consequences).
  - It needs an admin from M9 (architecture §9).
- **The event `created`** carries each new dataset's entry.

Only `snap_to_dataset` takes the global lock.

**Routes:**
- `GET /api/siq/data/{dataset}/manifest`: the manifest, as JSON.
- `GET /api/siq/data/{dataset}/thumb.jpg`: the first image's first plane, its channels
  composited in their colours and windows, at the first level within 256 pixels: the top level,
  once the pyramid is built.

**Measured:** with the simulated 100 MP camera (11 648 × 8742, Mono14), in release:
- the snap was a dataset in 1.05 s;
- its pyramid was built 0.41 s later;
- its first tile, at the level fitting 1920 pixels, came 1.46 s after the snap.

The milestone asks for 10 s.
