# siq-ascii

SpectrumIQ's data-driven ASCII dialects: a device's serial protocol, described by a TOML file
instead of code. The focus drive is the first device to use one; later ASCII peripherals can
follow.

What it provides:

- **`Dialect`:** a protocol, read from a file and checked whole:
  - the terminators and the step size;
  - the timeouts;
  - how the device moves, and how the end of a move shows;
  - the replies every command may get;
  - the commands.
- **`Template`:** a line with fields, such as `MA {target:steps}`. It renders a line from values,
  and matches a line back into values, so one description serves the driver and the emulator.
- **`AsciiFocusDrive`:** a `siq-devices` `FocusDrive` over a dialect:
  - one command at a time, with polled, timestamped status;
  - moves that end by the dialect's means, polling or a completion reply;
  - stop, jog, homing, and a unidirectional approach.

`siq-sim`'s `DialectEmulator` is the device side of any dialect. The design is in
[ADR-0030](../../docs/adr/0030-data-driven-ascii-dialects.md) and its P1.10b amendment.

## Place in the architecture

- **Layer:** L2 Devices.
- **Depends on:**
  - `regex`, which the templates compile to;
  - `toml` and `serde`, for the file;
  - `siq-serial`, for the link;
  - `siq-devices`, for the `FocusDrive` trait;
  - `tokio` (`rt`, `sync`, `time`), `tracing`;
  - `siq-types`, `thiserror`.
- **Used by:**
  - `siq-sim`'s dialect emulator;
  - P2.3's server, which loads `[focus] dialect`, or the example, and drives the focus with
    `AsciiFocusDrive`.
- **Tested against** `siq-sim`'s dialect emulator, a dev-dependency.
- **Phases:** P1.10a (dialects), P1.10b (the driver).

## API

| Item | What |
|---|---|
| `Dialect::load(path)`, `Dialect::parse(text, origin)` | A checked dialect, or a `DialectError` with every problem as `path: message` |
| `Dialect::example()`, `Dialect::EXAMPLE` | `config/dialects/example-focus.toml`, parsed and as text |
| `Dialect { name, step, terminators, timeouts, motion, replies, commands }` | The file's sections, as below |
| `dialect.framing()` | The `siq_serial::Framing` for a link in this dialect |
| `dialect.error(line)` | The `DeviceError::Fault` that an error reply reports, such as `Example focus drive error 2: out of range` |
| `dialect.reports(field)` | Whether the status reply has the field, such as `moving` |
| `Template::parse(text)` | Text with fields `{name:kind}`. `render(&values, step)`, `matches(line, step)`, `fields()`, `has(name)` |
| `Value::{Length, Int, Flag, Text}`, `Values` | Fields' values by name. Lengths are µm, and speeds µm/s |
| `lengths([(name, µm)])` | `Values` of lengths |
| `AsciiFocusDrive::connect(transport, dialect, clock)` | Connects over any `LineTransport` in the dialect's framing: `init`, `identify`, then the status. It spawns the status poll. Its timing, and the status's timestamps, are on `clock` |
| `AsciiFocusDrive` | A `siq_devices::FocusDrive` and `Device`. Its vendor is the dialect's name, and its model, firmware and serial come from `identify`. `limits()` is `motion.travel` |

```rust
let dialect = Arc::new(Dialect::load(&focus.dialect)?);
let transport = siq_serial::open(&focus.port, &PortSettings { baud: focus.baud, ..Default::default() }, dialect.framing())?;
let drive = AsciiFocusDrive::connect(transport, dialect, clock).await?;
drive.home().await?;
let status = drive.move_to(Micrometers(1_250.0), MoveOpts::default()).await?;
```

### The driver's behaviour

- **Replies:** each command waits up to `timeouts.reply` for its reply. An error fails it, a
  completion reply is noted, and other lines are skipped.
- **The end of a move:** with `done = "poll"`, the status's `{moving}` clears. With `"reply"`,
  `complete` comes, then a fresh status. The deadline is twice the move's time at its speed, plus
  2 s, on the clock.
- **The status:** polled at `motion.poll_hz`, and timestamped at its arrival less half the round
  trip. The state is the device's `{moving}` flag, or the driver's word if it has none.
- **A unidirectional move** arrives going up, overshooting by `motion.overshoot` when it would go
  down.
- **`stop`:** a move or a command still queued fails `Stopped`.

### The file

```toml
name = "Example focus drive"
step = 0.05                         # µm per step

[terminators]
write = "\r"
read = "\r"

[timeouts]
reply = 300.0                       # ms, real time
home = 30000.0                      # ms, on the clock

[motion]
travel = [0.0, 12000.0]             # µm
speed = 2000.0                      # µm/s
done = "poll"                       # or "reply", with replies.complete
poll_hz = 10.0
overshoot = 10.0                    # µm

[replies]
ok = "OK"
errors = ["ERR {code:int} {message:text}"]
complete = "DONE"

[commands]
init = ["ECHO 0"]
identify = { send = "ID?", reply = "ID {model:text} {firmware:text} {serial:text}" }
move = { send = "MA {target:steps} {speed:steps}" }
status = { send = "ST?", reply = "ST {position:steps} {moving:flag} {homed:flag}" }
stop = { send = "HALT" }
home = { send = "HOME" }
jog = { send = "JOG {velocity:steps}" }
```

### Field kinds

| Kind | Holds | Written as |
|---|---|---|
| `steps` | a length, or a speed per second | whole steps of `step` |
| `um`, `um.N` | a length in µm | N decimals, 3 by default |
| `mm`, `mm.N` | a length in mm | N decimals, 4 by default |
| `int` | a whole number | as is |
| `flag` | true or false | `1` or `0` |
| `text` | text | as is |

### Each template's fields

| Template | Fields |
|---|---|
| `commands.move.send` | `{target}`, required, and `{speed}` |
| `commands.jog.send` | `{velocity}`, required |
| `commands.status.reply` | `{position}`, required; the flags `{moving}` (required with `done = "poll"`) and `{homed}` |
| `commands.identify.reply` | `{model}`, `{firmware}`, `{serial}`, as text |
| `replies.errors` | `{code:int}`, `{message:text}` |
| everything else | none |

A command without `reply` is answered with `replies.ok`.

## Examples

| Example | What | Command |
|---|---|---|
| `focus_dialect_demo` | Reads a dialect (the example by default), shows what its commands send and what a broken one reports, then drives the dialect emulator in real time. It homes, moves, moves down unidirectionally, jogs and stops, first by polling and then waiting for the completion reply, and prints the drive's status and the true focus. About 2 s | `cargo run -p siq-ascii --example focus_dialect_demo [-- dialect.toml]` |

`siq-sim`'s `dialect_emulator_repl` plays a session in the example dialect, and its `sim_rig` runs
the whole simulated rig.

## Features

None.

## Testing

`cargo test -p siq-ascii` runs two binaries.

**`dialect`:**

- templates rendered and matched back, for every kind, as proptests;
- template errors;
- the example dialect, against the file in `config/dialects/`;
- a broken dialect, with every problem and its path.

**`driver`,** against the dialect emulator, on a clock that runs ten times faster than real time:

- connecting and identifying;
- moves, by poll and by completion reply, timed, and outside the travel;
- the unidirectional approach;
- stop mid-move, two stops at once, and a jog;
- homing, by poll and by reply;
- the device's errors, a completion reply that never comes, and a device that never answers.

## Limitations

- **Templates are small.** Literal whitespace matches exactly, and fields can't be optional. A
  raw-regex reply, which couldn't be emulated, is left for a device that needs it.
- **A device's fields are fixed by command:** a new kind of command needs code here.
- **`move_by` is relative to the latest status,** which may be up to a polling period old.
- **The status is polled during a move as well as by the polling task,** so twice as often then.
- **The real drive is tested only by hand,** in P10.3, with its own dialect.
