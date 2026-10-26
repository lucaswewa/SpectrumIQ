# siq-workflow

Workflows: what the instrument does by itself, move by move and frame by frame, on the device
traits. Its first module, from P5.2, holds the routines; the engine that runs scans, z-stacks and
time-lapses is M6's.

- **`routines::stage_camera`** (ADR-0049): the measurement of the stage–camera calibration.
  1. A frame where the stage is, the reference.
  2. The stage moved to each of the plan's places about it, each approached from the same side so
     that the backlash is the same at every arrival; a frame at each, registered against the
     reference by [`siq-register`](../siq-register/README.md).
  3. One place approached from either side, a few times over, for the backlash.
  4. Back where it started.

  Shifts are in sensor pixels: a binned frame's are scaled up. Registrations run on blocking
  threads, as one of a 100 MP frame takes a good part of a second. A registration whose
  peak-to-correlation energy is under the plan's least stops the routine: the field is bare, or the
  move left it.
- **`routines::shading`** (ADR-0050): the frames of the shading, summed, with the settings they
  were taken at.
  - **The dark:** every light off, the frames taken, then each light back at its level. Without
    the lights' device, the light path is blocked by hand.
  - **The flat:** the focus moved away by the plan's defocus (0 for a blank slide), frames at nine
    places a field apart about the stage, so that what is in the field averages away; then the
    stage and the focus put back, whether it worked or not.

## Place in the architecture

- **Layer:** L4 Engines.
- **Depends on:** `siq-acquire`, `siq-calib`, `siq-devices`, `siq-register`, `siq-types`;
  `thiserror`, `tokio`, `tracing`.
- **Used by:** `siq-server` (the `calibration` Thing).
- **Phase:** P5.2; P5.3 the shading's captures; the engine in M6.

## API

| Item | What |
|---|---|
| `routines::stage_camera::StageCameraPlan` | `moves` (µm from where the stage is), `backlash` (`Option<BacklashPlan>`), `min_pce` |
| `StageCameraPlan::for_field([width, height])` | For a field in µm: a star of 16 places, eight at 0.4 of the field each way and eight at 0.3, on the axes and diagonals; the backlash at the start, from 50 µm either side, four times; a least PCE of 100 |
| `BacklashPlan { at, reach, repeats }` | Where the backlash is measured, how far beyond it the approaches start, and how many times |
| `measure(&acquisition, &stage, &registrar, &plan)` | `Measurement { moves, pce, backlash_shift }`, the moves ready for `siq_calib::fit`, or `RoutineError` (`Device`, `Register`, `Featureless`, `Format`, `Failed`, `Calib`) |
| `routines::shading::capture_dark(&acquisition, lights, frames)` | `Captured { sum, meta }`: the frames' `Accumulator`, and the last frame's metadata |
| `routines::shading::FlatPlan { places, frames, defocus }` | `for_field(field, defocus)`: a 3 × 3 grid a field apart, two frames at each |
| `routines::shading::capture_flat(&acquisition, &stage, &focus, &plan)` | `Captured`, of every place |

**Why the moves reach 0.4 of the field:** the stage's unrepeatability, about 0.5 µm, is the
calibration's main error, and it weighs less the farther the moves go. At 0.4 a diagonal move
still leaves a third of the frame to register on.

## Examples

None of its own: `siq-server`'s `calibrate_sim` runs the stage–camera routine through the
`calibration` Thing.

## Features

None.

## Testing

Through `siq-server`'s tests, on the simulated instrument:

- `calibration`: the camera turned 0.4°, with 0.2 % more pixels per µm and a shear of 0.001, the
  stage with its backlash, scale error, whole steps and noise, binned 2 × 2;
- `shading`: a blank slide's dark, the lights back as they were, and its flat 50 µm out of focus,
  the focus back where it was.

## Limitations

- **Something must be in the field:** a bare slide stops the stage–camera routine as
  `Featureless`.
- **A flat out of focus is only as flat as the sample is uniform:** a sample with structure larger
  than the blur and the nine places' spread leaves it in the flat. A blank slide is the reliable
  target.
- **No settling time of its own:** a move is done when the stage says so. A stage that rings after
  it stops needs its driver to wait.
- **Sequential:** each frame is taken and registered before the next move. Moving while the last
  frame registers would save a fraction of the time.
