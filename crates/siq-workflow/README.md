# siq-workflow

Workflows: what the instrument does by itself, move by move and frame by frame, on the device
traits: the engine that runs acquisition plans, and the routines that measure the instrument.

- **`engine`** (ADR-0052): one executor for every acquisition plan.
  - **A `Plan`** is its dataset's name and steps: `move_to`, `focus`, `channel`, `snap` and
    `wait`; `repeat`, which numbers the frames' `t`, `z` or `p` within it; `index`, which sets one
    for the steps after it; and `policy`, steps with an error policy of their own. It is serde,
    with its JSON Schema.
  - **`run`** makes the plan's dataset, with the plan, the instrument and the calibration in force
    in its manifest, and records each snap there under the key of the indices and the channel.
    It reports progress and events, waits while paused, and stops when cancelled: a move under
    way is abandoned, a capture finishes, then the stage and the focus are halted. A failed step
    is tried again, skipped or the run's end, by its policy.
- **`DeviceSet`:** the instrument as the workflows see it, shared handles to each device, the
  acquisition, the channels and the calibrations.

- **`routines::stage_camera`** (ADR-0049): the measurement of the stage–camera calibration.
  1. A frame where the stage is, the reference.
  2. The stage moved to each of the plan's places about it, each approached from the same side so
     that the backlash is the same at every arrival; a frame at each, registered against the
     reference by [`siq-register`](../siq-register/README.md).
  3. One place approached from either side, a few times over, for the backlash.
  4. Back where it started, whether it worked or not.

  The places are about where the stage is or, if they would leave its travel there, about the
  nearest place where they don't. The flat's places are kept within it in the same way.

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
- **`routines::channel`** (ADR-0035): a channel put in place, in order, each step waiting for its
  device: every light off, the filter, the exposure, then the channel's light. The engine and the
  server's `imaging` Thing share it.

## Place in the architecture

- **Layer:** L4 Engines.
- **Depends on:** `siq-acquire`, `siq-buffer`, `siq-calib`, `siq-config`, `siq-devices`,
  `siq-register`, `siq-storage`, `siq-types`; `schemars`, `serde`, `serde_json`, `thiserror`,
  `tokio`, `tracing`.
- **Used by:** `siq-server` (the `calibration` and `imaging` Things; the `workflow` Thing in
  P6.5).
- **Phase:** P5.2; P5.3 the shading's captures; P6.1 the engine and the channel routine.

## API

| Item | What |
|---|---|
| `engine::Plan { name, steps, on_error }` | A plan; `engine::count(&steps)`, its device steps |
| `engine::Step` | `MoveTo { x, y, z? }`, `Focus { z }`, `Channel { id }`, `Snap`, `Wait { ms }`, `Repeat { count, index?, steps }`, `Index { index, value }`, `Policy { on_error, steps }` |
| `engine::ErrorPolicy { retries, skip }` | A failed step tried again `retries` times, then skipped if `skip`, else the run's end |
| `engine::run(&devices, &target, &plan, &control, &observe)` | `RunReport { dataset, path, outcome, error, steps, frames }`, or `RunError` (`Invalid`, `Storage`) before any dataset is made |
| `engine::DatasetTarget` | `data_root`, `min_free`, `catalog`, and the `instrument`'s snapshot |
| `engine::RunControl` | `pause`, `resume`, `cancel`, `command`, `cancelled`; its clones share it |
| `engine::RunEvent` | `Progress { progress }` (`done`, `total`, `eta_s`, `step`), `Frame { t, c, z, p }`, `Paused`, `Resumed`, `Problem { step, attempt, error, then }` |
| `engine::Outcome` | `Finished`, `Cancelled`, `Failed` |
| `DeviceSet` | `acquisition`, `camera`, `stage`, `focus`, `illumination`, `filter_wheel`, `turret`, `modality`, `channels`, `calibrations`, `clock` |
| `routines::channel::apply(&devices, &channel)` | The channel in place; `until_frames_begin_after(&camera)`, half an exposure |
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

| Example | What it shows | Run |
|---|---|---|
| `run_plan_sim` | A plan on the simulated instrument: twice, at three places, a frame in brightfield and in DAPI, through the GRBL driver and the simulated lights and filter wheel; each step as it is done, then the dataset's frames and where they were taken | `cargo run --release -p siq-workflow --example run_plan_sim [-- --out <data root>]` |

Every example also runs in smoke mode (`SIQ_SMOKE=1`) in `cargo xtask ci`. `siq-server`'s
`calibrate_sim` runs the stage–camera routine through the `calibration` Thing.

## Features

None.

## Testing

`cargo test -p siq-workflow` runs `engine`, on the simulated camera at 128 × 96 with mock motion
and peripherals:

- steps in order, and frames keyed by their indices, a repeat's scoped and a set one held;
- a paused run waiting before its next step, then going on;
- a cancelled run abandoning its move, the stage halted, its frames kept;
- a capture under way finishing before a cancelled run stops, and the camera ready after;
- the error policies: tried again, skipped, the run's end, and a group's own;
- the manifest's plan, instrument and calibration; a plan refused before any dataset, and a key
  taken twice.

The routines run through `siq-server`'s tests, on the simulated instrument:

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
  frame registers would save a fraction of the time. The engine, too, waits for each step.
- **A plan's keys must be unique:** a plan that snaps twice at one place without numbering the
  snaps stops at the second.
