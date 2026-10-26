# ADR-0050: The shading correction, and the BaSiC simplification

- **Status:** Accepted
- **Date:** 2026-10-25
- **Phase:** P5.3
- **Related:** ADR-0026, ADR-0032, ADR-0043, ADR-0049, REQ-CAL-002; Peng et al., "A BaSiC tool for
  background and shading correction of optical microscopy images" (Nat. Commun. 8, 14836, 2017)

## Context

- **A microscope doesn't light its field evenly.** Vignetting darkens the corners, often by
  10–30 %, and the illumination may sit off centre. Each tile of a mosaic is darker at its
  edges, so the mosaic shows a grid of seams.
- **The sensor adds its own offset** to every pixel: a dark level, and each pixel's own deviation
  from it.
- **The correction kernel exists** (P1.5): `(raw − dark) × gain`, clamped. What was missing is
  where the dark and the gain come from, how they are kept, and where they apply.
- **What it must meet:**
  - REQ-CAL-002: dark and flat calibration per objective × channel, and retrospective estimation;
  - M5's exit criterion: the seam contrast of a synthetic tile scan reduced at least 5×;
  - P5.3's tests: a known vignetting recovered within 2 %.
- **Two ways to measure a flat:**
  - captured: frames of a blank slide, or of the sample out of focus;
  - estimated after the fact, from the tiles of a scan, when no flat was taken.

## Decision

- **The model:** a frame corrected is `(raw − dark) × gain`, the gain `1 / flat` at each pixel's
  place on the sensor, the flat relative to its mean.
- **The dark** is the mean of frames taken with every light off (16 by default), rounded, of the
  whole sensor, at the camera's binning and exposure.
  - It is kept per pixel, as a NumPy `.npy` file beside its record, so that `numpy.load` reads
    it.
  - Its mean level is in the record.
- **The flat is a polynomial surface** of degree 6 in `x` and `y` over the sensor, 28 terms,
  fitted by least squares:
  - fitted to the mean of the flat frames less the dark level, shrunk by area to at most 128
    cells a side;
  - normalised to a mean of 1;
  - refused where any place gets under a twentieth of the mean;
  - kept as its coefficients in the record's JSON, with its least and greatest light.

  Evaluated at any pixel's place, it serves every region, binning and preview size. The gains of a
  frame's geometry are computed once, a row's polynomial at a time.
- **The capture** (`siq-workflow::routines::shading`):
  - the dark with every light off, then each light back at its level;
  - the flat at nine places a field apart about the stage, two frames each, after moving the focus
    by `defocus_um` (0, for a blank slide). The stage and focus are put back after.
- **The retrospective estimate (BaSiC, simplified)**, a job of `siq-jobs`:
  - each frame of the whole sensor, less a dark level, is shrunk to the grid and divided by its
    own mean, so that a darker place weighs as a brighter one;
  - each cell's **median** across the frames is the flat there: the sample's features at any one
    place don't move it;
  - the medians are fitted like a captured flat;
  - five frames or more are needed. The darkfield isn't estimated: the dark level is given, or is
    the level of the dark in force.
- **Storage:** the registry of ADR-0049, made generic over the kind (`Registry<R: Versioned>`):
  - darks and flats under `<settings>/shading/objective-<id>_channel-<id>/`, one JSON file each,
    never changed;
  - the newest of each kind is in force for its objective and channel;
  - a record's data is written beside it before the record, each file as `.part` then renamed.
- **Where it applies:**
  - **`FlatFieldView`** corrects frames of one region and binning as they are read: the dark
    cropped to the region (or its level, at another binning) and the flat's gains;
  - **the previews, when `display.flat_field` is on:** the dark level and the flat in force for
    the objective and channel, applied after shrinking, as the flat is smooth. They follow the
    objective and channel as the frames' stamp does;
  - **stitching** (M7, M8) will read them through `FlatFieldView`.
- **The `calibration` Thing:** `capture_dark`, `capture_flat` and `estimate_shading`, and
  `status`, now a computed property, with each objective's dark and flat per channel.

## Alternatives considered

- **A Gaussian low-pass of the measured flat, as the brief has it:** holding the edges beyond the
  image biases a curved fall-off toward the centre's level at the corners, by about 1 % at σ of a
  sixteenth of the side. The polynomial has no edges, needs no binary file, and smooths by its
  form.
- **A full-resolution gain map in the registry:** 400 MB of `f32` for a 100 MP sensor, per
  objective and channel. It would keep each pixel's own gain (0.5 % on the simulator), which is
  within the noise of a single frame.
- **BaSiC itself** (low-rank and sparse decomposition, with a DCT-smoothed flat and darkfield):
  more code and an iterative solver, for the darkfield, which a dark capture gives directly. The
  per-cell median of normalised tiles, fitted, recovers the vignetting within 1 % from 48 tiles.
- **The per-cell mean across tiles:** cells pull it down where they happen to fall. The median
  holds to the background.
- **The flat corrected into the dataset's image at write time:** raw stays raw, so that a better
  flat later still applies. Readers correct.
- **Darks keyed by channel alone,** since the objective doesn't change them: one key per objective
  and channel keeps the registry uniform, and a dark takes seconds.
- **A defocused sample as the only flat target:** on the simulator, the cells world's background
  varies by ±5 % over 200 µm, more than nine places and a modest defocus average away. A blank
  slide is the reliable target; defocus is an option.

## Consequences

- **Measured:**

  | | Flat against the vignetting | Seams before | Seams after |
  |---|---|---|---|
  | Captured: a blank slide, 8 frames (`siq-calib` test) | within 0.11 % | | |
  | Captured: over HTTP, 50 µm out of focus, 18 frames (`siq-server` test) | within 0.07 % | | |
  | Estimated: 6 × 5 tiles of 320 × 240 (`siq-calib` test) | within 1.04 % | 9.9 % | 0.05 % |
  | Estimated: 8 × 6 tiles of 2048 × 1536 (`estimate_flat`, release) | within 0.96 % | 10.1 % | 0.002 % |

  - The test measures the seams against the tiles without vignetting: the seam bands (the outer
    15 % each way) against the centre third.
  - `estimate_flat` measures them over all 48 tiles, which averages the sample away.
  - The estimate took 0.19 s for 48 frames of 3 MP; rendering the tiles took 3.8 s.
- **The criterion holds with a wide margin:** a 5× reduction was asked, and the test gives about
  200×.
- **What isn't corrected:** dust, each pixel's own gain, dark current at another exposure, and a
  darkfield in a scan without a dark.
- **The previews' window still comes from the raw frame:** corrected corners grow brighter, within
  the window of the brightest place.
- **Status is computed on each read,** so a flat estimated by a job is in it as soon as the job
  ends.

## Verification

- `crates/siq-calib/tests/shading.rs`:
  - a captured flat against the vignetting, and a frame corrected through `FlatFieldView`;
  - a region's and a binning's gains;
  - a tile scan's estimated flat, and its seams;
  - the registry's darks and flats.
- `crates/siq-preview/tests/preview.rs`: a shaded frame shown flat once corrected.
- `crates/siq-server/tests/shading.rs`: the dark, the flat, `status`, the previews' toggle, and
  the estimate as a job, over HTTP.
- **`estimate_flat`:** the measurements above.
