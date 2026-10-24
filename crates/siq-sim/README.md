# siq-sim

SpectrumIQ's simulators. So far:

- **image formation:** a realistic 14-bit frame of a `siq-synth` world, for the true stage pose
  and focus, through an optics and a sensor model;
- **the true state** of the simulated instrument;
- **the simulated camera;**
- **a GRBL 1.1 emulator,** the device end of a serial link.

The focus and peripheral simulators (P1.10) follow.

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
  - exact timing on a `ManualClock`.
- **`GrblEmulator`,** the device end of a serial link:
  - GRBL 1.1's lines, realtime bytes, planner and alarms;
  - trapezoidal moves within each axis's rate and acceleration;
  - soft limits and homing;
  - the true stage moved with backlash, scale error and random error.

The design is in
[ADR-0026](../../docs/adr/0026-simulated-camera-fidelity-and-the-noiseless-cache.md) and its P1.7
amendment, and, for the emulator,
[ADR-0027](../../docs/adr/0027-protocol-level-emulation-and-the-serial-transport.md).

## Place in the architecture

- **Layer:** L2 Devices.
- **Depends on:**
  - `siq-synth` (worlds);
  - `siq-imgproc` (blur, enlarging, row bands);
  - `siq-devices` (the `Camera` trait);
  - `siq-buffer` (the pool);
  - `siq-config` (the sim camera, objectives and `[sim]` sections);
  - `siq-types`;
  - `tokio` (the emulator's I/O and timer).
- **Used by:**
  - P2.1's acquisition engine and P2.3's server, which choose `SimCamera` for `kind = "sim"`;
  - P1.8 and P1.10's emulators, which write the `TrueState`;
  - later `siq-datagen` and the tests of M5, M8 and M11–M13.
- **Phases:** P1.6 (`optics`), P1.7 (`state`, `camera`), P1.8 (`grbl`).

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
| `SimCameraSetup` | Sensor, format, full-frame rate, pixel pitch, `Lens`es by turret position, optics and sensor models, trigger latency. `from_config(&camera, &objectives, &sim)` applies the `fast` profile. `capabilities()` |
| `SimCamera::new(setup, world, state, clock)` | A `siq_devices::Camera` |

### `siq_sim::grbl`

| Item | What |
|---|---|
| `GrblEmulator::new(settings, errors, seed, state, clock)` | `run(port)` serves the device's end of a link until it closes |
| `GrblSettings` | GRBL 1.1's `$` numbers, with defaults for a 100 × 75 × 20 mm stage; `get`, `set` |
| `GrblErrors` | Backlash per axis, scale error, random error and reply latency. `From<&SimErrors>` |

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
| `sim_live` | The camera of `config/sim.toml` live, in real time on the `fast` profile, with a new pose every second: frames, rate, lost frames and mean per pose. 5 s; smoke mode 1 s at 512 × 384 | `cargo run --release -p siq-sim --example sim_live` |

## Features

None.

## Testing

`cargo test -p siq-sim` runs three binaries.

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
