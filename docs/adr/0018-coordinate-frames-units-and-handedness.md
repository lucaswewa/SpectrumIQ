# ADR-0018: Coordinate frames, units and handedness

- **Status:** Accepted
- **Date:** 2026-10-23
- **Phase:** P0.2
- **Related:** ADR-0005, ADR-0013, REQ-ACQ-004, REQ-CAL-001, REQ-CAL-003

## Context

Almost every crate maps between stage positions, sensor pixels, the pixels of a frame read out
with a ROI and binning, mosaic pixels, and depth. The classic bugs are:

- sign flips between axes;
- half-pixel offsets between conventions;
- binning applied about the wrong origin;
- mixing mm and µm.

`siq-types` fixes one convention for all of them, before any crate depends on another.

## Decision

### Units

| Quantity | Type | Unit |
|---|---|---|
| Positions, distances | `Micrometers` | µm. `Millimeters` only at boundaries such as G-code; `Nanometers` for wavelengths |
| Durations (exposure, settle) | `Millis` | ms, with conversion to and from `std::time::Duration` |
| Rates | `Hertz` | Hz |
| Instants | `Timestamp` | ns since the epoch of the service's single monotonic `Clock` |
| Device instants | `DeviceMicros` | µs of the camera's or grabber's own clock. Not comparable with `Timestamp` |
| Angles | `f64` | radians |
| Gain | `f64` | linear factor; 1 is unity |

Units are `f64` newtypes. They serialize as bare numbers, and their JSON Schema description names
the unit.

### Stage frame

- **Axes:** µm. `x` and `y` lie in the sample plane, `z` along the optical axis.
- **Handedness:** the frame is right-handed, with **`+z` toward the objective**. `+z_coarse` and
  `+focus` both bring the sample and the objective closer.
- **Drivers** map their controller's axes (sign, offset, steps per µm) into this frame, from
  configuration.

**Sample coordinates are stage coordinates.** A sample point's coordinates are the stage XY reading
that puts it on the optical axis, at the centre of the sensor. Saved positions, scan regions,
tile poses and mosaic placement all use them.

### Sensor and image frames

- **Sensor frame:** unbinned sensor pixels.
  - The origin is the top-left corner of the top-left pixel; `x` runs right (columns) and `y`
    down (rows).
  - Pixel `(i, j)` covers `[i, i+1) × [j, j+1)`, and **its centre is `(i + 0.5, j + 0.5)`**.
- **Image frame** (a delivered frame): the same convention, over the frame's ROI after binning.

  ```text
  image_px = (sensor_px − roi_offset) / binning      (per axis)
  ```

  With corner origins, a binned pixel's centre maps to the centre of the sensor block it combines,
  and GPU texture sampling (wgpu) uses the same convention.

### Calibration

- **What it stores:** per objective, the linear map `L` from stage µm to unbinned sensor pixels, in
  px per µm. It includes the rotation, the shear, and the reflection when the camera's `y` runs
  against the stage's (then `det L < 0`). A stored calibration's translation is ignored.
- **The mapping.** For a frame taken with the stage at `P`, a sample point `S` appears at:

  ```text
  sensor_px = c + L·(S − P)          (c: the optical axis on the sensor, normally (W/2, H/2))
  ```

  `StageToPixel::for_frame(L, P, c, roi, binning)` implements this, composed with the image-frame
  step. `PixelToStage` is its inverse.
- **Moving the stage** by `ΔP` moves a sample feature by `−L·ΔP` on the sensor.

### Affine transforms

- `Affine2` is `p' = A·p + t` on column vectors. It serializes as `[[a, b, tx], [c, d, ty]]`.
- **Parameters:** `A = R(θ) · Shear(k) · Scale(sx, sy)`, where
  - `R(θ) = [[cos θ, −sin θ], [sin θ, cos θ]]`;
  - `Shear(k) = [[1, k], [0, 1]]`.
- **`decompose()`** always gives `sx > 0`. A reflection shows as `sy < 0`.
- **`compose(inner)`** is `self ∘ inner`: `inner` applies first.

### Mosaics and 3D

- **Mosaic level 0** has the image frame's orientation (`x` right, `y` down), so mosaics look like
  camera images. Level `k` is level 0 scaled by `2^-k`. Stage → mosaic px is a `StageToPixel`
  with the mosaic's own scale and origin, which the stitching ADRs (M7, M8) define.
- **Height maps and volumes** use the stage frame's `z`. A height is the focus position of best
  focus, so a higher surface has a larger height.

### Time and ids

- **One clock.** A service shares one `Arc<dyn Clock>`, and `Clock::utc` maps its timestamps to
  wall time. Tests use `ManualClock`.
- **Frame timestamps:** `FrameMeta.host_ts_mid_exposure` is the middle of the exposure.
- **`DatasetId` and `CalibrationId`** are UUID v7, ordered by creation within a process. `short()`
  is the last 8 hex digits, which are random; the first are the timestamp.
- **`ChannelId` and `ObjectiveId`** are the numbers the instrument configuration assigns, not list
  positions, so reordering the configuration keeps them.
- **`FrameMeta` holds no heap data** and is `Copy`, which the zero-allocation frame path needs.

## Alternatives considered

- **Pixel centres at integer coordinates (OpenCV):**
  - binned pixel centres then sit at fractional sensor coordinates that depend on the bin size;
  - it differs from GPU sampling by half a pixel.
- **A specimen frame that moves against the stage (`S = −P`):** puts a negation everywhere, while
  users and controllers think in stage readings.
- **mm internally:** convenient for G-code, but µm is microscopy's working unit, and avoids
  constant ×1000s.
- **Channel ids as names:** they would put a heap `String` in every frame's metadata.

## Consequences

- Drivers own the axis mapping. The calibration absorbs any image inversion of the optics.
- Every geometric computation goes through `Affine2`, `StageToPixel` or `PixelToStage`, so these
  conventions are tested in one place.
- `Clock::sleep_until` is added in P1.7, with its first user. The trait is extended then, before
  M1 ends.

## Verification

- `siq-types` affine tests:
  - `the_stage_position_maps_to_the_sensor_centre`;
  - `a_sample_point_maps_to_its_pixel_and_back`;
  - `moving_the_stage_moves_the_sample_the_other_way_in_the_image`;
  - property tests for inverse, composition and decomposition.
- `geometry::tests::pixel_centres_are_at_half_pixels`.
- The `affine_demo` example.
