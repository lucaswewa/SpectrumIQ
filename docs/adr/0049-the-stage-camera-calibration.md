# ADR-0049: The stage–camera calibration: its model, storage and versioning

- **Status:** Accepted
- **Date:** 2026-10-25
- **Phase:** P5.2
- **Related:** ADR-0018, ADR-0041, ADR-0048, REQ-CAL-001, REQ-CAL-003

## Context

- **Everything that puts pixels in stage space needs `L`:** the live mosaic (M7), stitching
  (M8), and an overlay drawn on the stage. Architecture §13 (ADR-0018) defines it as
  `sensor_px = c + L·(S − P)`, per objective, with `c` the sensor's centre.
- **The nominal `L` is not good enough:**
  - the sensor's pitch over the magnification is off by the objective's tolerance, a percent or
    two;
  - a camera is never mounted square: 0.4° over a 50 mm scan is 350 µm;
  - the stage's own scale error is in what it reports.
- **What it must meet:**
  - M5's exit criterion: on the simulator, the injected rotation, scale and shear recovered within
    0.1 % in scale and 0.05° in rotation;
  - REQ-CAL-003: calibrations versioned, and referenced by the frames and datasets taken under
    them. `FrameMeta.calibration` has held a `CalibrationId` since P0.2.
- **What the stage gives:** where it was told to go, not where it went. It has backlash, a scale
  error, whole steps, and unrepeatability of about half a micrometre. The simulated stage has all
  four.

## Decision

- **The model:** `L`, a 2 × 2 matrix in pixels per µm, per objective, with no translation.
  - Shifts are in sensor pixels: a binned frame's are scaled up, and a region of interest doesn't
    change them. One calibration holds at every binning and region.
  - The record keeps `L`, and its decomposition for people: the mean pixel size, the rotation, the
    scale of each axis, and the shear (`Affine2::decompose`).
- **The measurement** (`siq-workflow::routines::stage_camera`):
  - a reference frame where the stage is;
  - a star of 16 places, at 0.4 and 0.3 of the nominal field each way, on the axes and the
    diagonals, each approached from the same side so that the backlash is the same at every
    arrival;
  - a frame at each, registered against the reference (ADR-0048). A PCE under 100 stops it as
    featureless;
  - then one place approached from either side, four times, for the backlash;
  - and back to the start.
- **The fit** (`siq-calib`):
  - least squares of `shift = t − L·move`, with `t` an offset common to every move: the reference
    frame's own position error;
  - refused as degenerate with fewer than three moves, or when the moves' smaller spread is under
    a thousandth of the larger.
- **The thresholds are on the stage, in µm:** at least 6 moves, 1.5 µm RMS, 4 µm at most. A
  residual is the move reported less the move its shift shows. Stage repeatability, not the
  registration, bounds the residuals at any magnification but the lowest.
- **The backlash:** the shift between the two arrivals, averaged, through `L⁻¹`.
- **Storage:** one JSON file per calibration, `<settings>/calibrations/objective-<id>/<created>_<id8>.json`.
  - `settings` is `[server] settings`, resolved against the configuration's folder:
    `C:/ProgramData/SpectrumIQ/settings` on the instrument, `settings/` (ignored by git) for the
    simulator.
  - A file is written as `.part`, then renamed. It is never changed or removed.
  - **An objective's newest calibration is the one in force.** Calibrating again adds a version.
  - The registry reads every file when it opens. A file that isn't a calibration is left out, with
    a warning.
- **References:**
  - the acquisition's context carries the objective in use and its calibration in force, so every
    frame's `FrameMeta` has both. They are set at start, by `objective.select`, and after a
    calibration;
  - a snap's dataset keeps a copy of the record in its manifest's `calibration` snapshot, with its
    hash, and takes its pixel size from it.
- **The `calibration` Thing:**
  - `status`: each configured objective with its calibration in force, or `null`;
  - `stage_camera`: calibrates the objective in use, which is the context's, else the turret's,
    else the only one configured.
- **The simulator's truth:** `[sim.mount]` turns, scales and shears the simulated camera's image
  against the stage (`rotation` in degrees, `scale_error`, `shear`). `config/sim.toml` sets 0.4°,
  0.2 % and 0.001.

## Alternatives considered

- **Thresholds in pixels**, 0.5 px RMS: they refused every calibration at 10×, where the stage's
  0.5 µm is 1.3 px. They would pass a poor stage at 2×.
- **A fit with no offset:** the symmetric star cancels the reference frame's error in `L`, but the
  error stayed in every residual. Fitting `t` costs nothing.
- **Moves out to a third and a sixth of the field,** as first planned: on a 770 µm field, the
  rotation's spread from the stage's noise was about 0.03°, too near the 0.05° criterion.
  Precision goes with the reach, and at 0.4 a diagonal move still leaves a third of the frame to
  register on.
- **The stage's reported position after each move, instead of the planned place:** GRBL reports to
  1 µm, coarser than the emulator's 0.625 µm steps. It is no better.
- **One file per objective, rewritten on each calibration,** or a database: rewriting loses the
  versions that frames refer to, and a crash mid-write loses the one in force. One file per
  version is append-only, readable, and survives a crash.
- **`stage_camera(objective)`, as the brief has it:** only the objective in the light path can be
  calibrated. Selecting another is `objective.select`, which the wizard (P5.4) does first.
- **A rotation in the dataset's image:** NGFF 0.5 has scale and translation only. The image takes
  the calibrated pixel size, and the rotation stays in the manifest's copy, for stitching to apply.

## Consequences

- **Measured** (release, `calibrate_sim`: 4096 × 3072 binned 2 × 2, 10× on 3.76 µm, a field of
  1.5 × 1.2 mm; the test finds the same):

  | | Found | Injected | Off |
  |---|---|---|---|
  | Scale x, px/µm | 2.667032 | 2.666226 | +0.030 % |
  | Scale y, px/µm | 2.666853 | 2.666226 | +0.024 % |
  | Rotation | 0.3912° | 0.4° | −0.009° |
  | Shear | 0.00093 | 0.001 | −0.00007 |
  | Backlash x, y | 2.19, 2.55 µm | 2 µm | |

  The residuals were 0.87 µm RMS and 1.34 µm at most. It took 7.8 s, most of it the stage's
  moves. The injected scale includes the stage's own 0.05 %: the moves are the ones the stage
  reports.
- **The stage bounds the precision:** about its repeatability over the moves' reach, so a small
  field calibrates less precisely. At 1024 × 768 (385 µm), scale y was 0.45 % off. The simulated
  camera's full sensor at 10× is 4.4 mm across. At 60× it is 730 µm, near the test's field.
- **Registration is not the limit:** a few hundredths of a pixel against the stage's half
  micrometre. On the emulator, a place between whole steps lands up to 0.31 µm off, and without
  noise the shifts follow the steps exactly.
- **Recalibration is cheap and safe:** a new version comes into force, the old ones stay, and
  frames and datasets keep their references.
- **Not modelled:** lens distortion, and a camera or adapter changed under the same objective
  (calibrate again).

## Verification

- `crates/siq-calib/tests/calib.rs`:
  - a known affine from noisy moves;
  - an offset common to every move is no residual;
  - degenerate moves, and poor fits, refused;
  - the backlash;
  - the registry's versions, read again from disk.
- `crates/siq-server/tests/calibration.rs`, over HTTP on the simulator:
  - the mount recovered within the criterion;
  - the backlash measured;
  - the calibration kept, in `status`, and on the next frame;
  - a snap's dataset keeping a copy.
- **`calibrate_sim`:** the measurements above.
