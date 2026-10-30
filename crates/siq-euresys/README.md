# siq-euresys

The Euresys camera (ADR-0070): a `siq-devices` `Camera` that fills the frame pool's slots by DMA
from a Coaxlink CoaXPress grabber, through `siq-euresys-sys`'s shim. A per-model feature map names
the camera's features where they differ from GenICam SFNC's.

## Place in the architecture

- **Layer:** L2 Devices. One of the two crates allowed `unsafe` (ADR-0007): the shim's FFI, and
  DMA into the pool's slots.
- **Depends on:** `siq-euresys-sys`, `siq-devices`, `siq-buffer`, `siq-types`; `serde`, `toml`,
  `thiserror`, `tracing`, `tokio` (its timer, for `stop`).
- **Used by:** `siq-server`, with its `euresys` feature.
- **Phase:** P10.1; P10.2 measured the Adimec's modes and timestamps, and set the thread's
  priority (the [camera bring-up runbook](../../docs/hardware/camera-bringup.md)); P10.4 the strobe,
  and the 2 ms tick (ADR-0072).

## API

- **`EuresysCamera<G: Grabber>`:** the `Camera`. `new(grabber, map, clock)` reads what the
  camera can do, and sets it up as the simulated camera starts: the full frame, unbinned, in its
  deepest pixel format (datasets take 16-bit frames), at 10 ms and unity gain, free-running.
  - **Capabilities,** read from the camera: the sensor's size, the pixel formats the map knows,
    the binnings up to `BinningHorizontal.Max` and `BinningVertical.Max`, the region's steps
    (`.Inc`) and least size (`.Min`), and the exposure's range; the pixel pitch, the colour filter
    array and the readout's model come from the map.
  - **Settings:** GenICam's region is in binned pixels, so the camera sets the binning first, then
    the offsets cleared, the size, and the offsets. Free-running, the frame period is the readout,
    or the exposure and 4 ms if that is longer: the Adimec skips every other frame unless its
    period is more than 2 ms longer than its exposure. The period is set before the exposure and
    again after, as a camera shortens an exposure to fit its period. While acquiring, only the
    exposure and the gain change.
  - **Acquiring:** every pool slot is announced at its own index. One thread, at time-critical
    priority (architecture §4.3), pops each filled buffer within a 2 ms tick, hands it on as a
    frame, and requeues a slot once its last lease drops, or once it is dropped unpublished. A frame incomplete, or of another size than the
    settings say, is lost (`lost_device`); one the grabber had no slot for is an underrun
    (`lost_no_slot`). A caller waiting for the grabber, to trigger or to change the exposure, has
    it within a tick: the thread steps aside. A software trigger's exposure starts 0.45–1.05 ms
    after the call; with P10.1's 20 ms tick, up to 21 ms after it (P10.4).
  - **A strobe** (`with_strobe(Strobe { line, delay, duration })`, P10.4): triggered, the grabber's
    camera controller times a pulse on the output line (such as `IOUT11`) in each exposure, from
    `delay` after its start, for `duration` or the whole exposure. The line's source is
    `Device0Strobe`; free-running, `Low`.
  - **`with_grabber(read)`** runs `read` on the grabber beside the camera's thread: for its event
    counters, in a test or a bring-up.
  - **A frame's metadata:** its number; the grabber's timestamp as `device_ts`; the middle of its
    exposure on the host's clock; and its colour filter array, shifted by an odd offset of the
    region. The grabber stamps a frame as its readout starts, as its exposure ends, and it arrives
    its readout later (P10.2). So the middle of the exposure is the stamp, less half the exposure,
    with the clocks' offset taken as the least of the last 64 frames' arrival, less their readout,
    less their stamp.
  - **Stopping,** also on a panic: the grabber stops and revokes the slots before they go back to
    the pool.
- **`Grabber`:** what the camera needs of a grabber: features by name (`get`, `set`, `execute`,
  `entries`), `announce`, `start`, `stop`, `pop`, `requeue`, `counters`. `EGrabber` is eGrabber's,
  with `sdk`; the tests have a fake.
- **`EGrabber::open(producer, interface, device)`:** the camera at that device of that grabber
  card, through the default GenTL producer (Coaxlink's) or another, such as Playlink's.
  `features(module)` lists a module's features.
- **`FeatureMap`:** the camera's model, its colour filter array, its pixel pitch, its pixel formats
  by entry, the exposure's and the gain's features (the gain in dB or as a factor), the frame
  period's feature, the features each trigger mode sets, the trigger's command, and the readout's
  model. A setting names another module than the camera's first: `device:CameraControlMethod`.
  Its default is SFNC's, without a pixel pitch, which a camera must be given.

The Adimec D-103A12c's map is [`config/cameras/adimec-d103a12c.toml`](../../config/cameras/adimec-d103a12c.toml):
its raw Bayer samples (`BayerGB8`, `10` and `12`, as `Mono8`, `10` and `12`, GBRG), its gain as a
factor, its free-running period (`AcquisitionFramePeriod`), and its triggers. A software trigger
is the grabber's camera controller's `StartCycle` (`CameraControlMethod` `RC`): the camera's own
`TriggerSoftware` makes no frame on it.

## Examples

| Example | What it shows | Run |
|---|---|---|
| `eg_live` | Live view of the camera of grabber 0 into a frame pool, the full frame for 10 s in `mono12` at 10 ms by default: the frame rate, the frames lost, and the jitter of the grabber's timestamps. It puts back the camera's own features as it found them | `cargo run --release -p siq-euresys --features sdk --example eg_live -- [seconds] [mono8\|mono10\|mono12] [exposure ms]` |
| `hw_acquire_bench` | Each mode for 5 s at 1 ms (the full frame at 8, 10 and 12 bits, three regions, 2 × 2 and 4 × 4 binning, and the full frame at 50 ms): its rate against the map's model, its DMA throughput, its losses, the clocks' drift, the arrivals' jitter, and each mode's least lag after its stamp, which tells what the stamp marks | `cargo run --release -p siq-euresys --features sdk --example hw_acquire_bench -- [seconds a mode]` |
| `strobe_timing` | Software triggers of a 2048 × 2048 region at 5 ms, 200 ms apart, a strobe on `IOUT11` (`--line`, `--delay`, `--duration`): each call's time, each exposure's start after its call, and the camera's triggers, the strobes and the line's own edges, counted by the grabber, the line read back through a line input tool. 40 triggers: the exposure 0.45–1.05 ms after the call, and 40 of each count. It puts back everything it changes, on the camera and the grabber | `cargo run --release -p siq-euresys --features sdk --example strobe_timing -- [--triggers N] [--exposure ms] [--gap ms]` |
| `eg_features` | A module's features with their values and entries, changing nothing, for writing a feature map; or `--save <file>` and `--restore <file>` of the settings the camera changes, around an experiment | `cargo run --release -p siq-euresys --features sdk --example eg_features -- [remote\|device\|stream\|interface]` |

Both need the camera, so `cargo xtask ci` doesn't run them.

## Features

- **`sdk`:** `EGrabber`, through the installed eGrabber SDK (`EGRABBER_SDK_DIR`, or its default
  folder). Without it, the camera builds over any `Grabber`.

## Testing

- `tests/camera.rs`, in CI, over a fake grabber that writes each frame's number into the slot it
  fills:
  - the capabilities and settings read as the Adimec's map names them;
  - a map for another model, or without the pixel pitch, refused;
  - the region set in binned pixels, the trigger modes, ten triggers while the thread pops, and
    only the timing changing while acquiring;
  - every slot filled and requeued only once its last lease drops, an underrun, the slots back in
    the pool after a stop;
  - an incomplete buffer lost and filled again, and a start refused while a slot is held.
- `tests/hardware.rs`, ignored, on the camera of this PC. Each test reads the camera's own features
  first, and puts them back after:

  ```text
  set SIQ_EURESYS_DEVICE=0:0
  cargo test --release -p siq-euresys --features sdk --test hardware -- --ignored --test-threads 1
  ```

  - 100 full frames in sequence and evenly timed, with none lost;
  - one frame for each of 5 software triggers, and none without one;
  - a 4096 × 2048 region, unbinned and binned 2 × 2, at its size, with each phase of the colour
    filter's cell measured;
  - each mode at the rate its model gives, within 5 %: the full frame, two regions, 2 × 2 binning,
    and the full frame at 150 ms;
  - each exposure's middle following the grabber's stamps, within 0.25 ms from one frame to the
    next (0.02 to 0.07 ms measured);
  - 15 software triggers strobing `IOUT11`: 15 camera triggers, 15 strobes and 15 edges on the
    line, and each exposure within 2 ms of its call (P10.4). The grabber's settings it changes are
    put back too.

## How to try it

On the instrument, from the repository, with the SDK installed:

1. `cargo run --release -p siq-euresys --features sdk --example eg_live`: about 12.3 frames/s for
   the Adimec's full frame, a frame every 81.4 ms, jitter well under a millisecond, and none
   lost.
2. Run the hardware tests above: six pass.
3. `strobe_timing`: each exposure about 0.6 ms after its trigger, and as many strobes as triggers.
   With the illumination's strobe input wired to `IOUT11`, its light pulses.
4. `hw_acquire_bench`: each mode within a few % of its model, none lost (the
   [runbook](../../docs/hardware/camera-bringup.md#6-each-mode-and-the-timestamps)'s table).
5. The service with the camera, and the simulated stage and focus: copy
   `config/sim.toml` into a folder of its own, replace its `[camera]` with

   ```toml
   [camera]
   kind = "euresys"
   interface = 0
   device = 0
   features = "C:/repos/SpectrumIQ/config/cameras/adimec-d103a12c.toml"
   ```

   add a user to it (`spectrumiq-server user add`), and run:

   ```text
   cargo run --release -p spectrumiq-server --features euresys -- -c <that file>
   ```

   Log in, take control, and start live view: both previews stream at the camera's rate, and
   `/api/v1/camera/stats` shows none lost. Under load: `cargo xtask soak --euresys --viewers 8
   --minutes 10`. A strobe is `strobe = { line = "IOUT11" }` in `[camera]`, with `delay_ms` and
   `duration_ms` if wanted.
6. `eg_features --save before.txt` first and `--restore before.txt` after, to leave the camera's
   settings as they were.

## Limitations

- **The frame-rate model** ignores the 4 ms the period needs past a long exposure: it gives 6.67
  frames/s at 150 ms, where the camera runs 6.49.
- **The frames stay raw Bayer,** shown in colour and processed on their luminance (ADR-0071).
- **The strobe is the controller's,** for triggered frames only: free-running, the CIC doesn't
  run, and the line stays low. Its pulse is checked by counting, not on a scope.
- **No affinity:** the thread is time-critical, but not pinned to a core. Nothing measured asked
  for it: no frame lost under load, and the arrivals' jitter under 0.2 ms rms.
- **The pixel pitch,** 3.2 µm, is the GMAX32103 sensor's, still to confirm from Adimec's datasheet.
