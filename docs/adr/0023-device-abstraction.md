# ADR-0023: Device abstraction

- **Status:** Accepted
- **Date:** 2026-10-23
- **Phase:** P1.3
- **Related:** ADR-0016, ADR-0021, ADR-0022, REQ-DEV-001, REQ-STG-003, REQ-ACQ-003, REQ-ACQ-005

## Context

- **Engines depend on device traits, never on drivers** (architecture §3.1, rule 2). The server
  picks each device's driver, simulated or real, from the configuration.
- **The devices** (ADR-0016):
  - the camera, through Euresys or the simulator;
  - the GRBL stage: x, y and the coarse z;
  - the ASCII focus drive: the fine z;
  - illumination, a filter wheel, an objective turret and a modality unit, whose models aren't
    chosen yet.
- **Operations take milliseconds to seconds,** and are awaited from tokio tasks. `stop` must work
  while a move is under way.
- **Engines and the UI need positions with timestamps all the time** (REQ-STG-003), and need to
  know what each device can do, to check requests and to show limits.

## Decision

### Dyn-compatible async traits

- **Every operation returns a `BoxFuture<'_, DeviceResult<T>>`,** and takes `&self`.
  - Engines hold `Arc<dyn Stage>` and friends.
  - `stop` can run while a move is awaited elsewhere.
  - `BoxFuture` is the crate's own alias for `Pin<Box<dyn Future + Send>>`. It needs no
    `futures` crate.
- **Every device is a `Device`** (`Send + Sync`), with `info()`: vendor, model, serial, firmware.

| Trait | Operations | Status |
|---|---|---|
| `Camera` | `capabilities`, `settings`, `configure`, `start(pool, frames)`, `stop`, `trigger`, `stats` | `CameraStats` |
| `Stage` | `move_to`, `move_by` (with `MoveOpts { speed, approach }`), `jog(velocity)`, `stop`, `home`, `limits` | watch of `AxisStatus<Xyz>` |
| `FocusDrive` | the same, on one axis | watch of `AxisStatus<Micrometers>` |
| `Illumination` | `sources`, `set_level(source, 0..=1)` | watch of the levels |
| `Changer`, as `FilterWheel` and `ObjectiveTurret` | `positions` (names), `select(position)` | watch of the position |
| `ModalityUnit` | `modes`, `patterns`, `set_modality(mode, pattern)` | watch of the modality |

### Capabilities are data

- **`CameraCapabilities`:**
  - the sensor's size and pixel pitch;
  - the pixel formats and binnings;
  - the ROI's alignment (`RoiRules`);
  - the exposure range;
  - a `FrameRateModel`: readout overhead plus line time per row read, overlapped with the
    exposure. It gives the ROI and binning rates (REQ-ACQ-003).
- **`CameraCapabilities::check(settings)`** is the one place settings are checked. Every driver
  calls it.
- **The other descriptors:** `Travel` for the stage and the focus drive, position names for
  changers, and the modes and `PatternSet` for the modality unit, with `Modality::check`.
- **They serialize,** so that P2.3 can show them in Thing descriptions.

### Status is a watch, not a poll

- **`status()` returns a tokio `watch::Receiver`:**
  - it holds the latest status at once;
  - `changed()` wakes for the next;
  - changes a reader misses collapse into the latest;
  - the stream ends when the device is dropped.
- **One poller per device.** A driver that has to poll, such as GRBL's `?` at 5 Hz, polls once
  and feeds the watch. No consumer polls the device.
- **Collapsing suits status,** which is state, not a stream of events. A consumer that needs every
  sample, such as P2.1's pose timeline, reads promptly; at 5–50 Hz that is easy.
- **`AxisStatus { position, state, homed, ts }`:**
  - `state` is a `MotionState`: `Idle`, `Moving`, `Jogging`, `Homing`, `Holding`,
    `Alarm(code)` or `Disconnected`;
  - `ts` is on the service's shared `Clock`, so a pose can be matched to a frame's mid-exposure
    time.
- **Moves resolve when the axes have stopped at the target,** with the settled status. That is
  motion completion (REQ-STG-003).

### Frames go to a `FrameSink`

- **`start(pool, frames)`:** the camera fills pool slots and hands each `(FillGuard, FrameMeta)` to
  the sink. The meta has no pose or context yet.
- **The acquisition engine is the sink** (P2.1). It adds the pose and the context, publishes the
  frame, and passes it to the fan-out. Live frames, snaps and sequences all take this one path.
- **`CameraSettings::frame_meta`** builds the camera's part of the metadata, the same for every
  driver.
- **While acquiring,** the exposure and the gain change from the next frame. Other changes fail
  with `Busy` until the camera is stopped.
- **`CameraStats`** counts frames delivered, frames lost for want of a slot, and frames lost by the
  device (REQ-ACQ-005).

### Errors

`DeviceError` has:

- `Timeout`, `Disconnected`, `Fault`, `InvalidArgument`, `Busy` and `Unsupported`, each with a
  message;
- `Alarm(code)`;
- `Stopped`, for an operation that `stop()` interrupted.

### Mocks

The `mock` module has a mock of each trait that completes every operation at once, for engine
tests.

- **The mocks check their arguments** as drivers do: travel, ROI rules, positions, levels and
  modes.
- **They timestamp their status** on the clock they are given.
- **`fail_next(error)`** makes the next operation fail.
- **`MockCamera`:**
  - delivers one frame per `trigger()` in software mode, and one per `emit()` in any mode;
  - fills each frame's bytes with its number;
  - counts a frame as lost when no slot is free;
  - `lose_frame()` skips a number, as a grabber does.
- **`small_sensor()`** describes a 64 × 48 camera for tests.

### tokio in L2

`siq-devices` uses tokio's `sync` feature, for `watch`, and re-exports `watch`. The workspace's
tokio entry now names no features, and each crate names the ones it uses.

### Brief deviations

1. **`start(pool, ReleaseSink)` became `start(pool, Arc<dyn FrameSink>)`.**
   - A release sink is the driver's own business: the Euresys driver installs one to requeue its
     buffers (ADR-0021).
   - The camera needs somewhere to hand its frames.
2. **No `snap()`.** A snap is `trigger()` in software mode. Its frame reaches the sink like every
   other, so the pose stamping and the fan-out see it too. P2.1's `Snap` mode builds on it.
3. **`move_to`, `move_by` and `home` return the settled status,** not `()`.
4. **Added:**
   - `DeviceError::Stopped`;
   - `AxisStatus::homed`, since a GRBL reset during a move loses the position (P1.9);
   - `Camera::settings`;
   - `Changer`, which `FilterWheel` and `ObjectiveTurret` both are.
5. **`ModalityMode` moved from `siq-config` to `siq-types`,** so that the traits don't depend on
   the configuration crate. `siq-config` re-exports it, and its schema is unchanged.
6. **`MicrometersPerSecond` was added to `siq-types`,** for speeds and jog velocities.

## Alternatives considered

- **`async fn` in traits, with generic engines:** not dyn-compatible. Every engine would be
  generic over every device type, and the server couldn't choose drivers from the configuration.
- **The `async-trait` crate:** the same boxing, plus a procedural-macro dependency.
- **Blocking traits on threads:** serial I/O is async (P1.8), and a two-second move would block a
  runtime thread.
- **Status as a polled method:** every consumer would poll the device, multiplying the serial
  traffic.
- **Status as a broadcast channel:** a channel of events, whose lagging receivers get errors.
  Status is state.

## Consequences

- **Each operation boxes a future:** a few hundred nanoseconds, nothing at device rates. Frames
  don't take that path: `FrameSink::frame` is a plain call.
- **Engines can be tested on mocks,** without a simulator.
- **Drivers must keep their watch current,** including `Disconnected` when the link goes.

## Verification

- `siq-devices` tests:
  - `every_trait_is_dyn_compatible_and_its_futures_are_send`: every trait behind an `Arc<dyn …>`,
    with operations spawned on tokio;
  - the camera settings checks, and the frame-rate model;
  - the travel and modality checks;
  - the mock stage, camera and peripherals;
  - `a_status_watch_starts_at_the_latest_status_and_wakes_for_the_next`.
- The `mock_rig` example.
