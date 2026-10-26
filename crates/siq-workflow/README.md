# siq-workflow

Workflows: what the instrument does by itself, move by move and frame by frame, on the device
traits: the engine that runs acquisition plans, autofocus, and the routines that measure the
instrument.

- **`engine`** (ADR-0052): one executor for every acquisition plan.
  - **A `Plan`** is its dataset's name and steps: `move_to`, `focus`, `channel`, `snap` and
    `wait`; `autofocus`, `focus_map`, which makes the focus surface in force, and
    `focus_on_surface`; `repeat`, which numbers the frames' `t`, `z` or `p` within it; `index`,
    which sets one for the steps after it; `policy`, steps with an error policy of their own; and
    `tile_scan`, made into its moves, focusing and snaps when the run starts; and the sequences,
    `z_stack`, `multi_position` and `time_lapse`. It is serde, with its JSON Schema.
  - **`run`** makes the plan's dataset, with the plan, the instrument and the calibration in force
    in its manifest, and records each snap there under the key of the indices and the channel.
    It reports progress and events, waits while paused, and stops when cancelled: a move under
    way is abandoned, a capture finishes, then the stage and the focus are halted. A failed step
    is tried again, skipped or the run's end, by its policy.
- **`scan`** (ADR-0053): tile scans planned and estimated.
  - **The grid runs along the camera's axes,** by the stage–camera calibration: neighbours overlap
    by exactly the share of the frame asked, whatever the camera's rotation.
  - **The tiles kept** are those whose frame meets the region, a rectangle or a polygon.
  - **The order:** a snake, a raster, or the nearest tile next.
  - **The estimate:** the moves at the stage's speed and acceleration, settling, channel
    switches, exposures and readouts, and the bytes, against the disk.
  - **In a plan,** a `tile_scan` needs the calibration in force. Its grid goes in the dataset's
    `layout.json`, each tile's column, row and planned position. Each tile is focused as the
    focus is, at a fixed focus, on the surface in force, or by autofocus.
- **Sequences** (ADR-0055), steps that hold steps, in any order:
  - **`z_stack`:** planes about where the focus is, `z` numbered from the bottom, each approached
    from below when it goes up; then the focus back to the middle. A tile scan may take one at
    each tile.
  - **`multi_position`:** at each position, the stage there, its own focus, an optional autofocus,
    then the steps, whose places are about the position.
  - **`time_lapse`:** points `interval_s` apart from the first, on tokio's clock. A late point runs
    within its own interval or is skipped (`skip`), or slides the schedule (`shift`).
  - **`p`** numbers every position and tile of a plan in order, so scans don't collide.
    `layout.json` holds the grids with their first `p`, the positions, the focus maps, and each
    time-lapse's points, due and begun, and those skipped.
- **`autofocus`** (ADR-0054): focus found where the stage is, and focus surfaces.
  - **Two sweeps of the focus drive,** a frame at each step, scored by Tenengrad over the frame's
    middle three quarters: a coarse one over the range, binned 2 × 2 at a quarter of the exposure,
    then a fine one at the camera's own resolution about the coarse one's estimate. Each estimate
    is the vertex of a Gaussian fitted to the frames above half the sweep's peak.
  - **Nothing to focus on:** a coarse sweep whose sharpness rises by less than a fifth of its peak
    finds no focus, and the drive goes back where it was. As a step, it fails.
  - **The camera** is back at its own settings after, whatever happened.
  - **A focus map** autofocuses at each of its places, leaves out those with nothing to focus on,
    and fits a surface to the rest: a plane by least squares, places far off it set aside, or a
    thin-plate spline through them. Its residuals are each place's error, the spline's with the
    place left out. The maps go in `layout.json`.
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
  `siq-imgproc`, `siq-register`, `siq-storage`, `siq-types`; `schemars`, `serde`, `serde_json`,
  `thiserror`, `tokio`, `tracing`. Tokio's `test-util`, for the paused clock, in the tests.
- **Used by:** `siq-server` (the `calibration` and `imaging` Things; the `workflow` Thing in
  P6.5).
- **Phase:** P5.2; P5.3 the shading's captures; P6.1 the engine and the channel routine; P6.2
  the tile scans; P6.3 autofocus and focus maps; P6.4 the sequences.

## API

| Item | What |
|---|---|
| `engine::Plan { name, steps, on_error }` | A plan; `engine::count(&steps)`, its device steps |
| `engine::Step` | `MoveTo { x, y, z? }`, `Focus { z }`, `Channel { id }`, `Snap`, `Wait { ms }`, `Autofocus { range_um }`, `FocusMap { places, surface, range_um }`, `FocusOnSurface { x, y }`, `Repeat { count, index?, steps }`, `Index { index, value }`, `Policy { on_error, steps }`, `TileScan { region, overlap, order, channels, settle_ms, focus, z_stack? }`, `ZStack { planes, steps }`, `MultiPosition { positions, autofocus?, steps }`, `TimeLapse { interval_s, count?, duration_s?, overrun, steps }` |
| `engine::ZPlanes { range_um, step_um, direction }` | `planes()`: each plane's `z` and offset from the middle, in the order taken; `ZDirection::{Up, Down}`; `MAX_PLANES`, 1000 |
| `engine::Position { name, x, y, z?, focus? }` | A multi-position's position |
| `engine::Overrun` | `Skip` (the default) or `Shift` |
| `engine::Layout` | `layout.json`: `scans` (`ScanLayout { first_p, grid }`), `positions` (`Placed { p, position }`), `focus_maps`, `time_lapses` (`TimeLapseRecord { interval_s, overrun, points, skipped }`, each `TimePoint { t, due_s, began_s }`) |
| `engine::TileFocus` | A tile scan's focus: `None` (as it is), `Fixed { z }`, `Surface` (the focus map's), `Autofocus { range_um }` |
| `engine::FocusMap` | A map as made, in `layout.json`: `found` (`[x, y, z]`), `missed` (`[x, y]`), `fitted` |
| `autofocus::autofocus(&devices, &plan)` | `Focused { z, curve, rise }`: the focus, `None` with nothing to focus on; each frame's `[z, sharpness]`; the coarse sweep's rise, of its peak |
| `autofocus::AutofocusPlan` | `range_um`, `coarse`, `fine`, `min_rise`, `binning`; by default 50 µm, 11 and 13 frames, a fifth, 2 × 2 |
| `autofocus::fit_surface(&points, kind)` | `FittedSurface { surface, residuals, outliers }` through `[x, y, z]` points, three or more; `SurfaceKind::{Plane, ThinPlate}` |
| `autofocus::FocusSurface` | `Plane { coefficients }` or `ThinPlate { centre, scale, places, weights, plane }`; `z(x, y)` |
| `autofocus::depth_of_field(na, n, λ, pixel)` | Berek's, in µm: 7.37 for 10×/0.30 in green at 0.376 µm a pixel |
| `engine::resolve(&devices, &steps, &mut layout)` | The steps with each tile scan and multi-position made into their own, and their grids and positions in the `Layout`; `engine::LAYOUT`, `layout.json` |
| `scan::plan(&region, &l, field, overlap, order)` | `Grid { tiles, steps, overlap }`, each `Tile { column, row, x, y }`; `ScanError` for a region, overlap or `L` that can't make one, or over `MAX_TILES` |
| `scan::Region`, `scan::Order` | `Rect { x0, y0, x1, y1 }` or `Polygon { points }`; `Snake`, `Raster`, `Nearest` |
| `scan::estimate(&grid, start, settle_s, &exposures_s, frame_bytes, &timing)` | `Estimate { tiles, frames, seconds, bytes }`, and `fits(free, keep)`; `Timing`'s defaults are the simulator's |
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
| `autofocus_sim` | Autofocus at five places on a gentle relief, from up to 22 µm out (one, at 256 × 192, in smoke mode): each sweep's curve, the focus against the truth, and a thin-plate surface through them with each place's residual | `cargo run --release -p siq-workflow --example autofocus_sim` |
| `timelapse_sim` | A time-lapse of three points every 4 s, at two positions, a five-plane z-stack at each (two points of three planes in smoke mode): each frame as it comes, then each point's schedule | `cargo run --release -p siq-workflow --example timelapse_sim [-- --out <data root>]` |
| `tilescan_sim` | A 5 × 5 tile scan in brightfield and DAPI on the simulated instrument (3 × 3 in one channel, the `fast` profile, in smoke mode): its estimate, then its run | `cargo run --release -p siq-workflow --example tilescan_sim [-- --out <data root>]` |
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
  taken twice;
- on tokio's paused clock, a time-lapse's schedule through a pause, a cancel while it waits, and
  late points skipped or shifting the schedule;
- a multi-position's positions numbered, each at its focus, and a move about it.

And `scan`:

- as a property, a rectangle covered with the overlap asked, whatever the camera's rotation, each
  tile once;
- a concave polygon keeping only the tiles that meet it; the three orders;
- a 3 × 3 scan on the simulator, through the GRBL driver against its emulator: its dataset, its
  layout, each frame within 1 µm of its tile, and its time within 20 % of the estimate.

And `autofocus`, on the simulated instrument (`tests/common`, which the scan tests share: the
camera at 256 × 192 in brightfield, the GRBL emulator, the simulator's instant focus drive),
against the truth where the stage truly is:

- on grains over a tilted, curved surface, focus within a quarter of the depth of field from up
  to 20 µm out, at five places, the camera as it was;
- on a bare field, nothing to focus on, the focus back where it was, and the step failing; plans
  refused for an autofocus of no range, a map of two places, or a focus on the surface before any
  map;
- a plane exact, and a place 20 µm off it set aside; a thin-plate spline through its places and
  within 1 µm of a curved surface between them;
- a 5 × 5 scan with a 9-point focus map, every tile within a quarter of the depth of field.

And the sequences, on the same rig:

- a z-stack up and down: planes in order, numbered from the bottom, every move unidirectional;
- a time-lapse of two positions, each scanned in 2 × 1 tiles with three-plane stacks: every key
  once, the steps as counted, the tiles about each position;
- as a property, a stack's planes a step apart about the middle.

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
- **Autofocus finds the focus that sharpens the frame most:** where the sample's relief across
  the frame is more than the depth of field, that may not be the focus at its centre.
- **One sweep of the range:** a focus outside the range is found at its edge, not followed.
- **24 frames an autofocus,** each after the drive arrives: about 5 s at 512 × 384 on the
  simulator.
- **A cancel waits for an autofocus under way,** up to its 24 frames: stopped halfway, it would
  leave the camera binned.
- **A stack taken down moves twice a plane** on a drive that arrives from below.
- **A time-lapse's points are on tokio's clock,** which is the system's in a run: a suspended
  computer's sleep shows as lateness.
