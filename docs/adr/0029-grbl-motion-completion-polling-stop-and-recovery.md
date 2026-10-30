# ADR-0029: GRBL motion completion, status polling, and stop and recovery

- **Status:** Accepted
- **Date:** 2026-10-24
- **Phase:** P1.9
- **Related:** ADR-0016, ADR-0018, ADR-0023, ADR-0027, REQ-STG-001, REQ-STG-003, REQ-STG-004,
  REQ-STG-005

## Context

- **The stage is a GRBL 1.1 or grblHAL controller on a serial port** (ADR-0016). The engines drive
  it through `siq-devices`' `Stage` trait (ADR-0023). Scans are stop-and-go: move, settle, expose.
- **GRBL's `ok` means "planned", not "done".** A status report says what the controller is doing
  now, but nothing ties it to a line.
- **GRBL is open-loop.** Its position is the steps it has sent, not a measurement. Backlash, a
  missed step or an alarm are invisible to it.
- **Some stops lose the position.** A reset in motion, a hard limit or a failed homing leave the
  machine position untrustworthy until the stage is homed again.
- **The driver must work on the emulator (ADR-0027) and on real boards.** Those differ: real ones
  decelerate before they hold, may restart when the port opens, and send the work offset only now
  and then.

## Decision

### The link task

One task owns the transport. The stage talks to it through two queues.

- **Lines** go one at a time: the next is sent once the previous has its `ok` or `error:N`, or
  its deadline on the clock has passed. This is the brief's send-and-wait. Character-counting
  streaming is a follow-up.
- **Urgent requests** go out at once, ahead of a line that waits: a realtime byte (`!`, `~`,
  0x85), a status poll, or a soft reset, which is answered when the banner returns.
- **Polling:** `?` at `poll_hz` (default 5 Hz) on the clock, and on demand. Each report updates
  the `StageStatus` watch.
- **A report's timestamp is its arrival less half the round trip** since its `?`.
- **Replies that arrive too late are dropped.** A line that timed out still has a reply due. The
  link counts those, and drops that many `ok`s or errors, so that a late reply can't answer the
  next line. A reset clears the count, since the controller forgets every line.
- **A line queued before a reset isn't sent.** The link counts the resets it sends, and each line
  carries the count from when its operation began. A move interrupted by `stop` therefore can't
  send its remaining lines after the reset.
- **An `ALARM:N` fails the line that waits.** It stores the code for the status, and clears
  `homed` for the alarms that lose the position: 1, 3, and 6 to 9.
- **When the link closes,** the waiting line fails `Disconnected`, and so does the status.

### Connecting

1. Send a soft reset, and wait up to 5 s for the banner (`Grbl…`, including `GrblHAL…`). Then wait
   for 250 ms of quiet. A board that restarts when its port opens prints its banner late, and the
   reset may be lost in its bootloader. The power-up banner then serves, since power-up is a reset
   too.
2. Read `$$`:
   - the rates `$110–112`, accelerations `$120–122` and travel `$130–132`, which are required;
   - the homing seek rate `$25`, which is optional;
   - `$13=1`, reports in inches, which is refused.
3. Spawn the link, then re-sync (below).

### Re-sync

Re-sync runs after connecting, after a reset, and after homing:

- If the status is `Alarm` and `[stage.motion] unlock` is true (the default), send `$X`.
- Unless the controller is still locked, send `G21 G90`.
- Take the reported position as the commanded one, with each axis's last direction unknown.

### Moves

- **`move_to`** checks the target against the travel. Machine space is −`$13x` to 0 per axis,
  GRBL's default build. A target outside it is refused before anything is sent.
- **The lines:** `G0`, or `G1 … F…` when a speed is given. The axes' millimetres are the stage
  frame's µm ÷ 1000 (ADR-0018).
- **Completion:** `G4 P0`, which GRBL answers once the motion before it has finished. Then a
  status asked for after that `ok` must say `Idle`. An `Alarm` fails the move.
- **The deadline:** `G4 P0`'s comes from the path. It allows twice the time at the slowest axis's
  rate and acceleration, plus 2 s, on the clock.
- **`move_by`** is relative to the last commanded target. After a jog or a stop, which leave that
  unknown, it is relative to the reported position.

### The backlash-consistent approach

`Approach::Unidirectional` makes each axis arrive moving in its configured direction:
`[stage.motion] approach = [1, 1, 1]`, overshooting by `overshoot` (50 µm).

- An axis arrives the wrong way if it would move against its direction.
- If it doesn't move, it arrives the wrong way if its last move went against its direction, or if
  that direction is unknown.
- Such axes first go to the target less the overshoot, clamped to the travel, in one move. The
  other axes go straight to the target in that move. Then every axis goes to the target.
- So backlash is taken up the same way on every arrival (REQ-STG-005).

### Jogs

- `jog(v)` cancels a running jog (0x85), and waits for `Idle`.
- It then sends `$J=G90 … F|v|` to the edge of the travel along `v`. The jog runs until it is
  cancelled, or stops at the edge without a soft-limit alarm.
- `jog(0)` only cancels.

### Stop and recovery

1. **A feed hold** (`!`) stops the axes under control, or cancels a jog.
2. **Wait until still:** `Hold:0`, `Idle` or `Alarm`.
   - `Hold:1`, a hold still decelerating, is parsed as running, and so are `Door:2` and `Door:3`.
     A reset then would lose the position.
3. **Then a soft reset** clears the planner and keeps the position, since nothing is moving. GRBL
   then comes back idle, or still locked if it was.
4. **Re-sync.** The interrupted move fails `Stopped`.

- **Homing:** `$H`, with a deadline of twice the whole travel at the seek rate, plus 30 s. On its
  `ok`, `homed` is set and the alarm is cleared, then re-sync.
- **`homed`** is the driver's knowledge. GRBL doesn't report it. It starts false on every
  connection, is set by `$H`, and is cleared by an alarm that loses the position.

### Status

- **`StageStatus { position, state, homed, ts }`,** where the state maps as follows:

  | GRBL | `MotionState` |
  |---|---|
  | `Idle`, `Check`, `Sleep` | `Idle` |
  | `Run`, and holds or doors still stopping | `Moving` |
  | `Jog` | `Jogging` |
  | `Home` | `Homing` |
  | `Hold:0`, `Door:0`, `Door:1` | `Holding` |
  | `Alarm` | `Alarm(code)`. The code is the last `ALARM:N`, or 0 for the lock at power-up |

- **`WPos` is made `MPos` with the last `WCO`.** GRBL sends the offset only every 10 to 30
  reports, and in the first report after a reset.

### The open-loop caveat

- **Every position the driver reports is where the controller has stepped the stage,** not where
  it is.
- **Backlash, scale error and missed steps** are not seen. Calibration (P5.x) and stitching
  measure them from images.
- **The unidirectional approach** makes backlash repeatable, not zero.
- **Homing** is the only way back to a trusted position after a stop that loses it.

## Alternatives considered

- **Polling for `Idle` alone, without `G4 P0`:** a report taken just after a line's `ok` can still
  say `Idle`, before the planner starts the block. `G4 P0` ties completion to the lines.
- **Character-counting streaming:** it suits continuous scans, but stop-and-go doesn't need it. It
  is a follow-up for P6.x's fly scans.
- **A reset alone for `stop`:** this is fast, but loses the position whenever the stage is moving.
- **Keeping the stage generic over its transport,** as ADR-0027 sketched. The link task owns the
  transport instead, so `GrblStage` is one type behind `dyn Stage`. Only `connect` is generic.

## Consequences

- **A move resolves only when the stage is still at its target,** as the controller knows it, and
  with a timestamped status (REQ-STG-003).
- **`stop` keeps the position and `homed`** when it can, and says so when it can't.
- **Each move costs at least one round trip for `G4 P0`, and one for the confirming status.**
  P10.3 measures that on the real board.
- **The driver assumes GRBL's default machine space:** −travel to 0, without
  `HOMING_FORCE_SET_ORIGIN`. A board built otherwise needs its limits configured, in P10.3.
- **The emulator was brought closer to GRBL** (ADR-0027, amended):
  - a reset once held no longer alarms;
  - a reset at rest keeps the state, so an unlocked controller stays unlocked.

## Verification

`siq-grbl` tests:

- **`protocol`,** with proptests:
  - status reports of every state and substate, with extra fields and grblHAL's fourth axis;
  - `WPos` with a kept `WCO`;
  - lines that aren't reports;
  - settings, errors and the alarms that lose the position.
- **`driver`,** against the emulator, on a clock running ten times faster than real time:
  - connecting, with and without unlocking, then homing;
  - moves, timed and at a speed, and a target outside the travel;
  - the unidirectional approach against injected backlash;
  - stop mid-move;
  - jogs, cancelled and to the edge of the travel.
- **`driver`,** against a scripted controller:
  - grblHAL, and `WPos`;
  - a reply that never comes, and its late `ok`;
  - `ALARM:1` and recovery by `stop`;
  - `$13=1` refused.
- **The `grbl_drive` example,** and its manual `--port` path.

## Amendment (P10.3, 2026-10-29): the controller's settings checked, the round trip, and the repeatability

**Context.** M10 brings up the instrument's stage. A GRBL controller's settings live in its own
EEPROM: a firmware update, or a `$100=` typed in a terminal, changes how far a step moves the
stage, and nothing tells the driver. The owner kept the stage simulated for now, so P10.3 was
built on the emulator.

**Decision.**

- **`[stage.expect]`:** the configuration names the settings the controller must have, such as
  `"$100" = 1600.0`. On connecting, after `$$`, the driver compares them to a millionth. A
  controller that differs is refused, every difference named, before `$X` or any move. The
  settings to name are steps/mm, maximum rates, accelerations, travel, soft limits (`$20`) and
  homing (`$22`). `config/sim.toml` names the emulator's.
- **`GrblStage::settings()`** gives the whole `$$`, and `grbl_drive --port` prints it as a
  `[stage.expect]` section to copy.
- **The status's round trip,** from `?` to its report, is kept, for reports that were asked for:
  their count, least, mean and most (`GrblStage::round_trip()`). The driver already stamps a
  report at its arrival less half of it.
- **The repeatability** is measured with the backlash, in the stage–camera calibration (ADR-0049):
  each arrival at the backlash's place from −x −y, after the first, is registered against the
  first. The repeatability is the arrivals' standard deviation on each stage axis, the first's
  included (`siq_calib::repeatability`). It is kept in the calibration record, as
  `repeatability_um`. The `motion_repeatability` example measures both over more repeats.
- **Ports** are discovered (`siq_serial::ports`): `spectrumiq-server --list-ports`, the service's
  `GET /api/siq/system/ports`, and the web app's Settings page. They show the device the
  configuration puts on each port, and the configured ports the computer lacks.

**Alternatives considered.**

- **Writing the expected settings to the controller.** That changes a machine's setup unasked,
  and a wrong configuration would spread to the controller. Refusing and naming the difference
  leaves the choice to whoever set the stage up.
- **Repeatability as each arrival's distance from the first.** The first arrival's own error
  enters every term: on the emulator it read 1.2 µm against 0.5 injected. The arrivals' spread
  about their mean is the stage's own.

**Consequences.**

- A configuration with `[stage.expect]` fails to start on a controller that was changed. The
  service's log names what differs.
- Old calibration records have no `repeatability_um`, and read as not measured.

**Verification.**

- `siq-grbl` `driver`: `a_controller_s_settings_are_checked_and_its_round_trips_kept`. A
  difference and an unknown setting are named, and a setting that agrees isn't. One that agrees
  connects, with its settings and round trips kept.
- `siq-calib` `calib`: `the_repeatability_is_the_spread_of_the_arrivals_from_one_side`.
- `siq-server` `calibration`: the repeatability recorded, under 1.5 µm on the stage's 0.5 µm of
  noise, mono and colour.
- On the emulator: `grbl_drive` measures a round trip of 10.2–10.8 ms. `motion_repeatability`, with
  20 repeats, measures a repeatability of 0.61 and 0.57 µm against 0.5 injected, and a backlash of
  2.05 and 1.87 µm against 2.
