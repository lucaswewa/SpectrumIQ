# siq-calib

Calibration (ADR-0049): the stage–camera affine of each objective, fitted from known stage moves
and the image shifts they cause; the stage's backlash; and the registry that keeps every
calibration, versioned, in the settings folder. Pure: the routine that moves the stage and takes
the frames is [`siq-workflow`](../siq-workflow/README.md)'s, and the `calibration` Thing runs it.

- **The model** (architecture §13, ADR-0018): a point at stage position `S` appears at sensor
  pixel `c + L·(S − P)` with the stage at `P`. Moving the stage by `m` moves the image's content
  by `−L·m`. `L`, in pixels per µm, holds the pixel size, the camera's rotation, any scale
  difference between the axes, and their shear.
- **The fit:** least squares of `shift = t − L·move` over the moves. The offset `t`, common to
  every move, is the reference frame's own position error, so it isn't counted as a residual.
  Moves along one line, or too few, are refused as degenerate.
- **The residuals** are on the stage, in µm: the move reported, less the move its shift shows. The
  stage's own repeatability bounds them at any magnification but the lowest, so the thresholds are
  in µm, not pixels.
- **The backlash:** the shift between arriving at one place moving toward −x and −y and arriving
  moving toward +x and +y, through `L⁻¹`, in µm.
- **The registry:** one JSON file per calibration, never changed or removed, under
  `<settings>/calibrations/objective-<id>/`. An objective's newest calibration is the one in
  force; the older ones stay, so that frames stamped with them can be read again.

## Place in the architecture

- **Layer:** L3 Processing.
- **Depends on:** `siq-types`; `chrono`, `schemars`, `serde`, `serde_json`, `thiserror`,
  `tracing`.
- **Used by:** `siq-workflow` (the routine), `siq-server` (the `calibration` Thing, and the
  frames' stamp); stitching (M7, M8) reads the records to place tiles.
- **Phase:** P5.2.

## API

| Item | What |
|---|---|
| `Move { stage, shift }` | A move in µm, as the stage reports it, and the image shift it caused, in sensor pixels |
| `fit(&[Move])` | `Fit { affine, residuals }`, or `CalibError::Degenerate` |
| `fit.residual_rms()`, `residual_max()`, `pixel_size()` | On the stage, in µm; the mean pixel size on the sample |
| `fit.check(&Thresholds)` | `Ok`, or `CalibError::Rejected` with the reason |
| `Thresholds` | `min_moves` (6), `max_residual_rms_um` (1.5), `max_residual_um` (4.0) |
| `backlash(&fit, shift)` | The backlash on `x` and `y`, in µm |
| `CalibrationRecord::new(objective, &fit, backlash, method)` | A calibration as it is kept: `id`, `objective`, `created`, `affine`, `px_size_um`, `rotation_deg`, `scale`, `shear`, `residual_rms_um`, `residual_max_um`, `backlash_um`, `moves`, `method`; `affine()` gives `L` as an `Affine2` |
| `CalibrationRegistry::open(settings)` | The calibrations of a settings folder, read once; a file that isn't one is left out with a warning |
| `registry.save(&record)` | Written to a `.part` file, then renamed: in force from now |
| `registry.current(objective)`, `history(objective)`, `get(id)` | The one in force, every one newest first, and any by id |

## Examples

None of its own: `siq-server`'s `calibrate_sim` runs the whole calibration.

## Features

None.

## Testing

`cargo test -p siq-calib`:

- a known affine (0.4°, 0.2 % scale, a shear of 0.001) recovered from 16 noisy moves: scale within
  0.02 %, rotation within 0.01°;
- an offset common to every move is no residual;
- moves along one line, too few, or none, refused as degenerate; a poor fit, and too few moves,
  refused by the thresholds;
- the backlash from either side's arrival;
- the registry's versions: the newest in force, the history newest first, by id, and read again
  from disk with a broken file left out.

`siq-server`'s `calibration` test runs it on the simulated instrument.

## Limitations

- **Linear only:** lens distortion isn't modelled. Over the moves of a calibration, its residual
  shows in the fit's residuals.
- **One calibration per objective:** binning and the region of interest are accounted for in the
  shifts, so a calibration holds at any of them; a different camera or adapter needs a new one.
- **The stage's precision bounds it:** the rotation's error is about the stage's repeatability over
  the moves' reach. A small field calibrates less precisely than a large one.
