# Camera bring-up runbook

How to bring up a CoaXPress camera on the Coaxlink, from the SDK to the service, and what P10.1
and P10.2 found for the Adimec D-103A12c/CXP-T01 (ADR-0070). Run each step from the repository's
root, in a terminal with the eGrabber SDK installed.

## 1. What is needed

- The Euresys eGrabber SDK and the Coaxlink driver, in `C:\Program Files\Euresys\eGrabber`, or
  `EGRABBER_SDK_DIR` set to the SDK's folder.
- MSVC, which builds the shim (`siq-euresys-sys`, feature `sdk`).
- Every command below needs `--features sdk` (`--features euresys` for the service). CI builds
  none of them.

## 2. Keep the camera's settings

The tests and examples put back what they change, but an experiment by hand may not. Save the
camera's settings first, and put them back after:

```text
cargo run --release -p siq-euresys --features sdk --example eg_features -- --save before.txt
cargo run --release -p siq-euresys --features sdk --example eg_features -- --restore before.txt
```

## 3. Find the camera

```text
cargo run -p siq-euresys-sys --features sdk --example eg_discover -- --open
```

It lists the grabber cards, the grabbers and the cameras, with each camera's size and pixel
format. The Adimec is camera 0 of interface 0, `PC3603 - Coaxlink Quad CXP-12 (1-camera)`.

**The link:** `eg_features -- device` shows `CxpLinkConfiguration`. Here it is `CXP12_X2`, two of
the camera's four links (its default is `CXP12_X4`). The full frame at 12 bits needs 1.9 GB/s, which
two links carry: the rate is the sensor's, the same at 8, 10 and 12 bits (step 6). Four cables would
leave headroom.

## 4. Write the camera's feature map

`config/cameras/<model>.toml` names the camera's features where they differ from GenICam SFNC's
(`siq-euresys`'s `FeatureMap`). List them:

```text
cargo run --release -p siq-euresys --features sdk --example eg_features -- remote
cargo run --release -p siq-euresys --features sdk --example eg_features -- device
```

Then fill in:

- `model`: `DeviceModelName`, which the map is checked against.
- `pixel_formats`: each `PixelFormat` entry and the samples it gives, `Mono8` to `Mono16`; and
  `cfa` for a colour camera, from the entry's name (`BayerGB` is `gbrg`).
- `pixel_size_um`: from the sensor's datasheet; GenICam doesn't give it.
- `exposure` (µs), `gain`, and `gain_db` if the gain is in dB.
- `frame_period`: the feature that sets the free-running period, if the camera has one
  (`AcquisitionFramePeriod` on the Adimec).
- `[trigger]`: what each mode sets, in order, with another module's features prefixed
  (`device:CameraControlMethod`), and `trigger_command` and `trigger_module`. On the Adimec, the
  camera's own `TriggerSoftware` makes no frame: a software trigger is the grabber's camera
  controller (`CameraControlMethod` `RC`, `CycleTriggerSource` `StartCycle`, command `StartCycle`
  on the device module).
- `readout_overhead_ms`, `line_time_us` and `binned_rows`: from step 6.

## 5. Live view

```text
cargo run --release -p siq-euresys --features sdk --example eg_live -- 10 mono12 10
```

The full frame for 10 s at a 10 ms exposure: the rate, the jitter of the grabber's timestamps, and
the frames lost. The Adimec gives 12.28 frames/s, a frame every 81.43 ms, jitter 0.001 ms rms, none
lost.

## 6. Each mode, and the timestamps

```text
cargo run --release -p siq-euresys --features sdk --example hw_acquire_bench
```

Each mode for 5 s at a 1 ms exposure: its rate against the map's model, its DMA throughput and its
losses; the host's clock against the grabber's, their drift and the arrivals' jitter; and each
mode's least lag, against the first mode's.

- **The readout model:** fit the periods of the full frame and the regions against their rows.
  Here a frame takes 2.0 ms and 8.63 µs a row (`readout_overhead_ms`, `line_time_us`).
- **Binning:** if a binned mode is no faster than the full frame, the sensor reads every row and
  bins after: `binned_rows = false`, as on the Adimec.
- **What the grabber's timestamp marks:** a smaller region arrives earlier after its stamp, by the
  readout of the rows it lacks, so the stamp marks the start of the frame's readout, as its
  exposure ends. A longer exposure doesn't move it. The camera takes the middle of the exposure as
  the stamp less half the exposure, on the host's clock.

The Adimec's (P10.2):

| Mode | Frames/s | The model | MB/s by DMA | Lost |
|---|---|---|---|---|
| Full, `Mono12` | 12.28 | 12.29 | 2546 | 0 |
| Full, `Mono10` | 12.28 | 12.29 | 2546 | 0 |
| Full, `Mono8` | 12.28 | 12.29 | 1273 | 0 |
| 4096 × 3072 | 35.07 | 35.07 | 883 | 0 |
| 2048 × 2048 | 50.82 | 50.83 | 426 | 0 |
| 1024 × 1024 | 91.40 | 92.28 | 192 | 0 |
| Full, binned 2 × 2 | 12.28 | 12.29 | 637 | 0 |
| Full, binned 4 × 4 | 12.28 | 12.29 | 159 | 0 |
| Full, 50 ms | 12.28 | 12.29 | 2546 | 0 |
| Full, 150 ms (the hardware test) | 6.49 | 6.67 | 1345 | 0 |

The arrivals' jitter is 0.15–0.20 ms rms, 0.6 ms at worst; the clocks drift by up to 60 ppm.
The model's column is the measured model's; with P2.5's (no overhead, 8.85 µs a row) the regions'
rates were up to 20 % high, and the binned modes' twice the real ones.

## 7. The hardware tests

```text
set SIQ_EURESYS_DEVICE=0:0
cargo test --release -p siq-euresys --features sdk --test hardware -- --ignored --test-threads 1
```

Five pass, each putting back the camera's features:

- 100 full frames in sequence, evenly timed, none lost;
- one frame for each software trigger;
- a binned region at its size, with its colour filter's phases;
- each mode at its model's rate within 5 %, the full frame at 150 ms too;
- each exposure's middle following the grabber's stamps.

## 8. The service on the camera

Copy `config/sim.toml` into a folder of its own and replace its `[camera]`:

```toml
[camera]
kind = "euresys"
interface = 0
device = 0
features = "C:/repos/SpectrumIQ/config/cameras/adimec-d103a12c.toml"
```

Add a user to it, and run:

```text
cargo run --release -p spectrumiq-server --features euresys -- -c <that file>
```

At start, the service logs the frame pool (12 slots of 207 MB, locked) against the computer's RAM,
and refuses a pool that, with the jobs' and tile cache's budgets, needs more than 90 % of it. The
camera starts as the simulator does: the full frame, in its deepest format, at 10 ms.

Under load, with viewers and an operator:

```text
cargo xtask soak --euresys --viewers 8 --minutes 10
```

The stage stays simulated, so it can't be calibrated against the camera: the operator snaps into
datasets, which keeps the recorder busy, and doesn't scan, and the live mosaic doesn't paint. In P10.2,
10 minutes with 8 viewers published 4,725 frames and lost none; every viewer received every preview;
the overview came 147 ms after the middle of its exposure on average, p95 ≤ 200 ms, with 20 ms of
the pipeline's work; the service used 8 % of the CPU.

## 9. Troubleshooting

| What | Why, and what to do |
|---|---|
| Half the frame rate at a long exposure | The Adimec skips every other frame unless its period is more than 2 ms longer than its exposure: the camera sets the period to the exposure and 4 ms |
| An exposure shorter than asked | The camera shortens an exposure to fit its period: set the period first (the camera does, and again after) |
| A software trigger makes no frame | The camera's `TriggerSoftware` does nothing with `TriggerSource` `Trigger`: use the grabber's camera controller |
| A start refused, "slots are still held" | Every pool slot is announced at start: a consumer still holds a frame |
| "only 16-bit frames for now" on a snap | The camera is in an 8-bit format: datasets take 16-bit frames |
| A frame lost "of the wrong size" | The region the camera sends isn't the one set: check that GenICam's region is in binned pixels |
