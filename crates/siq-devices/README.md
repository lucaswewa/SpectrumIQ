# siq-devices

SpectrumIQ's device traits: what the engines know of the camera, the stage, the focus drive and
the peripherals, whatever drives them.

- Every operation returns a boxed `Send` future, so the traits are dyn-compatible: engines hold
  `Arc<dyn Stage>`, and the server picks each driver from the configuration.
- What a device can do is data: camera capabilities with a frame-rate model, travel, position
  names, modes.
- Status comes on a `watch` channel: the latest at once, and each change as it comes, with no
  polling.
- The `mock` module has a mock of each trait that completes every operation at once, for engine
  tests.

The design is in [ADR-0023](../../docs/adr/0023-device-abstraction.md).

## Place in the architecture

- **Layer:** L2 Devices.
- **Depends on:** `siq-types`, `siq-buffer` (the camera fills pool slots), `tokio` (`sync`, for
  `watch`), `serde`, `schemars`, `thiserror`.
- **Used by:**
  - the drivers, which implement the traits: P1.7's simulated camera, P1.9's GRBL stage, P1.10b's
    ASCII focus drive and peripheral simulators, and P10.1's Euresys camera;
  - the engines, which use only the traits: P2.1's acquisition and every workflow;
  - P2.3's server, which picks the drivers.
- **Phase:** P1.3; P10.1 added `CameraCapabilities::cfa`; P10.2 `FrameRateModel::readout` and
  `binned_rows`.

## API

| Item | What |
|---|---|
| `Device` | Every device: `info()` → `DeviceInfo` (vendor, model, serial, firmware) |
| `BoxFuture<'_, T>`, `DeviceResult<T>`, `DeviceError` | What operations return. Errors: `Timeout`, `Disconnected`, `Fault`, `InvalidArgument`, `Busy`, `Unsupported`, `Alarm(code)`, `Stopped` |
| `Camera` | `capabilities`, `settings`, `configure(CameraSettings)`, `start(pool, Arc<dyn FrameSink>)`, `stop`, `trigger`, `stats` → `CameraStats` (delivered, lost for want of a slot, lost by the device) |
| `CameraCapabilities` | Sensor, pixel size, formats, a colour sensor's filter array (`cfa`, P10.1), binnings, `RoiRules`, exposure range, `FrameRateModel`. `check(&settings)` is the one settings check; `frame_rate.readout(&settings)` the readout's time and `frame_rate.max_rate(&settings)` the rate for an ROI and binning. `binned_rows` (P10.2) says whether vertical binning reads fewer rows: not on a sensor that reads them all and bins after, as the Adimec |
| `CameraSettings` | ROI, binning, format, exposure, gain, `TriggerMode` (`FreeRun`, `Software`, `External`). `full_frame`, `image_size`, `frame_bytes`, `frame_meta` |
| `FrameSink` | Where a camera hands each `(FillGuard, FrameMeta)`: the acquisition engine, which adds the pose and publishes |
| `Stage`, `FocusDrive` | `move_to`, `move_by` (with `MoveOpts { speed, approach: Direct or Unidirectional }`), `jog(velocity)`, `stop`, `home`, `status()`, `limits()` → `Travel`. Moves resolve once settled, with the status. A `Stage`'s `kinematics()`, `Kinematics { speed_um_s, acceleration_um_s2 }` in the plane, for estimates, is `None` unless its driver says (P6.6) |
| `AxisStatus<P>`, `MotionState` | `position`, `state` (`Idle`, `Moving`, `Jogging`, `Homing`, `Holding`, `Alarm(code)`, `Disconnected`), `homed`, `ts` on the shared clock. `StageStatus` for `Xyz`, `FocusStatus` for `Micrometers` |
| `Illumination` | `sources`, `set_level(source, 0..=1)`, `status()` → levels |
| `Changer`, `FilterWheel`, `ObjectiveTurret` | `positions` (names), `select(position)`, `status()` → `ChangerStatus` |
| `ModalityUnit` | `modes`, `patterns` → `PatternSet`, `set_modality(Modality { mode, pattern })`, `status()`. `Modality::check` validates against a unit |
| `watch` | `tokio::sync::watch`, re-exported |
| `mock::*` | `MockCamera` (`emit`, `lose_frame`), `MockStage`, `MockFocusDrive`, `MockIllumination`, `MockChanger`, `MockModalityUnit`, each with `fail_next(error)`, and `small_sensor()` |

```rust
let stage: Arc<dyn Stage> = drivers.stage();
let mut status = stage.status();
let settled = stage.move_to(Xyz::from_um(1_000.0, 500.0, -2_000.0), MoveOpts::default()).await?;
camera.trigger().await?;                         // software mode: the frame goes to the sink
while status.changed().await.is_ok() { show(*status.borrow_and_update()) }
```

## Examples

| Example | What it shows | Run |
|---|---|---|
| `mock_rig` | Homes a mock stage, visits a 3 × 3 grid, snaps a mock frame at each stop, and prints the stage's status stream and the camera's statistics | `cargo run -p siq-devices --example mock_rig` |

## Features

None.

## Testing

`cargo test -p siq-devices` runs two binaries:

| Binary | What |
|---|---|
| `traits` | Every trait behind `Arc<dyn …>` with its futures spawned on tokio; the camera settings checks; the frame-rate model; the travel and modality checks |
| `mock` | The mock stage, camera and peripherals, and the status-watch semantics: latest at once, waking, collapsing, and the end when the device goes |

## Limitations

- **The mocks complete at once.** Timing, motion profiles and protocol behaviour are the
  simulators' (P1.7, P1.8, P1.10a, P1.10b).
- **A status watch collapses changes** that a reader misses. A reader that needs every sample
  must keep up.
- **`FrameSink::frame` runs on the camera's thread.** It must be quick.
