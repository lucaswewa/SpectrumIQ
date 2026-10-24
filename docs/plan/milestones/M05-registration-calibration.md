# M5 — Registration and calibration

**Outcome:**

- sub-pixel image registration;
- the stage↔camera affine calibration per objective;
- dark/flat and retrospective shading correction;
- calibration wizards.

Everything later that places pixels in stage space depends on this milestone.

**Exit criteria:**

- on the simulator with an injected rotation, scale and shear, the calibration recovers them within
  0.1 % scale and 0.05°;
- flat-field correction reduces the seam contrast of a synthetic tile scan by at least 5×;
- the first-run wizard guides a new user to a calibrated state.

---

### P5.1 — Phase correlation (`siq-register`)

**Budget** ~800 · **Depends on** P1.5 · **Requirements** REQ-STI-002, REQ-CAL-001

**Goal.** Robust sub-pixel translation estimation, the workhorse for calibration and stitching.

**Scope**

- **Correlation:** windowing (Hann or Tukey), then the normalized cross-power spectrum via
  `realfft` in 2D.
- **Peaks:** top-K candidates. Wrap ambiguities are resolved by NCC on the actual overlap for each
  of the 4 hypotheses.
- **Sub-pixel refinement:** parabolic or Foroosh.
- **Confidence:** peak-to-correlation energy (PCE) and NCC.
- **Coarse-to-fine:** search on downsampled images, then refine at full resolution in a window.
- **FFT plan cache.**

**Tests**

- Ground-truth shifts on `siq-synth` patches with noise: error < 0.1 px.
- Large shifts.
- Low overlap.
- Detection of periodic patterns (low confidence reported).

**Example.** `register_pair`.

**ADR.** The registration method (`microscope`'s ADR-0023 as a reference).

---

### P5.2 — Stage–camera calibration (`siq-calib`, `siq-workflow::routines`)

**Budget** ~800 · **Depends on** P5.1, P2.4 · **Requirements** REQ-CAL-001, REQ-CAL-003

**Goal.** Measure the affine between stage µm and camera pixels, per objective, and version it.

**Scope**

- **`siq-calib` (pure):**
  - a least-squares fit of the 2×2 matrix plus residuals, from N known moves and measured shifts;
  - a backlash estimate from forward and backward moves;
  - validation thresholds;
  - `CalibrationRecord { id, objective, created, affine, px_size_um, residual_rms, backlash,
    method }`;
  - a `CalibrationRegistry` of versioned JSON files in the settings folder.
- **`siq-workflow::routines::stage_camera`:** the acquisition routine (move, settle, snap,
  register), built on the device traits. This is the crate's first module; the engine comes in
  M6.
- **`calibration` Thing:** `stage_camera(objective)` and `status`.
- `FrameMeta.calibration` is stamped from the registry.

**Tests**

- On the simulator with a known injected calibration: recovery within the targets.
- Rejection of degenerate move sets.
- Registry versioning.

**Example.** `calibrate_sim`.

**ADR.** The calibration model, its storage, and its versioning.

---

### P5.3 — Shading correction (`siq-calib::shading`)

**Budget** ~850 · **Depends on** P5.2, P4.3 · **Requirements** REQ-CAL-002

**Goal.** Seamless mosaics: dark and flat calibration, with retrospective estimation as the
fallback.

**Scope**

- **Dark capture:** illumination off, mean of N frames.
- **Flat capture:** a blank or defocused target, mean of N frames, minus the dark, normalized and
  low-pass smoothed, giving an f32 gain map.
- **Storage** per objective × channel in the registry.
- **Application:** an optional preview toggle; the dataset readers (`FlatFieldView`); and, later,
  stitching.
- **Retrospective estimation:** a simplified BaSiC run as a job. A robust per-pixel statistic across
  many tiles gives the flat, then smoothing; the darkfield estimate is optional.

**Tests**

- Known vignetting recovered within 2 %.
- Seam contrast on a synthetic scan reduced by at least 5×.
- Retrospective estimate against ground truth.

**Example.** `estimate_flat` on a `siq-datagen` tile scan.

**ADR.** The shading correction pipeline, and the BaSiC simplification.

---

### P5.4 — Calibration UI

**Budget** ~800 · **Depends on** P5.2, P5.3, P3.4 · **Requirements** REQ-CAL-001, REQ-CAL-002

**Goal.** Guided calibration in `microscope`'s wizard style.

**Scope**

- **Adapt `microscope`'s calibration wizard** (`CalibrationWizard`, `GuideStep`, `CalibrateStep`,
  `tasks`).
- **Stage–camera wizard:** choose the objective, run, see the residuals, accept or redo.
- **Dark/flat wizard.**
- **Objective setup:** turret positions ↔ objectives, with the parfocal offsets shown.
- **Status page** per objective × channel.
- **First-run prompt.**
- **Gating:** scans require a valid calibration.

**Tests**

- Wizard state machine.
- Gating rules.

**Example.** `/#/dev/calibration`.

**ADR.** Calibration UX and gating rules.
