# siq-sim

SpectrumIQ's simulators. So far, **image formation**: a realistic 14-bit frame of a `siq-synth`
world, for the true stage pose and focus, through an optics and a sensor model. The simulated
camera (P1.7), the GRBL emulator (P1.8) and the peripheral simulators (P1.10) follow.

- Defocus from the pose and the world's surface: a Gaussian whose σ comes from NA and λ. A relief
  is sharp only where it is in focus.
- Vignetting, PRNU, shot noise (Poisson below 20 e⁻), read noise, dark offset, DSNU, gain,
  quantization and clipping to the format.
- Counter-based noise: a frame repeats for its seed and number, and an ROI is a crop.
- A renderer with a noiseless cache: a still pose costs only its noise, and a moving one renders
  drafts.

The design is in
[ADR-0026](../../docs/adr/0026-simulated-camera-fidelity-and-the-noiseless-cache.md).

## Place in the architecture

- **Layer:** L2 Devices.
- **Depends on:**
  - `siq-synth` (worlds);
  - `siq-imgproc` (blur, enlarging, row bands);
  - `siq-devices` (`CameraSettings`);
  - `siq-types`.
- **Used by:** P1.7's simulated camera; later `siq-datagen` and the tests of M5, M8 and M11–M13.
- **Phase:** P1.6.

## API (`siq_sim::optics`)

| Item | What |
|---|---|
| `Shot { pose, calibration, sensor, settings }` | One frame: the true pose, the stage-to-sensor map (px per µm), the whole sensor's size, and the `CameraSettings` (ROI, binning, format, exposure, gain) |
| `OpticsModel` | NA, immersion index, wavelength, `Vignetting::{None, Cos4 { corner }, Polynomial { a, b }}`, and the flux (e⁻ per ms at intensity 1). `blur(defocus)` gives the σ on the sample |
| `SensorModel` | Fixed-pattern seed, e⁻ per ADU, read noise, dark offset, DSNU and PRNU |
| `render_signal(world, optics, sensor, shot, detail, &mut Scratch)` → `Rendered` | The noiseless signal, in `Scratch::signal()` |
| `digitize(signal, optics, sensor, shot, seq, out)` | Noise, offsets, gain, quantization and clipping, into a `u16` frame |
| `form_image(world, optics, sensor, shot, seq, &mut Scratch, out)` | Both, at full detail |
| `Renderer::new(world, optics, sensor)` | `render(shot, seq, out)`: noise only on a still pose, a draft of about 6 M samples on a new one, full detail when it holds. `set_draft_samples`, `optics_mut` |
| `Rendered` | The detail used, the blur range in frame pixels, and whether the signal was cached |

```rust
let mut renderer = Renderer::new(world, OpticsModel::default(), SensorModel::default());
let shot = Shot { pose: true_state.pose(), calibration, sensor: (11_648, 8_742), settings };
let rendered = renderer.render(&shot, seq, &mut slot.image_mut::<u16>(width, height)?)?;
```

## Examples

| Example | What it shows | Run |
|---|---|---|
| `render_frame` | Renders 100 MP frames of cells through a 10× objective: drafts, full detail and noise only, on the sensor and on the `fast` profile, with their times. It writes the last as `frame.raw` (Mono14 in little-endian `u16`) plus a PNG preview. Smoke mode uses a 1 MP sensor | `cargo run --release -p siq-sim --example render_frame [-- <x> <y> <focus> [<folder>]]` |

## Features

None.

## Testing

`cargo test -p siq-sim` runs the `optics` binary:

- determinism by seed and frame;
- sharpness falling with the defocus, either way;
- the vignetting profile;
- the noise's mean and variance against Poisson plus read noise, and P(0) of a Poisson;
- ROI as crop and binning as sum;
- clipping to Mono14, Mono12 and Mono8;
- a relief blurred only where it is out of focus;
- the renderer's drafts and cache.

## Limitations

- **A full-detail frame after a stop takes 1.6 s at 100 MP.** Still and moving frames meet their
  targets (ADR-0026).
- **About 1.2 GB of buffers** at 100 MP.
- **Volumes are rendered with their surface at 0.**
- **A binned pixel takes its first pixel's PRNU and DSNU.**
