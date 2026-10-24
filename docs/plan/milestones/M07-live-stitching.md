# M7 — Real-time live stitching (wgpu)

**Outcome:** goal "Phase 1". While the stage moves, each frame is placed on a large GPU canvas
through a deterministic affine (calibration × pose). The server renders the client's viewport into
a low-bandwidth stream. The live mosaic doubles as the navigation map for scan setup.

**Exit criteria:**

- on the simulator, jogging across a 6×6-field area paints a coherent mosaic;
- placement error is ≤ 2 canvas px at canvas scale ½ (it is limited by the stage error, by
  design);
- painting never causes lost camera frames;
- VRAM stays within the configured budget.

---

### P7.1 — GPU foundation (`siq-gpu`)

**Budget** ~800 · **Depends on** P0.3b · **Requirements** REQ-STI-001, REQ-3D-005, REQ-NFR-002

**Goal.** Shared, safe wgpu infrastructure for live stitching and the 3D kernels.

**Scope**

- **Adapter selection:** prefer the discrete NVIDIA GPU; report the features and limits. In
  particular, report `max_storage_buffer_binding_size` and `max_texture_dimension_2d`, and request
  the higher adapter limits.
- **A dedicated GPU thread** owns the `Device` and `Queue`. `GpuHandle` submits work over a channel,
  with async completion. P4.3's GPU lane moves here.
- **Staging rings** for upload and readback.
- **u16 upload:** packed into a u32 storage buffer, or `R16Uint` textures where supported.
- **Pipeline cache.**
- **WGSL** included with `include_str!` and validated by naga in tests.
- **Timestamp queries** when available.
- **Device-lost handling.**
- **CPU fallback flag.**

**Tests**

- Skip with a logged reason when no adapter is present.
- Upload and readback round trip.
- A LUT kernel against the CPU reference.
- Limits reporting.

**Example.** `gpu_info`.

**ADR.** wgpu usage: backends, buffers vs textures, limits, threading.

---

### P7.2 — Live mosaic canvas (`siq-stitch-live`)

**Budget** ~900 · **Depends on** P7.1, P5.2, P5.3 · **Requirements** REQ-STI-001, REQ-STI-003

**Goal.** A sparse, paged GPU canvas with affine painting and feathered blending.

**Scope**

- **Canvas:**
  - 4096² pages, allocated sparsely;
  - format fixed by the ADR (e.g. u16 packed in u32, or `R32Float`);
  - `canvas_scale` (default ½);
  - a VRAM budget with LRU eviction, where evicted pages are downsampled into host RAM.
- **Paint kernel:**
  - input: a binned, flat-corrected frame;
  - affine `M`: frame px → canvas px;
  - dispatch per destination page over the frame footprint;
  - for each pixel: inverse map, bilinear sample, alpha feathered by the distance to the frame
    edge, and a blend.
- **Painting policy:** paint only when the frame's `PoseQuality` is `Settled` or `Interpolated` with
  uncertainty < threshold. The newest frame wins inside a feather band.
- **`clear()`.**

**Tests**

- GPU against a CPU reference paint.
- Placement accuracy with a known affine.
- Eviction under a small budget.
- Page boundaries.

**Example.** `paint_sim`: paints synthetic tiles and writes a viewport PNG.

**ADR.** Live canvas design: format, paging, blending, and painting policy.

---

### P7.3 — Viewport rendering and the `mosaic_live` Thing

**Budget** ~700 · **Depends on** P7.2, P2.3 · **Requirements** REQ-STI-001, REQ-PRV-001

**Goal.** Stream what the client looks at, at constant bandwidth.

**Scope**

- **Viewport render kernel:** center in µm, zoom, output size ≤ 1920×1080. It samples the pages,
  applies the LUT to u8 on the GPU, reads back, and encodes JPEG into `MjpegStream`
  `mosaic_live/view`.
- **`mosaic_live` Thing:**
  - `enabled`, `viewport`, `bounds`, `canvas_scale`;
  - `clear`;
  - `save_snapshot`, which writes the pages at canvas scale to a dataset in OME-Zarr.
- **Wiring:** a `LatestOnly` fan-out consumer. Binning and flat-field run on the frame-path pool,
  then the frame goes to the GPU thread.

**Tests**

- Viewport geometry.
- End to end on the simulator: move the stage and assert that the painted region matches.
- Lost frames stay 0 under load.

**Example.** `live_mosaic_sim`.

**ADR.** An amendment to the live canvas ADR covering viewport streaming.

---

### P7.4 — Live mosaic UI

**Budget** ~700 · **Depends on** P7.3, P6.6 · **Requirements** REQ-STI-001, REQ-WF-001

**Goal.** Navigate by the mosaic, and plan scans on it.

**Scope**

- **Mosaic page:** the stream image with pan and zoom, which set `viewport` (throttled).
- **Overlays:** the current camera footprint and the bounds minimap.
- **Actions:** click-to-move, clear, and save.
- **Scan setup:** the stage map uses the live mosaic as its background.

**Tests**

- Viewport maths.
- Throttling.
- Footprint overlay placement.

**Example.** `/#/dev/mosaic`.

**ADR.** Mosaic navigation UX.
