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

`siq-sim`'s `DialectEmulator` is the device side of any dialect. The focus-drive driver follows in
P1.10b. The design is in [ADR-0030](../../docs/adr/0030-data-driven-ascii-dialects.md).

## Place in the architecture

- **Layer:** L2 Devices.
- **Depends on:**
  - `regex`, which the templates compile to;
  - `toml` and `serde`, for the file;
  - `siq-serial`, for the link's `Framing`;
  - `siq-devices`, for `DeviceError`;
  - `siq-types`, `thiserror`.
- **Used by:**
  - `siq-sim`'s dialect emulator;
  - P1.10b's `AsciiFocusDrive`;
  - P2.3's server, which loads `[focus] dialect`, or the example.
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

None in this crate yet: P1.10b adds `focus_dialect_demo`. `siq-sim`'s `dialect_emulator_repl`
plays a session in the example dialect.

## Features

None.

## Testing

`cargo test -p siq-ascii` runs `dialect`:

- templates rendered and matched back, for every kind, as proptests;
- template errors;
- the example dialect, against the file in `config/dialects/`;
- a broken dialect, with every problem and its path.

## Limitations

- **Templates are small.** Literal whitespace matches exactly, and fields can't be optional. A
  raw-regex reply, which couldn't be emulated, is left for a device that needs it.
- **A device's fields are fixed by command:** a new kind of command needs code here.
