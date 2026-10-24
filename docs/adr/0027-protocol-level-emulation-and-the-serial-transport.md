# ADR-0027: Protocol-level emulation, and the serial transport abstraction

- **Status:** Accepted
- **Date:** 2026-10-24
- **Phase:** P1.8
- **Related:** ADR-0012, ADR-0016, ADR-0026, REQ-SIM-002, REQ-STG-001

## Context

- **The stage is a GRBL or grblHAL controller on a serial port** (ADR-0016). The focus drive and
  the peripherals have ASCII protocols of their own (P1.10).
- **The drivers (P1.9, P1.10) must be tested without hardware.** That means against something that
  speaks the controller's protocol, with its quirks:
  - `ok` on planning, not on completion;
  - `G4 P0` as a motion barrier;
  - realtime bytes outside the lines;
  - alarms, locks and resets.
- **The simulated camera must see where the stage really is,** which differs from where an
  open-loop controller thinks it is (ADR-0012).

## Decision

### `LineTransport`

`siq-serial` defines one trait, which both the drivers and the emulators' tests use:

- `write_line(&str)`: the line, then the framing's terminator;
- `write_realtime(u8)`: one byte, outside the lines;
- `read_line(timeout)`: the next line, without its terminator (`Timeout` or `Closed` otherwise).

A few details of how it works:

- **`async fn` in the trait,** with `Send` futures, for drivers that are generic over their
  transport (`GrblStage<T: LineTransport>`). It isn't dyn-compatible: a device trait above it is.
- **`LineStream<S>`** implements it over any `AsyncRead + AsyncWrite` stream, with a `Framing`:
  - the write terminator (default `\n`);
  - the read terminator (default `\n`; a `\r` before it is dropped).
- **A timeout keeps what has arrived of a line,** so a reply split across reads still arrives
  whole.
- **The two transports:**
  - `SerialTransport` is a serial port through `tokio-serial`, with its baud rate, data bits,
    parity, stop bits and flow control;
  - `DuplexTransport` is an in-memory `tokio::io::duplex`. The emulator serves the other end.
- **Every line and realtime byte is traced at debug level,** with the link's name.

### The GRBL emulator

- **`GrblEmulator::run(port)`** serves the device's end of a link, as the controller does, until
  the link closes.
- **A synchronous core** (`Machine`) takes each byte and the clock's time, and returns the lines to
  send. The async loop around it reads, ticks the core every 2 ms, and writes. So the protocol, the
  planner and the motion are deterministic on a `ManualClock`.
- **The protocol subset:**
  - motion and modes: `G0`, `G1`, `G4`, `G20`/`G21`, `G90`/`G91`, `F`, and comments;
  - system commands: `$`, `$$`, `$N=V`, `$X`, `$H`, `$J=…`;
  - realtime bytes: `?`, `!`, `~`, 0x18 and 0x85;
  - replies: `ok`, `error:N` (1, 2, 3, 5, 9, 15, 20, 22, 28), `ALARM:2` (soft limit), `ALARM:3`
    (reset in motion), the banner, the lock message, and status reports
    `<State|MPos:x,y,z|FS:f,0>` in whole steps.
- **The planner, as GRBL's:**
  - a motion line is queued and answered at once;
  - a `G4` or `$H` holds up the lines after it, and is answered when it ends. `G4 P0` therefore
    answers when the moves before it have finished, which P1.9's motion completion relies on.
  - an empty line is answered `ok`, as GRBL does, so drivers end lines with `\n` alone.
- **Motion:**
  - each block is a trapezoidal move along a line, from rest to rest;
  - its speed and acceleration are limited by every axis's `$11x` and `$12x`, scaled by that
    axis's share of the direction;
  - soft limits keep the machine within `−$13x` to 0 per axis (GRBL's homed machine space);
  - homing seeks machine zero at `$25`;
  - with homing on, the controller starts locked until `$H` or `$X`. A reset in motion loses the
    position (ALARM:3), so homing is needed again.
- **The `$` settings** are GRBL 1.1's numbers, with defaults for a stage of 100 × 75 × 20 mm.

### The true stage and the injected errors

The emulator moves `TrueState`'s stage on every tick, from the commanded position:

```text
true = commanded × (1 + scale error) − direction × backlash / 2 + the last move's random error
```

- **`direction`** is the sign of each axis's last motion, so approaching from one side always
  lands in the same place.
- **The random error** is drawn per completed move, from a seeded hash.
- **Units:** the stage frame is machine millimetres × 1000 (µm). The driver's axis mapping is
  P1.9's configuration.
- **`GrblErrors::from(&SimErrors)`** takes the configuration's `[sim.errors]`.
- **Reply latency** delays every reply, on the clock.

### Brief deviations

1. **Feed hold and jog cancel stop at once,** rather than decelerating. The position at the stop
   is exact, so tests can state it.
2. **Missed steps aren't emulated.** The brief lists them as optional.
3. **Blocks start and stop at rest:** there is no junction blending. That suits stop-and-go
   scanning, and makes the timing exact to state.

## Alternatives considered

- **Mocking the driver's trait (P1.3's mocks):** doesn't exercise the driver's protocol code,
  which is where the bugs are.
- **A virtual COM port pair, with the emulator in a separate process:** closer to the hardware,
  but slower and harder to time. `SerialTransport` and the emulator meet over a duplex instead,
  and a real port is P10.3's.
- **Running grblHAL's own simulator:** C code, and not deterministic under a test clock.

## Consequences

- **The P1.9 driver is tested end to end against the emulator,** over the same `LineTransport` it
  uses with the port.
- **The camera sees the true stage,** so stitching and calibration have real errors to find.
- **The emulator's motion is an idealized GRBL:** P10.3 measures the real one's timing.

## Verification

- `siq-serial` tests (`lines`):
  - framing, and partial lines across a timeout;
  - realtime bytes;
  - a closed link, and `\r` framing;
  - a port that isn't there.
- `siq-sim` tests (`grbl`):
  - the golden transcripts `protocol.txt`, `motion.txt`, `alarms.txt` and `hold_jog.txt`, played
    through a `DuplexTransport` on a `ManualClock`;
  - `a_diagonal_move_takes_its_limiting_axis_s_time_and_the_true_stage_follows`;
  - `the_true_stage_carries_backlash_scale_error_and_random_error`.
- The `grbl_emulator_repl` example.
