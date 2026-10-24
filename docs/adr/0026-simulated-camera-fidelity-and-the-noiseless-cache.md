# ADR-0026: Simulated-camera fidelity versus performance, and the noiseless cache

- **Status:** Accepted
- **Date:** 2026-10-24
- **Phase:** P1.6
- **Related:** ADR-0012, ADR-0018, ADR-0024, ADR-0025, REQ-SIM-001, REQ-ACQ-003

## Context

- **The simulated camera must look real enough** to develop live view, autofocus, stitching and
  calibration against:
  - the stage pose and the focus decide what is seen, and how sharply;
  - vignetting, offsets, fixed pattern and noise must be in the frames;
  - the frames are 14-bit.
- **The targets of the brief** (P1.6):
  - stationary live view at 9 frames/s for 100 MP;
  - at least 3 frames/s for 100 MP while the stage moves;
  - at least 9 frames/s for the 25 MP `fast` profile.
- **A world costs 67–133 ns per sample** (ADR-0024). At full resolution, 100 MP is about
  1.5 s of all the cores.

## Decision

### Two stages: signal, then digitization

- **`render_signal`** makes the noiseless signal at the frame's size:
  - the world sampled on the frame's own pixel-to-stage mapping, with the pixel's footprint;
  - blurred by the defocus;
  - times the vignetting, each pixel's gain (PRNU) and the binned area.
- **`digitize`** turns the signal into a frame:
  - electrons: the signal times the flux and the exposure;
  - shot noise, read noise and the offsets;
  - the conversion to ADU, then quantization and clipping to the pixel format.
- **`form_image`** is the two together. It is a pure function of the world, the models, the shot
  (pose, calibration, sensor, settings) and the frame number.

### Defocus

- **The defocus at a point** is `z_coarse + focus` plus the world's surface height there. A
  volume's surface is taken as 0 (ADR-0018: +z toward the objective).
- **The blur's σ on the sample** has two parts, added in quadrature:
  - the diffraction-limited spot, 0.21 λ / NA;
  - half the radius of the geometric cone, of half-angle asin(NA / n).
- **It is computed on a grid** every 32 frame pixels.
  - Where it varies by under half a pixel, the frame gets one Gaussian blur.
  - Otherwise up to six blur levels are blended per pixel, by the σ interpolated from the grid:
    a relief is sharp where it is in focus.
- **A strongly blurred frame is sampled coarsely,** at about half its least blur, then blurred
  and enlarged. Blurred detail costs nothing to drop.

### The sensor model

| Effect | Model | Default |
|---|---|---|
| Vignetting | cos⁴, scaled to a corner value, or `1 + aρ² + bρ⁴` | corners at 0.8 |
| PRNU | a fixed gain per pixel, from its hash | 0.5 % RMS |
| Shot noise | Poisson below 20 e⁻, by inversion; Gaussian above | — |
| Read noise | Gaussian, once per binned pixel read | 2.5 e⁻ |
| Conversion | electrons per ADU at gain 1, divided by the camera's gain | 2 e⁻/ADU |
| Offsets | a dark offset, and a fixed offset per pixel (DSNU) | 100 ADU, 1 ADU RMS |
| Quantization | rounded, clipped to the format's maximum | Mono14 |

### Randomness is counter-based

- **Every random number is a hash** of the sensor seed and the sensor pixel. For the temporal
  noise, the hash also covers the frame number.
- **So frames are reproducible:** a frame repeats exactly for its seed and number.
- **An ROI's pixels are exactly the full frame's,** away from the ROI's edges, where the blur
  reaches past it.
- **The pixels are independent,** so the noise is computed in parallel.
- **Gaussian deviates come from a table** of 64 Ki standard-normal quantiles, indexed by the hash.

### Binning

- **A binned pixel is rendered at its centre, with its own footprint,** times its area.
- **It equals the sum of its sensor pixels to within the world's own averaging:** a mean
  difference of 0.5 % and a worst case of 3 % in the tests. Rendering binned frames at sensor
  resolution and then summing would cost the full sampling of a frame that binning makes cheaper.
- **Its read noise is that of its pixels together.** Its PRNU and DSNU are those of its first
  pixel.

### The renderer and the noiseless cache

`Renderer` keeps the last signal and the shot it was for.

- **The same signal as the last frame** (the same pose, calibration, ROI and binning): only
  `digitize` runs. The exposure, gain and format don't change the signal.
- **A new pose or readout region:** a draft, sampled coarsely enough for about 6 M samples.
  - That is 1/5 of the resolution at 100 MP, 1/3 at 25 MP, and full detail for frames of 6 MP
    or less.
  - Motion blurs the real camera's frames anyway.
- **The same shot as a draft:** the full-detail signal, which is then kept.
- **The buffers keep their memory and only grow,** so drafts and full renders don't reallocate in
  turn.

### Brief deviations

1. **`form_image` takes a `Shot`,** holding the pose, calibration, sensor and camera settings,
   and a frame number in place of an `rng`, which counter-based noise doesn't need.
2. **The cache drafts on a new pose** at a coarser detail, from a sample budget, rather than the
   brief's "reduced resolution when σ is large" alone. Strong blur is also sampled coarsely.
3. **Volumes are rendered with their surface at 0.** Bead z-stacks are `siq-datagen`'s, through
   the beads' own z (P12.4).
4. **The example writes a PNG preview, not a JPEG.** The JPEG encoder the plan names
   (`jpeg-encoder`, P2.2) is licensed "(MIT OR Apache-2.0) AND IJG", and IJG isn't on the licence
   allow-list yet.

## Measurements

`render_frame`, release build, development PC (16 threads), the 11 648 × 8742 sensor at 0.376 µm
per pixel:

| Frame | Time | Rate | Target |
|---|---|---|---|
| Still pose, noise only | 96–104 ms | 9.6–10.4 frames/s | 9 frames/s |
| New pose, draft at 1/5 | 266 ms (339 ms when the buffers first grow) | 3.8 frames/s | ≥ 3 frames/s |
| Still pose, full detail, once per stop | 1.57 s | — | — |
| 12 µm defocus, full: sampled at 1/2 | 0.59 s | — | — |
| `fast` profile (25 MP), noise only | 21 ms | 48 frames/s | ≥ 9 frames/s |
| `fast` profile, draft at 1/3 | 102 ms | 9.8 frames/s | — |

**Memory:** a 100 MP full render holds about 1.2 GB of `f32` buffers: the signal, the sharp image
and the blur pass. A blended relief needs one more.

## Alternatives considered

- **Render every frame in full:** 0.6 frames/s at 100 MP.
- **Shift the cached signal by whole pixels while jogging:** fast for translations, but
  sub-pixel moves, the focus and the ROI all need a render anyway. It is a possible later
  optimization.
- **A seeded generator per frame:** the noise would depend on the order of evaluation, and an
  ROI wouldn't be a crop.
- **Box-Muller per pixel:** about ten times the cost of the table.
- **Spatially varying blur by a kernel per pixel:** far too slow. The blend of six levels covers
  the reliefs depth from focus needs.

## Consequences

- **The stationary rate is only just met:** the noise pass is about 90 ms of the 111 ms budget at
  100 MP. P1.7 may cache each pixel's fixed offset to reduce it.
- **The first full-detail frame after a stop takes 1.6 s at 100 MP.** P1.7 decides how the camera
  times it: drop the frames due meanwhile, or deliver late.
- **A relief with more than six levels' worth of blur is approximated** between its levels.

## Verification

- `siq-sim` tests:
  - `a_frame_repeats_for_its_seed_and_number`;
  - `sharpness_falls_as_the_focus_moves_away_either_way`;
  - `vignetting_falls_from_the_centre_to_its_corner_value`;
  - `noise_has_the_mean_and_variance_of_poisson_plus_read_noise`, including P(0) of a Poisson;
  - `an_roi_is_a_crop_and_binning_a_sum`;
  - `a_frame_clips_to_its_pixel_format`;
  - `a_varying_defocus_blurs_only_where_the_surface_is_out_of_focus`;
  - `the_renderer_drafts_while_moving_keeps_a_still_signal_and_samples_strong_blur_coarsely`.
- The `render_frame` example.

## Amendment (P1.7, 2026-10-24): timing, triggering and `TrueState`

### `TrueState`

- **What it holds:** the true pose, as stage x, y, z_coarse and focus, and the illumination
  levels, the filter, the objective and the modality.
- **Who writes it:** the emulators (P1.8, P1.10) and the peripheral simulators. The camera reads
  it.
- **Reads take no lock.**
  - The pose is behind a sequence lock of atomics: writers take turns, and a reader retries while
    one writes, so it never sees half of a move.
  - The peripherals are single atomics.
- **The light reaching the sample** is the sum of the levels, or 1 when no sources are simulated.
  It scales the flux.
- **The objective at the turret position in place** sets the calibration
  (magnification ÷ pixel pitch) and the optics' NA and index.

### Timing

- **The camera runs on its own thread,** timed by the shared `Clock` through
  `wait_until(deadline, 20 ms)`, so that it notices a stop.
- **Exposures start every period, `max(exposure, readout)`.** The readout is the capability
  model's: 1 ms plus a line time per row read, which gives the configured full-frame rate.
- **A frame is ready its exposure plus its readout after it starts.**
- **Its timestamps:**
  - the host timestamp is its mid-exposure, on the schedule;
  - the device timestamp counts microseconds from `start()`.
- **Exact on a `ManualClock`.** The schedule doesn't depend on how long rendering takes, so the
  timing is exact on a manual clock.
- **The time zero is taken when `start()` is called,** not when the thread first runs.
- **A late render slips the schedule.**
  - When a render runs late, the next frame is ready at once rather than overdue.
  - Frame numbers stay contiguous, and the rate drops.
  - Lateness is judged before the frame is handed to the sink, which may take any time.
  - So the slow full-detail frame after a stop costs rate, not lost frames. Lost frames mean a
    want of slots (REQ-ACQ-005).

### Triggering

- **`Software` and `External` modes:** an exposure starts `trigger_latency` (0.1 ms) after
  `trigger()`, and not before the last frame's period allows.
- **The simulator has no trigger line,** so `trigger()` also stands in for an external trigger.
- **One trigger is pending at a time.** Another before its exposure has started is refused as
  `Busy`.

### Settings, slots and stopping

- **Settings are read at the start of each exposure,** so a change takes effect at the next frame.
- **While acquiring,** only the exposure and the gain change.
- **No free slot when a frame is ready:** the frame is lost and counted as `lost_no_slot`, and its
  number skipped.
- **`stop()` resolves when the thread has finished,** through a waker the thread wakes.

### The `fast` profile

`SimCameraSetup::from_config` halves each side of the sensor and doubles its pixel, keeping the
field of view. ROI steps are the largest of 8, 4, 2 and 1 that divide each side, so a sensor's full
frame is always a valid ROI.

### Measurements

- **`sim_live`,** release build, 5824 × 4371 (`fast`), real time, a new pose every second: 6–8
  frames per pose, 5.9–7.0 frames/s. Each new pose costs a draft and one full render, about
  0.45 s, before the noise-only frames at up to 9 frames/s. No frames were lost.
- **On a `ManualClock`,** frames come at exactly 9.000 frames/s for the full 192-row test sensor,
  and 26.5 frames/s for a 64-row ROI.

### Verification

- `siq-sim` camera tests:
  - `free_running_frames_come_at_the_model_s_rate_for_the_full_frame_and_an_roi`;
  - `a_software_trigger_starts_an_exposure_after_its_latency`;
  - `frames_are_lost_and_counted_while_consumers_hold_every_slot`;
  - `a_settings_change_takes_effect_at_the_next_frame`;
  - `the_camera_sees_the_true_state`;
  - `a_pose_is_never_seen_half_written`.
- The `sim_live` example.
