# M11 — 3D: depth from focus

**Outcome:** z-stacks, single-field or tiled, become height maps, all-in-focus (EDF) images and
confidence maps, computed while the stack is acquired. They are viewable in deep zoom (all-in-focus)
and in a VTK.js 3D surface view (height).

**Exit criteria:**

- on `siq-datagen` height fields: height RMSE ≤ z_step/3 on textured regions;
- a 100 MP slice is processed in ≤ 150 ms on the GPU, so live DFF keeps up at 9 fps with headroom;
- a tiled z-stack scan produces stitched all-in-focus and height maps.

---

### P11.1 — DFF core on the CPU (`siq-dff`)

**Budget** ~800 · **Depends on** P1.5, P4.9 · **Requirements** REQ-3D-001, REQ-NFR-002

**Goal.** Streaming depth-from-focus with bounded memory.

**Scope**

- **Focus operators:**
  - LoG (separable Gaussian + Laplacian);
  - sum-modified-Laplacian;
  - Tenengrad;
  - window aggregation by integral-image box filter.
- **`DffAccumulator`,** updated once per slice. Per pixel it keeps:
  - the best measure and its index;
  - the neighbouring measures, for 3-point Gaussian interpolation;
  - the all-in-focus value taken from the best slice.

  That is about 2 GB at 100 MP.
- **Confidence:** peak ratio and curvature.
- **z mapping** from slice index to µm, including uneven steps.
- **Optional height-map smoothing,** median or confidence-weighted.

**Tests**

- Ground-truth height RMSE.
- All-in-focus sharpness compared with the best single slice.
- Memory bound.
- Uneven z steps.

**Example.** `dff_on_dataset`.

**ADR.** The DFF operator, interpolation, and streaming design.

---

### P11.2 — DFF on the GPU

**Budget** ~700 · **Depends on** P11.1, P7.1 · **Requirements** REQ-3D-001, REQ-3D-005

**Goal.** Live-rate DFF on the RTX 3060.

**Scope**

- **WGSL kernels:** the focus measure, and the running argmax/accumulator update.
- **Banding:** the frame is split into bands so that each storage binding stays below
  `max_storage_buffer_binding_size` (100 MP of f32 is 400 MB).
- **VRAM-aware residency:** if the accumulators don't fit, they are paged band by band.
- **CPU and GPU equivalence.**

**Tests**

- Equivalence within tolerance.
- Band boundaries.
- A performance benchmark (recorded).

**Example.** `dff_gpu_bench`.

**ADR.** GPU DFF and the banding strategy.

---

### P11.3 — DFF workflow, live DFF and outputs

**Budget** ~700 · **Depends on** P11.2, P6.4, P8.4 · **Requirements** REQ-3D-001, REQ-WF-003, REQ-DATA-003

**Goal.** DFF as part of acquisition, with standard outputs.

**Scope**

- **DFF job** on existing datasets.
- **Live DFF:** a `Bounded` fan-out tap during a z-stack feeds the accumulator, so the result is
  ready when the stack ends.
- **Outputs:** `dff.ome.zarr` holding height (f32 µm), all-in-focus (u16) and confidence (f32),
  plus the pyramid.
- **Tiled z-stacks:** DFF per tile, then stitching that registers on the all-in-focus images and
  composites the height maps with per-tile z offsets.
- **Exports:** height map as f32 TIFF, and line profiles as CSV.

**Tests**

- Integration: a simulated tiled z-stack scan → stitched all-in-focus and height maps against
  ground truth.

**Example.** `examples/python/dff_scan.py`.

**ADR.** DFF dataset outputs and tiled DFF.

---

### P11.4 — 3D viewer foundation with VTK.js (height maps)

**Budget** ~900 (web ~750, Rust endpoint ~150) · **Depends on** P11.3, P4.8 · **Requirements** REQ-VIS-003, REQ-VIS-005

**Goal.** A 3D view inside the same app, in the browser and in the desktop shell.

**Scope**

- **`@kitware/vtk.js` integration:** Vite config, tree-shaken imports, WebGL2, all assets bundled.
- **`Viewer3D` component:**
  - render-window lifecycle and resize;
  - dispose without leaks;
  - background from the theme tokens.
- **Height-map surface:**
  - source: `/api/siq/data/{id}/heightmap?level&roi`, a binary f32 grid with a JSON header;
  - geometry: a warped grid;
  - colour: a height colormap, or the all-in-focus texture;
  - axes, scale bar, and picking that shows µm coordinates.

**Tests**

- Binary decoding.
- Mount and unmount without leaks (WebGL contexts released).
- Endpoint HTTP tests.

**Example.** `/#/dev/3d-heightmap`.

**ADR.** 3D data transport to VTK.js: binary formats, level of detail, and limits.
