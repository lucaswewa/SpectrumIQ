# siq-calib

Calibration: the stage–camera affine of each objective, fitted from known stage moves and the
image shifts they cause, and the stage's backlash (ADR-0049); the shading, the dark and the flat
of each objective and channel (ADR-0050); and the registry that keeps every calibration,
versioned, in the settings folder. Pure: the routines that move the stage and take the frames are
[`siq-workflow`](../siq-workflow/README.md)'s, and the `calibration` Thing runs them.

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
- **The shading** (`shading`): a corrected frame is `(raw − dark) × gain`, the gain `1 / flat`
  at each pixel's place on the sensor.
  - **The dark** is each pixel's offset: the mean of frames taken with no light, kept as a NumPy
    `.npy` file beside its record.
  - **The flat** is a polynomial surface of degree 6 over the sensor, relative to its mean,
    fitted by least squares to the mean of flat frames less the dark, shrunk to at most 128 cells
    a side. It is smooth by its form: dust, the sample and each pixel's own gain don't fit it, and
    it has no edge effects.
  - **The estimate** (BaSiC, simplified) gives the flat from many frames of different places:
    each shrunk and divided by its mean, the median of each cell across them, then fitted.
  - **`FlatFieldView`** corrects frames of one region and binning as they are read.
- **The registry:** one JSON file per calibration, never changed or removed, under
  `<settings>/calibrations/objective-<id>/` and `<settings>/shading/objective-<id>_channel-<id>/`,
  with any data beside it. A key's newest calibration is the one in force; the older ones stay, so
  that frames stamped with them can be read again.

## Place in the architecture

- **Layer:** L3 Processing.
- **Depends on:** `siq-types`, `siq-imgproc`; `chrono`, `schemars`, `serde`, `serde_json`,
  `thiserror`, `tracing`.
- **Used by:** `siq-workflow` (the routines), `siq-jobs` (the shading estimate), `siq-preview`
  (the previews' correction), `siq-server` (the `calibration` Thing, and the frames' stamp);
  stitching (M7, M8) reads the records to place and correct tiles.
- **Phase:** P5.2; P5.3 the shading.

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
| `Registry<R: Versioned>`, `CalibrationRegistry`, `ShadingRegistry` | The calibrations of one kind |
| `Registry::open(settings)` | The calibrations of a settings folder, read once; a file that isn't one is left out with a warning |
| `registry.save(&record, &[(name, bytes)])` | Its data, then the record, each written to a `.part` file and renamed: in force from now |
| `registry.current(key)`, `history(key)`, `get(id)`, `path(&record, name)` | The one in force, every one newest first, any by id, and where its file or data is |
| `shading::Accumulator` | `new(width, height)`, `add(&frame)`, `frames()`, `mean()`, and `dark(binning)`: the mean rounded, as a `Dark { image, binning }` with its `level()` |
| `shading::FlatField { sensor, coefficients }` | `fit(&mean, area, sensor, dark)`; `at(x, y)`, the light at a sensor position; `range()`; `gains(roi, binning, size)` and `gains_on(origin, step, size)`, `1 / flat` for a frame or a preview |
| `shading::Estimator` | `new(dark)`, `add(&frame)`, `frames()`, `finish(area, sensor)`: the retrospective flat, of 5 frames or more |
| `shading::FlatFieldView::new(&meta, dark, flat)` | The correction of frames of `meta`'s region and binning; `read(&frame, max)` gives a frame corrected. A dark of another binning is applied as its level |
| `shading::ShadingRecord` | `id`, `kind` (`dark`, `flat`), `objective`, `channel`, `created`, `method` (`capture`, `estimate`), `frames`, `exposure_ms`, `binning`, `level`, `flat`, `range`; `new(key, &meta, frames, method)`, `with_flat(flat, level)` |
| `ShadingRegistry::save_dark(key, &dark, &meta, frames)`, `dark(&record)` | A dark kept with its `.npy`, and read back |

## Examples

None of its own: `siq-server`'s `calibrate_sim` runs the whole stage–camera calibration, and
`siq-jobs`'s `estimate_flat` a tile scan's flat.

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

`tests/shading.rs`, on frames the simulator renders with its cos⁴ vignetting (corners at 0.8),
offset, per-pixel gains and offsets, and noise:

- a flat of a blank slide less its dark, within 0.11 % of the vignetting, and a frame corrected
  through `FlatFieldView` flat within 1 %;
- a region's and a binning's gains at their place on the sensor;
- a 6 × 5 tile scan of the cells world: its estimated flat within 1.04 % of the vignetting, and
  its seams, against the tiles without vignetting, from 9.9 % to 0.05 %; too few frames, and a
  frame of another size, refused;
- darks and flats kept per objective and channel, the dark's `.npy` read back.

`siq-server`'s `calibration` and `shading` tests run them on the simulated instrument.

## Limitations

- **Linear only:** lens distortion isn't modelled. Over the moves of a calibration, its residual
  shows in the fit's residuals.
- **One calibration per objective:** binning and the region of interest are accounted for in the
  shifts, so a calibration holds at any of them; a different camera or adapter needs a new one.
- **The stage's precision bounds it:** the rotation's error is about the stage's repeatability over
  the moves' reach. A small field calibrates less precisely than a large one.
- **The flat is smooth by design:** dust on the sensor, and each pixel's own gain, stay
  uncorrected.
- **The estimate needs variety:** five tiles or more, of different places, with the sample sparse
  enough that most cells show background in most tiles. Its darkfield isn't estimated: the frames
  are measured less a given dark level.
- **A dark holds for its binning and exposure.** At another binning its level is used; at another
  exposure, dark current isn't scaled.
