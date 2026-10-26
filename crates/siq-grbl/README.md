# siq-grbl

SpectrumIQ's driver for the XYZ stage's GRBL 1.1 or grblHAL controller: a `siq-devices` `Stage`
over a serial line transport.

What it does:

- **Connects:** a soft reset, then the banner and `$$`. Then `$X` if the controller is locked and
  the configuration allows it, and `G21 G90`.
- **Moves to a target,** and resolves once the stage is still there. `G4 P0` marks the end of the
  motion, and a fresh `Idle` status confirms it. Optionally, it moves with a backlash-consistent
  approach, overshooting so that each axis arrives from its configured side.
- **Jogs** with `$J=` and jog cancel.
- **Stops** with a feed hold, then a reset once still, so that the position is kept.
- **Homes** with `$H`.
- **Polls the status,** and publishes it on a watch, timestamped on the shared clock.

The design is in
[ADR-0029](../../docs/adr/0029-grbl-motion-completion-polling-stop-and-recovery.md).

## Place in the architecture

- **Layer:** L2 Devices.
- **Depends on:**
  - `siq-devices`, for the trait it implements;
  - `siq-serial`, for the transport;
  - `siq-config`, for `[stage]`'s options;
  - `siq-types`;
  - `tokio` (`rt`, `sync`, `time`) and `tracing`.
- **Used by:** P2.3's service core, for `[stage] kind = "grbl"`, and for `kind = "sim"` against
  `siq-sim`'s emulator. P10.3 brings up the real stage.
- **Tested against** `siq-sim`'s GRBL emulator, a dev-dependency.
- **Phase:** P1.9.

## API

| Item | What |
|---|---|
| `GrblStage::connect(transport, options, clock)` | Connects over any `LineTransport`, and spawns the link task, which owns the transport. Its timing is on `clock`, which also timestamps the status. Fails if `$$` lacks the axes' settings, or reports in inches |
| `GrblStage` | A `siq_devices::Stage` and `Device`. The `Device` info's vendor is `GRBL` or `grblHAL`, and its firmware is the banner. `limits()` is −`$13x` to 0 µm per axis; `kinematics()` the slower of x's and y's `$11x` and `$12x` |
| `GrblOptions { poll, motion }` | From `&GrblStageConfig` or `&SimStageConfig` |
| `protocol::{parse_status, parse_setting, error, loses_position}` | GRBL's replies: status reports (with the last `WCO` kept by the caller), `$N=V` settings, `error:N` as a `DeviceError`, and the alarms that lose the position |

```rust
let transport = siq_serial::open("COM5", &PortSettings::default(), Framing::default())?;
let stage = GrblStage::connect(transport, GrblOptions::from(&grbl_config), clock).await?;
stage.home().await?;
let opts = MoveOpts { approach: Approach::Unidirectional, ..MoveOpts::default() };
let status = stage.move_to(Xyz::from_um(-20_000.0, -10_000.0, -1_000.0), opts).await?;
```

### Configuration

`[stage]`, for `kind = "grbl"` or `"sim"`:

- `poll_hz` (5);
- `[stage.motion]`:
  - `unlock` (true): send `$X` when the controller is locked;
  - `approach` (`[1, 1, 1]`): the direction, 1 or −1, from which each of x, y and z arrives in a
    unidirectional move;
  - `overshoot` (50 µm).

### Units and timing

- **The stage frame's µm are machine millimetres × 1000** (ADR-0018). The machine space is −travel
  to 0 per axis, as GRBL's.
- **Positions are where the controller has stepped the stage.** It is open-loop, so they aren't
  measured.
- **Deadlines are on the clock.** `G4 P0` may take twice the path's time at the slowest axis, plus
  2 s; `$H` twice the travel at the seek rate, plus 30 s.
- **The handshake and status replies have 5 s of real time.**

## Examples

| Example | What | Command |
|---|---|---|
| `grbl_drive` | Against the emulator, in real time: it connects, homes, moves directly and from one side, jogs, and stops a move. Each step prints the reported position and the true stage's. About 3 s | `cargo run -p siq-grbl --example grbl_drive` |
| `grbl_drive --port COMn` | Against a real controller: it connects, and prints the firmware, travel and status. `--baud` sets the rate (115 200). Only `--move` moves the stage: 1 mm along −X and back. It never homes. Manual | `cargo run -p siq-grbl --example grbl_drive -- --port COM5 --move` |

## Features

None.

## Testing

`cargo test -p siq-grbl` runs two binaries.

**`protocol`,** with proptests:

- status reports, of every state and substate, with extra fields;
- work positions, with the last offset kept;
- lines that aren't reports, which parse as none;
- settings, errors and alarm codes.

**`driver`,** against the emulator, on a clock that runs ten times faster than real time:

- connecting, unlocked or locked, then homing;
- timed moves, at a speed, and outside the travel;
- the unidirectional approach against injected backlash;
- stopping mid-move;
- jogs, cancelled or to the edge of the travel.

**`driver`,** against a scripted controller:

- grblHAL and work positions;
- a reply that never comes, and its late `ok`;
- a hard-limit alarm, and recovery by `stop`;
- a controller that reports in inches.

## Limitations

- **Send-and-wait only:** one line at a time. Character-counting streaming is a follow-up, for fly
  scans.
- **`homed` is the driver's own knowledge.** GRBL doesn't report it, so it starts false on every
  connection.
- **GRBL's default machine space is assumed.** A build with `HOMING_FORCE_SET_ORIGIN` has
  different travel.
- **The real board is tested only by hand,** in P10.3.
