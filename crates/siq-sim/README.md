# siq-sim

SpectrumIQ's simulators:

- **image formation:** a realistic 14-bit frame of a `siq-synth` world, for the true stage pose
  and focus, through an optics and a sensor model;
- **the true state** of the simulated instrument;
- **the simulated camera;**
- **a GRBL 1.1 emulator,** the device end of a serial link;
- **a dialect emulator,** a focus drive made from a `siq-ascii` dialect file;
- **the peripheral simulators:** illumination, the filter wheel, the objective turret and the
  modality unit.

Together, with the real drivers, they make the simulated rig of `config/sim.toml` (the `sim_rig`
example).

What these give:

- **Defocus** from the pose and the world's surface: a Gaussian whose σ comes from NA and λ. A
  relief is sharp only where it is in focus.
- **The sensor:** vignetting, PRNU, shot noise (Poisson below 20 e⁻), read noise, dark offset,
  DSNU, gain, quantization and clipping to the format.
- **Counter-based noise:** a frame repeats for its seed and number, and an ROI is a crop.
- **A renderer with a noiseless cache:** a still pose costs only its noise, and a moving one
  renders drafts.
- **`SimCamera`,** a `Camera` that renders into pool slots on its own thread:
  - frame rates from the capability model;
  - free-run and software trigger;
  - lost frames when no slot is free;
  - exact timing on a `ManualClock`;
  - the camera's mount, `[sim.mount]`: the image turned, scaled and sheared against the stage, as
    the stage–camera calibration finds it (P5.2).
- **`GrblEmulator`,** the device end of a serial link:
  - GRBL 1.1's lines, realtime bytes, planner and alarms;
  - trapezoidal moves within each axis's rate and acceleration;
  - soft limits and homing;
  - the true stage moved in whole steps, with backlash, scale error and random error.
- **`DialectEmulator`,** the device end of a serial link in any dialect:
  - its commands answered with their replies, and other lines with its errors;
  - moves at their speed, jogs to the edge of the travel, and homing;
  - a completion reply, for dialects that wait for one;
  - the true focus moved.
- **The peripheral simulators,** behind their `siq-devices` traits: each change takes a realistic
  time on the clock, then sets the true state.

The design is in
[ADR-0026](../../docs/adr/0026-simulated-camera-fidelity-and-the-noiseless-cache.md) and its P1.7
amendment. For the emulators, it is in
[ADR-0027](../../docs/adr/0027-protocol-level-emulation-and-the-serial-transport.md) and
[ADR-0030](../../docs/adr/0030-data-driven-ascii-dialects.md). For the peripherals and the rig, it is
in [ADR-0012](../../docs/adr/0012-simulation-strategy.md)'s P1.10b amendment.

## Place in the architecture

- **Layer:** L2 Devices.
- **Depends on:**
  - `siq-synth` (worlds);
  - `siq-imgproc` (blur, enlarging, row bands);
  - `siq-devices` (the `Camera` and peripheral traits);
  - `siq-buffer` (the pool);
  - `siq-config` (the sim camera, objectives and `[sim]` sections);
  - `siq-ascii` (the dialects);
  - `siq-types`;
  - `tokio` (the emulators' I/O and timers, and the peripherals' turns).
- **Dev-dependencies:** `siq-grbl` and `siq-serial`, for the tests and `sim_rig`.
- **Used by:**
  - P2.1's acquisition engine, which runs on `SimCamera`;
  - P2.3's server, which chooses `SimCamera`, the emulators and the peripheral simulators for
    `kind = "sim"`;
  - the drivers' tests, in `siq-grbl` and `siq-ascii`;
  - later `siq-datagen` and the tests of M5, M8 and M11–M13.
- **Phases:** P1.6 (`optics`), P1.7 (`state`, `camera`), P1.8 (`grbl`), P1.10a (`dialect`),
  P1.10b (`peripherals`); P5.2 the camera's mount.

## API

### `siq_sim::optics`

| Item | What |
|---|---|
| `Shot { pose, calibration, sensor, settings }` | One frame: the true pose, the stage-to-sensor map (px per µm), the whole sensor's size, and the `CameraSettings` (ROI, binning, format, exposure, gain) |
| `OpticsModel` | NA, immersion index, wavelength, `Vignetting::{None, Cos4 { corner }, Polynomial { a, b }}`, and the flux (e⁻ per ms at intensity 1). `blur(defocus)` gives the σ on the sample |
| `SensorModel` | Fixed-pattern seed, e⁻ per ADU, read noise, dark offset, DSNU and PRNU |
| `render_signal(world, optics, sensor, shot, detail, &mut Scratch)` → `Rendered` | The noiseless signal, in `Scratch::signal()` |
| `digitize(signal, optics, sensor, shot, seq, out)` | Noise, offsets, gain, quantization and clipping, into a `u16` frame |
| `form_image(world, optics, sensor, shot, seq, &mut Scratch, out)` | Both, at full detail |
| `Renderer::new(world, optics, sensor)` | `render(shot, seq, out)`: noise only on a still pose, a draft of about 6 M samples on a new one, full detail when it holds. `set_draft_samples`, `set_flux` (keeps the signal), `optics_mut` |
| `Rendered` | The detail used, the blur range in frame pixels, and whether the signal was cached |

### `siq_sim::state`

| Item | What |
|---|---|
| `TrueState::new(sources)` | `pose`, `set_pose`, `set_stage`, `set_focus`; `level` and `set_level` per light source, and `light()`; `filter`, `objective` and `modality`, each with its setter. Reads take no lock |

### `siq_sim::camera`

| Item | What |
|---|---|
| `SimCameraSetup` | Sensor, format, full-frame rate, pixel pitch, `Lens`es by turret position, the `mount` (an `Affine2`), optics and sensor models, trigger latency. `from_config(&camera, &objectives, &sim)` applies the `fast` profile. `capabilities()` |
| `SimCamera::new(setup, world, state, clock)` | A `siq_devices::Camera` |

### `siq_sim::grbl`

| Item | What |
|---|---|
| `GrblEmulator::new(settings, errors, seed, state, clock)` | `run(port)` serves the device's end of a link until it closes |
| `GrblSettings` | GRBL 1.1's `$` numbers, with defaults for a 100 × 75 × 20 mm stage; `get`, `set` |
| `GrblErrors` | Backlash per axis, scale error, random error and reply latency. `From<&SimErrors>` |

### `siq_sim::dialect`

| Item | What |
|---|---|
| `DialectEmulator::new(dialect, state, clock)` | A drive at the bottom of its travel, not homed. `run(port)` serves the device's end of a link until it closes. It identifies as `SIM-FOCUS 1.0 0001` |

### `siq_sim::peripherals`

| Item | What | Time per change |
|---|---|---|
| `SimIllumination::new(sources, state, clock)` | An `Illumination`, at the true state's levels | 5 ms |
| `SimChanger::filter_wheel(filters, state, clock)` | A `FilterWheel` that moves the true filter. Nothing is in the path while it moves | 60 ms a position, the shorter way round |
| `SimChanger::objective_turret(positions, state, clock)` | An `ObjectiveTurret` of positions named `position 0` and on, which moves the true objective | 400 ms a position, the shorter way round |
| `SimModalityUnit::new(modes, patterns, state, clock)` | A `ModalityUnit`, starting in its first mode, which sets the true modality | 300 ms a mode, 20 ms a pattern |

```rust
let setup = SimCameraSetup::from_config(&sim_camera, &config.objectives, &config.sim);
let world = WorldInfo::from_config(&config.sim.world, config.sim.seed).build()?;
let camera = SimCamera::new(setup, world.into(), true_state.clone(), clock.clone());
camera.start(pool, acquisition_sink).await?;  // frames at the model's rate, into the sink
```

## Examples

| Example | What it shows | Run |
|---|---|---|
| `render_frame` | Renders 100 MP frames of cells through a 10× objective: drafts, full detail and noise only, on the sensor and on the `fast` profile, with their times. It writes the last as `frame.raw` (Mono14 in little-endian `u16`) plus a PNG preview. Smoke mode uses a 1 MP sensor | `cargo run --release -p siq-sim --example render_frame [-- <x> <y> <focus> [<folder>]]` |
| `grbl_emulator_repl` | A scripted session with the emulator in real time: unlock, settings, a move watched by status reports, a dwell, a jog cancelled, a feed hold, a reset in motion, and the true stage's errors. About 3 s | `cargo run -p siq-sim --example grbl_emulator_repl` |
| `dialect_emulator_repl` | A scripted session with the dialect emulator in real time, in the example dialect: identification, a move watched by status queries, a stop, a jog, homing and errors; then a move that ends with the completion reply. About 3 s | `cargo run -p siq-sim --example dialect_emulator_repl` |
| `sim_rig` | M1's simulated rig, from `config/sim.toml`: the camera on the `fast` profile, the GRBL driver and the ASCII focus driver against their emulators, and the peripheral simulators. It sets up the first channel, homes, moves the stage and focus, and prints each frame's mean and sharpness with the reported and true poses. About 3 s; smoke mode uses a 1024 × 768 sensor | `cargo run --release -p siq-sim --example sim_rig` |
| `sim_live` | The camera of `config/sim.toml` live, in real time on the `fast` profile, with a new pose every second: frames, rate, lost frames and mean per pose. 5 s; smoke mode 1 s at 512 × 384 | `cargo run --release -p siq-sim --example sim_live` |

## Features

None.

## Testing

`cargo test -p siq-sim` runs five binaries.

**`optics`:**

- determinism by seed and frame;
- sharpness falling with the defocus, either way;
- the vignetting profile;
- the noise's mean and variance against Poisson plus read noise, and P(0) of a Poisson;
- ROI as crop and binning as sum;
- clipping to Mono14, Mono12 and Mono8;
- a relief blurred only where it is out of focus;
- the renderer's drafts and cache.

**`camera`,** on a `ManualClock`:

- the free-run rate, for the full frame and an ROI;
- trigger latency;
- frames lost while consumers hold every slot;
- a settings change at the next frame;
- the camera following the true state's light and pose;
- the pose's sequence lock under concurrent writes and reads.

**`grbl`,** through a `DuplexTransport` on a `ManualClock`:

- four golden transcripts in `tests/transcripts/`: the protocol, motion and dwells, alarms and
  homing, and hold, resume and jogs;
- a diagonal move's timing;
- the true stage's backlash, scale error and random error.

**`dialect`,** through a `DuplexTransport` on a `ManualClock`, in the example dialect:

- commands, replies and errors;
- moves, a stop, a jog to the edge of the travel and homing, with the true focus;
- the completion reply, for a move and homing but not a stop or a jog.

**`peripherals`,** on a `ManualClock` that the test moves, so each change takes exactly its time:

- a light settling;
- the filter wheel the shorter way round, with nothing in the path meanwhile;
- the turret's 400 ms a position;
- the modality unit's mode and pattern times, and what it refuses.

## Limitations

- **A full-detail frame after a stop takes 1.6 s at 100 MP,** and slips the camera's schedule by
  that much. Still and moving frames meet their targets (ADR-0026).
- **About 1.2 GB of buffers** at 100 MP.
- **Volumes are rendered with their surface at 0.**
- **A binned pixel takes its first pixel's PRNU and DSNU.**
- **The pose is read when a frame is rendered,** not at its mid-exposure. A moving stage's frame
  is rendered at the pose of its readout.
- **There is no trigger line:** `trigger()` stands in for an external trigger too.
- **The emulator's hold and jog cancel stop at once,** and its blocks start and stop at rest.
- **The dialect emulator moves without acceleration,** replies at once, and has no positioning
  error.
