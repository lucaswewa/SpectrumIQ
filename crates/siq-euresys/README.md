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
- **Phase:** P10.1.

## API

- **`EuresysCamera<G: Grabber>`:** the `Camera`. `new(grabber, map, clock)` reads what the
  camera can do, keeps the camera's format, region, exposure and gain, and sets it free-running.
  - **Capabilities,** read from the camera: the sensor's size, the pixel formats the map knows,
    the binnings up to `BinningHorizontal.Max` and `BinningVertical.Max`, the region's steps
    (`.Inc`) and least size (`.Min`), and the exposure's range; the pixel pitch, the colour filter
    array and the readout's model come from the map.
  - **Settings:** GenICam's region is in binned pixels, so the camera sets the binning first, then
    the offsets cleared, the size, and the offsets. The frame period is set before the exposure
    and again after, as a camera shortens an exposure to fit its period. While acquiring, only
    the exposure and the gain change.
  - **Acquiring:** every pool slot is announced at its own index. One thread pops each filled
    buffer within a 20 ms tick, hands it on as a frame, and requeues a slot once its last lease
    drops, or once it is dropped unpublished. A frame incomplete, or of another size than the
    settings say, is lost (`lost_device`); one the grabber had no slot for is an underrun
    (`lost_no_slot`). A caller waiting for the grabber, to trigger or to change the exposure, has
    it within a tick: the thread steps aside.
  - **A frame's metadata:** its number; the grabber's timestamp as `device_ts`; the middle of its
    exposure estimated on the host's clock, from when it was popped; and its colour filter array,
    shifted by an odd offset of the region.
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
    filter's cell measured.

## How to try it

On the instrument, from the repository, with the SDK installed:

1. `cargo run --release -p siq-euresys --features sdk --example eg_live`: about 12.3 frames/s for
   the Adimec's full frame, a frame every 81.4 ms, jitter well under a millisecond, and none
   lost.
2. Run the hardware tests above: three pass.
3. The service with the camera, and the simulated stage and focus until P10.3: copy
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

   Log in, take control, set the pixel format to `Mono12` on the Live page, and start live view:
   both previews stream at the camera's rate, and `/api/v1/camera/stats` shows none lost.
4. `eg_features --save before.txt` first and `--restore before.txt` after, to leave the camera's
   settings as they were.

## Limitations

- **The frame-rate model** is P2.5's measurement of the full frame: binning doesn't shorten this
  camera's readout, which the model assumes. P10.2 measures each mode.
- **The middle of the exposure** is estimated from when the buffer was popped. P10.2 maps the
  grabber's timestamps onto the host's clock.
- **The frames stay raw Bayer.** The previews, the viewer and the stitching treat them as mono
  until M10's colour phases.
- **The thread's priority and affinity** are P10.2's.
- **The pixel pitch,** 3.2 µm, is the GMAX32103 sensor's, still to confirm from Adimec's datasheet.
