# M6 — Workflows: tile scan, autofocus, focus maps, z-stacks, sequences

**Outcome:** scriptable, cancellable, pausable acquisition workflows that produce datasets:

- stop-and-go tile scans with focus maps;
- z-stacks;
- multi-position and time-lapse sequences.

Each workflow can be set up and monitored in the UI, and driven from Python.

**Exit criteria:**

- on the simulator, a 5×5 tile scan with a 9-point focus map over a tilted, curved specimen
  produces a dataset whose tiles are all in focus (AF error < DOF/4);
- cancel and pause work at any step;
- the Python examples run end to end.

---

### P6.1 — Workflow engine (`siq-workflow::engine`)

**Budget** ~850 · **Depends on** P2.4, P4.1, P5.2 · **Requirements** REQ-WF-005, REQ-DATA-001

**Goal.** One executor for every acquisition plan.

**Scope**

- **`Plan`:** a serde/JsonSchema enum of composable steps.
- **`Executor`:**
  - async, with `Clock` and `CancellationToken`;
  - pause and resume at step boundaries;
  - `Progress { done, total, eta, step }`;
  - events;
  - an error policy per step (retry n, skip, abort).
- **`DeviceSet`:** `Arc<dyn …>` for each device trait, plus the acquisition, calibration registry
  and dataset writer.
- Creates the dataset and records the plan, with the instrument and calibration snapshots, in the
  manifest.
- **Determinism:** every simulator uses `Clock`, so tests run under `tokio::time::pause`.

**Tests**

- Step ordering.
- Cancellation mid-move and mid-capture: devices are left in a safe state.
- Pause and resume.
- Error policies.
- Manifest contents.

**Example.** `run_plan_sim`.

**ADR.** The workflow engine model.

---

### P6.2 — Tile scan planner and step

**Budget** ~750 · **Depends on** P6.1 · **Requirements** REQ-WF-001, REQ-STG-005, REQ-STI-003

**Goal.** Turn a region into an efficient stop-and-go grid.

**Scope**

- **Planner:**
  - a region (rectangle or polygon, in stage µm) and the field of view from the calibration and
    sensor give a grid with the given overlap %;
  - ordering: snake, raster, or nearest-neighbour;
  - a backlash-consistent approach;
  - settle time;
  - estimates: tile count, time, bytes, and RAM and disk checks.
- **Step:** move → settle → focus z from the surface provider → for each channel, `select_channel`,
  then snap with `save`. The nominal pose and the planned grid index are recorded.

**Tests**

- Planner proptests: full coverage, overlap ≥ the requested value, polygon clipping.
- Estimate accuracy against a simulated run.
- A 3×3 simulated scan produces the expected dataset.

**Example.** `tilescan_sim` (smoke: 3×3 with the `fast` profile).

**ADR.** The scan planning model (`microscope`'s ADR-0028 as a reference).

---

### P6.3 — Autofocus and focus maps (`siq-workflow::autofocus`)

**Budget** ~800 · **Depends on** P6.1 · **Requirements** REQ-WF-002

**Goal.** Fast, robust focus on the focus drive, and a focus surface for scans.

**Scope**

- **Autofocus:**
  - switch the camera to a fast mode (binning/ROI), then restore it;
  - a coarse sweep on the focus drive, using a `Bounded` fan-out tap for the focus measure;
  - a fine search: parabolic fit around the peak, or golden section;
  - returns z, the focus curve, and the confidence.
- **Focus map:**
  - sample points, on a grid or explicit;
  - autofocus at each;
  - a surface fit: plane least squares with outlier rejection, or bilinear / thin-plate spline
    interpolation;
  - validation residuals;
  - a `FocusSurface` provider for the tile scan.

**Tests**

- On the simulator, a tilted and curved height field: autofocus error < DOF/4.
- A low-texture field reports low confidence.
- Surface residuals.

**Example.** `autofocus_sim`, which prints the focus curve.

**ADR.** The autofocus algorithm and the focus surface model (`microscope`'s ADR-0022 and ADR-0037
as references).

---

### P6.4 — Z-stacks, multi-position and time-lapse

**Budget** ~700 · **Depends on** P6.2, P6.3 · **Requirements** REQ-WF-003, REQ-WF-004

**Goal.** The remaining acquisition patterns, and their composition.

**Scope**

- **`ZStack`:** a range around the current or autofocus z, a step in µm, a direction; optionally
  per tile.
- **`MultiPosition`:** saved positions, with optional autofocus at each.
- **`TimeLapse`:** interval, count or duration, drift-tolerant scheduling, and a skip policy when an
  iteration overruns.
- **Composition:** for example, a time-lapse of a multi-position of tile scans with z-stacks.
- **Indexing:** datasets are indexed by t / c / z / p, consistent with the raw file names.

**Tests**

- Composition counts.
- Scheduling under paused time.
- Overrun behaviour.
- Z ordering, and backlash on the focus drive.

**Example.** `timelapse_sim`.

**ADR.** The sequence model (`microscope`'s ADR-0031 as a reference).

---

### P6.5 — `workflow` Thing, saved positions and scripting

**Budget** ~600 · **Depends on** P6.4 · **Requirements** REQ-WF-005, REQ-WF-006, REQ-NET-002

**Goal.** Expose the workflows over HTTP for the UI and for scripts.

**Scope**

- **`workflow` Thing:**
  - actions: `tile_scan(plan)`, `z_stack`, `multi_position`, `time_lapse`, `pause`, `resume`;
  - `state` and `progress` properties, observed;
  - events `tile_acquired` and `finished`;
  - plan validation errors → 422;
  - the global lock is held for the whole run;
  - cancellation is a `DELETE` of the invocation.
- **Saved positions** as a setting on the server: list, add, rename, remove.
- **Post-processing hook interface:** `on_dataset_complete`, which M8 uses to queue stitching.
- **Scripts in `examples/python/`:** `tile_scan.py`, `z_stack.py`, `timelapse.py`.

**Tests**

- HTTP integration: start, observe progress, cancel, and `GlobalLockBusy` for a manual move during
  a scan.

**Example.** The Python scripts against the simulated server.

**ADR.** The workflow API: invocation semantics, locking, and cancellation
(`microscope`'s ADR-0029 as a reference).

---

### P6.6 — Scan setup UI

**Budget** ~900 · **Depends on** P6.5, P5.4 · **Requirements** REQ-WF-001, REQ-WF-002, REQ-WF-003

**Goal.** Plan a scan visually, with honest estimates.

**Scope**

- **Stage map canvas:**
  - stage coordinates;
  - the current field-of-view rectangle;
  - saved positions;
  - rectangle and polygon region drawing;
  - a tile-grid preview.

  The background is the live mosaic once M7 is done.
- **Plan form:**
  - objective, channels, overlap, ordering;
  - focus strategy: none, fixed, map (point picking), or per tile;
  - z-stack options.
- **Estimate panel:** tiles, time, data size, RAM and disk check, and warnings.
- **Start**, which hands over to the run monitor.

**Tests**

- Map geometry (stage ↔ canvas).
- Form validation against the plan's JSON Schema.
- Estimate display.

**Example.** `/#/dev/scan`.

**ADR.** Scan setup UX.

---

### P6.7 — Run monitor and sequence UI

**Budget** ~700 · **Depends on** P6.6 · **Requirements** REQ-WF-004, REQ-WF-005

**Goal.** Watch and control running workflows. Set up multi-position and time-lapse.

**Scope**

- **Run panel:**
  - progress, ETA, and the current step;
  - thumbnails of the latest tiles;
  - pause, resume, and cancel (with confirmation);
  - errors, with retry and skip choices where the policy allows them.
- **Multi-position and time-lapse setup:** adapt `microscope`'s `SequencePage`.
- **Notifications** when a run finishes or fails.

**Tests**

- State transitions.
- Confirmations.
- Thumbnail updates.

**Example.** `/#/dev/run`.

**ADR.** An amendment to the scan setup UX ADR covering run control and recovery choices.
