# siq-acquire

SpectrumIQ's acquisition engine. It owns the camera, the frame pool and the fan-out, and gives
every frame its pose and context:

- **Modes:**
  - live view, free-running;
  - snaps, software-triggered once the stage and focus are still;
  - sequences of snaps, with a hook after each frame.

  A change of region or binning stops the camera, reconfigures it and starts it again.
- **Frame numbers go on across restarts,** so a gap always means lost frames.
- **Every frame is stamped** with the context and the pose in force at its mid-exposure. The pose
  comes from a timeline of the stage's and focus drive's readings: settled, interpolated with an
  uncertainty, or unknown.
- **Frames that need their pose wait for it:** a captured frame, a snap, or a sequence's frame
  waits for a reading after it, for up to 500 ms. Live frames go at once, and frames are published
  in order.
- **Captures** hand over the next frames, marked for the recorder.
- **Statistics:** the rate, the frames lost, and each consumer's lag.

The design is in
[ADR-0031](../../docs/adr/0031-frame-pose-association-and-context-stamping.md).

## Place in the architecture

- **Layer:** L4 Engines.
- **Depends on:**
  - `siq-devices`, for the `Camera` trait and the status watches;
  - `siq-buffer`, for the pool and the fan-out;
  - `siq-types`;
  - `tokio` (`rt`, `sync`, `time`), `tracing`.
- **Used by:**
  - P2.2's previews, which subscribe;
  - P2.3's server, whose camera Thing starts live view and snaps;
  - P2.4's channel switching, which sets the context;
  - the recorder and the workflows of M3 onwards.
- **Tested with** `siq-sim`'s camera and `siq-grbl` against the GRBL emulator, which are
  dev-dependencies.
- **Phase:** P2.1.

## API

| Item | What |
|---|---|
| `Acquisition::new(camera, &PoolConfig, clock)` | An engine for any `Camera`, with its own pool, whose slots must fit the largest frame. Its timing is on `clock`, the camera's and drivers' clock |
| `follow_stage(watch)`, `follow_focus(watch)` | Feeds the drivers' status into the pose timeline |
| `subscribe(name, policy, filter)` | A fan-out consumer of the published frames (ADR-0022) |
| `start_live()`, `stop()` | Free-running live view, and stopping |
| `snap(save)` → `FrameLease` | Waits for the stage and focus to be still, triggers, and gives back the frame with its pose decided. It is published too |
| `sequence(n, save, hook)` | `n` snaps, calling `hook(index, frame)` before each next one |
| `configure(settings)` | The exposure and gain at once; anything else stops, reconfigures and restarts. The engine keeps its own trigger mode |
| `set_context(AcquisitionContext { channel, objective, calibration })`, `context()` | What frames exposed from now on are taken with |
| `capture_next(n, save)` → `mpsc::Receiver<FrameLease>` | The next `n` frames to arrive, marked `flags.save` if `save` |
| `stats()` → `AcquisitionStats` | Mode, rate, frames published, lost for want of a slot and by the device, waiting for their pose, and each consumer's `SubscriberStats` |
| `timeline()` → `&PoseTimeline` | `pose_at(t)` → `PoseAt { pose, quality, moving }`, `decided(t)`, `still()` |
| `POSE_WAIT` | 500 ms on the clock: the longest a frame waits for its pose |

```rust
let acquisition = Acquisition::new(camera, &pool_config, clock)?;
acquisition.follow_stage(stage.status());
acquisition.follow_focus(focus.status());
let mut preview = acquisition.subscribe("overview", Policy::LatestOnly, |_| true);
acquisition.start_live().await?;
stage.move_to(target, MoveOpts::default()).await?;
let frame = acquisition.snap(true).await?;   // PoseQuality::Settled
```

## Examples

| Example | What | Command |
|---|---|---|
| `acquire_live` | Live view of `config/sim.toml`'s simulated camera, on the `fast` profile, while the GRBL driver moves the stage against its emulator. Every 250 ms it prints the rate, frames lost, and the latest frame's pose and quality, then it snaps a settled frame. About 3 s; smoke mode uses a 1024 × 768 sensor | `cargo run --release -p siq-acquire --example acquire_live` |

## Features

None.

## Testing

`cargo test -p siq-acquire` runs two binaries.

**`timeline`,** with proptests:

- interpolation between readings, with any path between them within the uncertainty;
- settled only between still readings at the same place;
- carrying on past the last reading;
- the focus with the stage;
- order and capacity.

**`acquire`,** with `siq-sim`'s camera and the GRBL driver against its emulator, on a clock ten
times faster than real time:

- a still frame's pose within the injected error, with its context;
- moving frames, interpolated within their uncertainty;
- numbering across modes and a reconfiguration;
- captures, and lost frames counted as gaps;
- the context in force at each frame's mid-exposure.

## Limitations

- **A live frame's pose can be carried on past the last reading,** and overshoot the end of a
  move by up to its stated uncertainty.
- **The pool is the engine's,** sized when it is made. A frame larger than its slots is refused
  when the camera starts.
- **Stop the engine before dropping it,** or the camera runs on, discarding frames.
- **Metrics:** `stats()` is a snapshot. The registry's gauges come with P2.4's system routes.
