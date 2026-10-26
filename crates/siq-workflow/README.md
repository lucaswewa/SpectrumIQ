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

## Place in the architecture

- **Layer:** L4 Engines.
- **Depends on:** `siq-acquire`, `siq-calib`, `siq-devices`, `siq-register`, `siq-types`;
  `thiserror`, `tokio`, `tracing`.
- **Used by:** `siq-server` (the `calibration` Thing).
- **Phase:** P5.2; the engine in M6.

## API

| Item | What |
|---|---|
| `routines::stage_camera::StageCameraPlan` | `moves` (µm from where the stage is), `backlash` (`Option<BacklashPlan>`), `min_pce` |
| `StageCameraPlan::for_field([width, height])` | For a field in µm: a star of 16 places, eight at 0.4 of the field each way and eight at 0.3, on the axes and diagonals; the backlash at the start, from 50 µm either side, four times; a least PCE of 100 |
| `BacklashPlan { at, reach, repeats }` | Where the backlash is measured, how far beyond it the approaches start, and how many times |
| `measure(&acquisition, &stage, &registrar, &plan)` | `Measurement { moves, pce, backlash_shift }`, the moves ready for `siq_calib::fit`, or `RoutineError` (`Device`, `Register`, `Featureless`, `Format`, `Failed`) |

**Why the moves reach 0.4 of the field:** the stage's unrepeatability, about 0.5 µm, is the
calibration's main error, and it weighs less the farther the moves go. At 0.4 a diagonal move
still leaves a third of the frame to register on.

## Examples

None of its own: `siq-server`'s `calibrate_sim` runs the routine through the `calibration`
Thing.

## Features

None.

## Testing

Through `siq-server`'s `calibration` test, on the simulated instrument: the camera turned 0.4°,
with 0.2 % more pixels per µm and a shear of 0.001, the stage with its backlash, scale error,
whole steps and noise, binned 2 × 2.

## Limitations

- **Something must be in the field:** a bare slide stops the routine as `Featureless`.
- **No settling time of its own:** a move is done when the stage says so. A stage that rings after
  it stops needs its driver to wait.
- **Sequential:** each frame is taken and registered before the next move. Moving while the last
  frame registers would save a fraction of the time.
