# M8 — Post-acquisition stitching

**Outcome:** goal "Phase 2". Stage-guided, sub-pixel, globally consistent mosaics of 500 MP and
more, composited out-of-core into OME-Zarr, with QA overlays in the viewer. Scans can stitch
automatically when they finish.

**Exit criteria:**

- on `siq-datagen` ground truth (scans of 3×3 to 10×10, stage σ 5 µm, 15 % overlap):
  - RMS placement error ≤ 0.3 px and maximum ≤ 1 px;
  - no visible seams after flat-field;
- the 500 MP benchmark completes within the targets recorded in P8.5, with peak RAM ≤ 14 GB.

---

### P8.1 — Overlap graph and pairwise registration (`siq-stitch::pairs`)

**Budget** ~750 · **Depends on** P5.1, P5.3, P4.3 · **Requirements** REQ-STI-002

**Goal.** Measure every useful pairwise offset, efficiently and robustly.

**Scope**

- **Nominal tile positions** (pose × calibration → mosaic px) and the overlap graph, above a
  minimum overlap.
- **Overlap extraction** from memory-mapped raw tiles, with the flat-field applied.
- **Registration:** coarse (×4 downsampled), then fine within a search window sized from the stage
  uncertainty.
- **`Edge { i, j, shift, pce, ncc, overlap_px }`.**
- **Parallelism:** rayon over edges, bounded by the job's RAM estimate.

**Tests**

- Ground-truth offset recovery.
- Low-texture overlaps get low confidence.
- Memory bound.

**Example.** `pairs_on_dataset`.

**ADR.** The pairwise registration strategy for mosaics.

---

### P8.2 — Global alignment (`siq-stitch::solve`)

**Budget** ~700 · **Depends on** P8.1 · **Requirements** REQ-STI-002, REQ-STI-004

**Goal.** A consistent placement that removes accumulated drift: the "pose graph" of the goals,
restricted to translations because rotation, scale and shear come from the calibration (ADR-0013).

**Scope**

- **Weighted least squares over tile translations.** A weak prior ties each tile to its stage
  position (σ_stage).
- **Solvers:** dense normal equations below about 2,000 tiles; conjugate gradient beyond.
- **Robustness:** Huber IRLS, and rejection of outlier edges (residual > kσ, or low NCC).
- **Connectivity:** tiles in unconnected components fall back to their stage positions.
- **Residual report** per edge and per tile.
- **Multi-channel and z:** registration runs on the reference channel; all channels and planes
  share the XY solution.

**Tests**

- Ground truth with injected outlier edges.
- Removal of a drift ramp.
- Disconnected components.
- Solver equivalence (dense vs CG).

**Example.** `solve_on_dataset`.

**ADR.** The global alignment formulation.

---

### P8.3 — Out-of-core compositing (`siq-stitch::composite`)

**Budget** ~850 · **Depends on** P8.2, P4.2, P4.4 · **Requirements** REQ-STI-002, REQ-STI-003, REQ-NFR-002

**Goal.** Write the final mosaic chunk by chunk, within the RAM budget.

**Scope**

- **Output geometry:** bounds and the level-0 size.
- **Work unit:** output blocks of chunk multiples, processed in parallel with rayon.
- **Tile index:** a grid-bucket spatial index.
- **Per block:**
  - gather the overlapping tiles;
  - sample them with their sub-pixel shift (bilinear, or integer-only as an option);
  - weight by linear feathering (precomputed edge-distance ramps);
  - normalize;
  - write the Zarr chunk.
- **Tile views:** an LRU of memory-mapped tile views under the RAM budget.
- **Scope of output:** each channel and z plane.
- **Wrapped as a job,** with its estimate, then followed by the pyramid build.

**Tests**

- Composite of ground-truth placement against the ground-truth image (PSNR threshold).
- Seam metric.
- Peak memory.
- Integer-only mode.

**Example.** `composite_dataset`.

**ADR.** Blending and out-of-core compositing.

---

### P8.4 — Stitch job, auto-stitch and QA UI

**Budget** ~800 · **Depends on** P8.3, P6.5, P4.8 · **Requirements** REQ-STI-002, REQ-WF-001, REQ-VIS-001

**Goal.** One-click and automatic stitching, with visible quality.

**Scope**

- **The `stitch` job:** pairs → solve → composite → pyramid. The manifest's processing history
  records the parameters, residuals and versions.
- **QA:** a JSON report (edges, residuals) and an overlay endpoint (tile outlines, and edge and
  residual colours).
- **API:** `jobs.stitch(dataset, options)`; auto-stitch through the `on_dataset_complete` hook.
- **UI:**
  - a Jobs page with queue, progress and cancel;
  - a stitch options dialog;
  - Viewer2D overlay layers for tile borders and the residual heat map, through the copied overlay
    system.

**Tests**

- HTTP integration: scan → auto-stitch → tiles.
- Overlay rendering units.

**Example.** `examples/python/stitch_dataset.py`.

**ADR.** Stitch QA, re-processing, and provenance.

---

### P8.5 — Ground-truth validation and benchmarks

**Budget** ~400 (mostly tests) · **Depends on** P8.4, P4.9 · **Requirements** REQ-STI-003, REQ-STI-004, REQ-NFR-002

**Goal.** Prove accuracy and scale, and record the defaults.

**Scope**

- **Accuracy suite** over the generated scan matrix: sizes, overlaps, stage σ, noise levels, and
  textured or sparse specimens.
- **500 MP benchmark:** `siq-datagen`, then the full pipeline. Record wall time per stage, peak RAM,
  disk I/O and CPU use. Set the targets from the first measurement, with ≤ 3 min end to end as the
  initial goal.
- **Tune the defaults:** search window, coarse factor, Huber k, and feather width.

**Tests.** The accuracy suite runs in CI at small sizes, and fully with `--release --ignored`.

**Example.** `bench_stitch_500mp`.

**ADR.** An amendment to the stitching ADRs with the measured results and the tuned defaults.

---

### P8.6 — (Optional) Feature-based fallback registration

**Budget** ~850 · **Depends on** P8.5 · **Requirements** REQ-STI-002

**Goal.** Handle overlaps where phase correlation fails, such as periodic or sparse specimens.

**Scope**

- **Pure Rust:** Harris/FAST corners, BRIEF/ORB-style binary descriptors, Hamming matching, and
  RANSAC for a translation model.
- Used when the phase-correlation confidence falls below a threshold.
- Results feed the same `Edge` model.

**Tests**

- Periodic ground-truth patterns where phase correlation is ambiguous and features succeed.

**Example.** `features_pair`.

**ADR.** The feature-based fallback.
