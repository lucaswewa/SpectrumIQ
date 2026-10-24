# M4 — Storage, jobs and the 2D deep-zoom viewer

**Outcome:** datasets on disk with full provenance. The pipeline from raw to OME-Zarr to pyramid
runs as background jobs. The browser and desktop app inspect any dataset in a deep-zoom viewer with
per-channel LUTs. OME-TIFF and DZI exports exist, and so does a generator for synthetic datasets
with ground truth.

**Exit criteria:**

- a snapped 100 MP frame is viewable in deep zoom within 10 s;
- a 500 MP synthetic mosaic from `siq-datagen` is browsable at interactive speed;
- an OME-TIFF export opens in Fiji;
- an OME-Zarr opens in napari. The manual checks are recorded in the impl notes.

---

### P4.1 — Datasets and the raw scratch writer (`siq-storage::dataset`, `::raw`)

**Budget** ~800 · **Depends on** P2.1 · **Requirements** REQ-DATA-001, REQ-DATA-002, REQ-DATA-006

**Goal.** A dataset layout with full provenance, and zero-copy recording of selected frames.

**Scope**

- **Layout** per architecture §8.1.
- **`Manifest`:**
  - schema-versioned;
  - id, kind, created, user;
  - instrument and calibration snapshots with hashes;
  - plan;
  - processing history;
  - written atomically (temp file + rename).
- **`DatasetWriter`:**
  - a writer thread consuming a `Lossless` subscriber filtered on `flags.save`;
  - each frame is written with `FILE_FLAG_NO_BUFFERING | FILE_FLAG_WRITE_THROUGH`
    (`OpenOptionsExt::custom_flags`) directly from the page-aligned lease, followed by a JSON
    sidecar.
- **Disk-space guard:** refuses to start below the configured free space and stops cleanly when the
  disk fills.
- **`DatasetReader`:** lists frames and memory-maps them (`memmap2`) as `ImageView<u16>`.
- **`Catalog`:** scans `data_root` into an in-memory index, refreshed when datasets are written.

**Tests**

- Round trip (bytes + metadata).
- Alignment.
- An interrupted manifest write leaves the previous manifest intact.
- The disk-full path, through an injected writer error.
- Catalog listing.

**Example.** `record_sim`: records 20 frames from SimCamera and prints the throughput in GB/s.

**ADR.** Dataset layout and raw scratch format.

---

### P4.2 — OME-Zarr I/O (`siq-storage::zarr`)

**Budget** ~850 · **Depends on** P4.1 · **Requirements** REQ-DATA-003

**Goal.** The master format, readable by the ecosystem.

**Scope**

- **`zarrs`-based multiscale arrays:** `tczyx`; u16 and f32.
- **Chunked writer** (parallel, caller-driven chunk order) and **region reader** at any level.
- **NGFF metadata:** `multiscales`, axes with µm units, `coordinateTransformations` (scale +
  translation), and `omero` channels (colour, window).
- **Codec:** zstd.
- The ADR picks NGFF 0.5 (Zarr v3) unless the napari/Fiji check fails; the fallback is 0.4
  (Zarr v2).

**Tests**

- Round trip on odd sizes.
- Partial chunks.
- Metadata snapshot.
- Concurrent writers to disjoint chunks.

**Example.** `zarr_write_read`: writes a synthetic 3-channel image and reads a region back.

**ADR.** The OME-Zarr profile: version, chunking, compression, axes, and the metadata subset.

---

### P4.3 — Job system (`siq-jobs`, `jobs` Thing)

**Budget** ~850 · **Depends on** P0.3, P4.1 · **Requirements** REQ-3D-005, REQ-NFR-002

**Goal.** Run heavy processing in the background without disturbing acquisition, within the
memory budget.

**Scope**

- **`Job` trait:** `estimate() -> Resources { ram, vram, gpu, threads }` and
  `run(&JobCtx) -> Result<JobOutput>`. `JobCtx` provides `progress`, `cancelled`, `log`, and
  `dataset`.
- **`Scheduler`:**
  - priority queue;
  - admission against the RAM and VRAM budgets;
  - FIFO within a priority;
  - starvation guard;
  - a GPU lane semaphore (P7.1 replaces it with the GPU thread).
- **Pool:** a dedicated rayon pool whose start handler sets the threads to below-normal priority
  (`windows-sys`).
- **Job records** (id, kind, params, state, progress, timings, error) are appended to the dataset
  manifest's processing history.
- **`jobs` Thing** in `siq-server`: `queue` and `running` properties (observed); `cancel` action.

**Tests**

- Admission.
- Priority and starvation.
- Cancellation.
- Persistence.
- Behaviour when an estimate is exceeded (the job is logged and allowed, but counted).

**Example.** `jobs_demo`: runs synthetic jobs and shows their admission.

**ADR.** Job scheduling and memory admission.

---

### P4.4 — Pyramid builder (`siq-pyramid::build`)

**Budget** ~600 · **Depends on** P4.2, P4.3 · **Requirements** REQ-DATA-003, REQ-DATA-005, REQ-STI-003

**Goal.** Multiscale levels from level 0, out-of-core.

**Scope**

- Levels are built by 2×2 mean (u16 with rounding; f32) down to ≤ 256 px.
- **Streaming over chunk rows:** memory is bounded at about 3 chunk rows per level. Work is
  parallel across chunks.
- Runs as a `Job` with an estimate, and updates the `multiscales` metadata.

**Tests**

- Against a naive reference.
- Odd dimensions.
- Peak memory bounded (counting allocator).
- Metadata snapshot.

**Example.** `build_pyramid`: builds the levels for a synthetic 20k × 20k image.

**ADR.** Pyramid scheme: filter, level count, and chunk alignment.

---

### P4.5 — Tile service and DZI (`siq-pyramid::tiles`, `/api/siq/data/…`)

**Budget** ~850 · **Depends on** P4.4, P2.3 · **Requirements** REQ-DATA-005, REQ-NET-003, REQ-VIS-001

**Goal.** Deep-zoom tiles on demand, with LUTs, for OpenSeadragon.

**Scope**

- **DZI geometry:** tile 254, overlap 1, levels 0..⌈log₂(max dim)⌉. DZI levels map to Zarr
  levels.
- **`.dzi` descriptor** per dataset, channel and plane.
- **Tile render:** read the region at the level, apply the LUT (`min`, `max`, `gamma`, `cmap` query
  parameters), encode JPEG (opaque) or PNG.
- **Cache:** an LRU bounded by bytes, keyed on (dataset, c, z, t, level, col, row, lut-hash), with
  `ETag` / `Cache-Control`.
- **`/value` probe:** true 16-bit values.
- **DZI export to disk** as a job.

**Tests**

- Geometry proptest against OpenSeadragon's conventions: edge tiles, overlap.
- HTTP tests: the descriptor, tiles, 404s, and LUT parameter validation.
- Cache eviction.
- Concurrent requests.

**Example.** `export_dzi`: writes a static pyramid for a synthetic dataset.

**ADR.** On-demand DZI rendered from OME-Zarr, compared with pre-rendering.

---

### P4.6 — OME-TIFF export (`siq-storage::ometiff`)

**Budget** ~800 · **Depends on** P4.4 · **Requirements** REQ-DATA-004

**Goal.** Compatibility export for Fiji, QuPath and commercial tools.

**Scope**

- **Evaluate the `tiff` crate's encoder** for BigTIFF, tiles and SubIFDs. If it falls short, write a
  minimal BigTIFF writer: IFD chain, tiled strips, SubIFDs for the pyramid levels, and
  deflate (`miniz_oxide`) or no compression.
- **OME-XML** from the manifest, in level 0's ImageDescription.
- **Streaming** from Zarr, as a job.

**Tests**

- Our own structural reader validates the IFDs, tiles and SubIFDs.
- The `tiff` crate decodes level 0.
- OME-XML snapshot.
- Manual Fiji and QuPath checks, recorded.

**Example.** `export_ometiff`.

**ADR.** OME-TIFF export profile.

---

### P4.7 — `datasets` Thing and the processing-on-snap pipeline

**Budget** ~700 · **Depends on** P4.3–P4.5 · **Requirements** REQ-DATA-001, REQ-ACQ-006, REQ-NET-003

**Goal.** Datasets as first-class API objects. A snap becomes viewable automatically.

**Scope**

- **`datasets` Thing:**
  - `summary` (paged);
  - `snap_to_dataset(channels?)`;
  - `export_ome_tiff`;
  - `export_dzi`;
  - `delete` (Admin from M9);
  - event `created`.
- **Snap pipeline:** raw → Zarr level 0 → pyramid job.
- **Routes:** `/api/siq/data/{id}/manifest` and `/api/siq/data/{id}/thumb.jpg` (rendered from the
  top level).

**Tests**

- End to end over HTTP: snap → dataset → pyramid → tile request.

**Example.** `examples/python/snap_and_fetch_tile.py`.

**ADR.** An amendment to the dataset ADR covering processing on snap and the dataset API.

---

### P4.8 — Dataset browser and 2D viewer UI

**Budget** ~900 · **Depends on** P4.7, P3.2 · **Requirements** REQ-VIS-001, REQ-VIS-005

**Goal.** Browse datasets and inspect them in deep zoom.

**Scope**

- **Datasets page:** adapt `microscope`'s gallery (`GalleryPage`, `GalleryCard`) into cards with
  thumbnails, filters, and a metadata panel.
- **Viewer2D:** adapt `DeepZoom.vue` to OpenSeadragon, with:
  - one tiled image per channel, composited additively;
  - per-channel LUT controls (min, max, gamma, colour, visibility) that update the tile URL
    parameters, debounced;
  - a scale bar from µm/px;
  - a pixel probe on hover (the `/value` endpoint, debounced);
  - z/t sliders;
  - the overlay layers infrastructure (the copied `overlay/`), used by M8.

**Tests**

- Tile URL builder.
- LUT store.
- Probe debounce.
- Component mount/unmount, with OpenSeadragon destroyed cleanly.

**Example.** `/#/dev/viewer2d` against a synthetic dataset.

**ADR.** An amendment to ADR-0009 covering the 2D viewer integration and multi-channel
compositing.

---

### P4.9 — Synthetic dataset generator (`apps/siq-datagen`)

**Budget** ~700 · **Depends on** P1.6, P4.1 · **Requirements** REQ-SIM-004, REQ-STI-004

**Goal.** Datasets with exact ground truth, for algorithm work, without running devices.

**Scope**

- **Subcommands:**
  - `tilescan`: world, grid, overlap, stage-error model → raw tiles, nominal poses, and true
    offsets;
  - `zstack`: a height-field world → slices with defocus, plus the true height map;
  - `beads`: a 3D volume through a PSF → stack, plus the true volume.
- Uses the `siq-sim` image formation offline.
- `--smoke` gives small sizes.
- **Plugin-style subcommands**, so M12 (SIM/HiLo) and M13 (light field) can add theirs.

**Tests**

- Ground-truth consistency.
- The output loads as a dataset.

**Example.** `cargo run -p siq-datagen -- tilescan --smoke`.

**ADR.** Synthetic dataset and ground-truth file format.
