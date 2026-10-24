# ADR-0030: Data-driven ASCII dialects

- **Status:** Accepted
- **Date:** 2026-10-24
- **Phase:** P1.10a
- **Related:** ADR-0012, ADR-0016, ADR-0018, ADR-0027, REQ-STG-002, REQ-SIM-002

## Context

- **The focus drive has a documented ASCII serial protocol, but which one isn't known yet**
  (ADR-0016). Later peripherals may have ASCII protocols too.
- **The protocols differ in detail, not in kind:**
  - a command with a number in some unit;
  - a reply to match;
  - an acknowledgement or an error;
  - a status to poll;
  - a move that ends when a flag clears, or when the device says so.
- **The driver must be tested without the device** (ADR-0012). An emulator written by hand per
  device would drift from the driver's idea of the protocol.

## Decision

### A dialect file

A TOML file describes the protocol. `config/dialects/example-focus.toml` is the example, and the
real drive's follows in P10.3.

| Section | Keys |
|---|---|
| top level | `name` (the `DeviceInfo` vendor) and `step`, the µm in one step |
| `[terminators]` | `write`, after each line sent (`\r`), and `read`, the one character that ends a line received (`\r`) |
| `[timeouts]` | `reply`, in real time (500 ms); `home`, on the clock (60 s) |
| `[motion]` | `travel` `[min, max]` µm; the default `speed`, µm/s; `done`, `"poll"` or `"reply"`; `poll_hz` (10); `overshoot` (10 µm) for a unidirectional approach |
| `[replies]` | `ok`, the acknowledgement; `errors`, with optional `{code:int}` and `{message:text}`; `complete`, the completion reply |
| `[commands]` | `init` lines; `identify`; `move`, `status` and `stop`, which are required; `home` and `jog` |

- **A command is `{ send, reply }`.** Without a reply, `replies.ok` answers it.
- **Each command's fields are fixed:**

  | Template | Fields |
  |---|---|
  | `move.send` | `{target}`, required, and `{speed}` |
  | `jog.send` | `{velocity}` |
  | `status.reply` | `{position}`, required, and the flags `{moving}` and `{homed}` |
  | `identify.reply` | `{model}`, `{firmware}` and `{serial}`, as text |
  | the error replies | `{code}` and `{message}` |
  | every other template | none |

### Templates, for both directions

A template is text with fields, `{name:kind}`:

| Kind | Holds | Written | Matched |
|---|---|---|---|
| `steps` | a length, or a speed per second | whole steps of `step` µm | `[-+]?\d+` |
| `um`, `um.N` | a length in µm | N decimals (3) | a decimal |
| `mm`, `mm.N` | a length in mm | N decimals (4) | a decimal |
| `int` | a whole number | as is | `[-+]?\d+` |
| `flag` | true or false | `1` or `0` | `[01]` |
| `text` | text | as is | `.+?` |

- **A template renders a line from values,** for the commands the driver sends and the replies the
  emulator sends.
- **It also compiles to an anchored regex,** with the literal text escaped and a named group per
  field. That regex matches a line back into values, for the replies the driver reads and the
  commands the emulator reads.
- **Values are µm and µm/s at the boundary.** `steps`, `um` and `mm` convert them.
- **Braces only delimit fields.** A field is named with a–z and `_`, and appears once.

### Checking a dialect

- **A syntax error, an unknown key or a bad template** is a TOML error, with its line and column.
  A template is checked as it is deserialized.
- **Every other rule is checked over the whole file,** and every problem is reported as
  `path: message`:
  - a positive step, speed and timeouts;
  - a single read terminator;
  - an ordered travel, and a polling rate of at most 100 Hz;
  - each template's fields, with their kinds, and the required ones;
  - `done = "poll"` needs a `{moving:flag}` in the status, and `"reply"` needs `replies.complete`.

### The emulator

`siq_sim::dialect::DialectEmulator` serves the device end of a link from the same `Dialect`:

- **Matching:** each line is matched against the commands' send templates, in turn, and answered
  with its reply rendered, or `ok`.
- **Errors:** a line that matches nothing gets the first error template with code 1, and a move
  outside the travel or at no speed gets code 2.
- **Motion:**
  - moves run at their speed with no acceleration;
  - jogs run until stopped, or to the edge of the travel;
  - homing runs to the bottom of the travel at the default speed, and sets `homed`.
- **Completion:** with `done = "reply"`, a move or homing that ends sends `complete`. A stop or a
  jog doesn't.
- **The true focus** is written to `TrueState` on every 2 ms tick. The reported position is in
  the dialect's units, so in whole steps for the example.

### Brief deviations

1. **Replies are templates, not regexes.** The brief asked for reply regexes with named groups. A
   template compiles to exactly such a regex, but it can also be rendered, so the emulator needs
   nothing more. A raw-regex form, for replies that a template can't describe, is left until a
   real device needs it (P10.3). Such a reply couldn't be emulated.
2. **Error patterns are templates too,** for the same reason.
3. **P1.10 was split.** This phase is the dialect and the emulator. The driver and the peripheral
   simulators are P1.10b.

## Alternatives considered

- **A driver written in Rust per device:** that means code, review and an emulator for each device.
  Dialects suit protocols that differ in detail.
- **Regexes for replies, plus reply templates for the emulator:** two descriptions of each reply,
  which can disagree.
- **A scripting language (Lua, Rhai):** more power than these protocols need, and a dependency
  and an attack surface.

## Consequences

- **A new ASCII device whose protocol fits takes a TOML file,** checked when loaded, and gets an
  emulator for free.
- **The template language is deliberately small.** Literal whitespace matches exactly, and fields
  can't be optional. A device that needs more extends it, or gets the raw-regex escape hatch.
- **The emulator is idealized:** no acceleration, no reply latency and no positioning error.

## Verification

- **`siq-ascii` `dialect` tests:**
  - `a_length_renders_and_matches_back_within_its_precision` and
    `numbers_flags_and_text_match_back_exactly` (proptests);
  - `a_template_s_fields_are_named_with_their_kinds`, with the template errors;
  - `the_example_dialect_is_valid_and_is_the_file_in_config`;
  - `a_broken_dialect_reports_every_problem_with_its_path`.
- **`siq-sim` `dialect` tests,** on a manual clock:
  - `commands_get_their_replies_and_others_an_error`;
  - `moves_jogs_and_homing_run_at_their_speeds_and_the_true_focus_follows`;
  - `with_done_by_reply_a_move_or_homing_that_ends_sends_its_completion`.
- **The `dialect_emulator_repl` example.**

## Amendment (P1.10b, 2026-10-24): the focus driver

`siq_ascii::AsciiFocusDrive<T: LineTransport>` is a `siq-devices` `FocusDrive` over a dialect.

- **One command at a time.** A `tokio` mutex holds the transport, and each command waits up to
  `timeouts.reply`, in real time, for its reply. Every line received is matched, in this order:
  1. the reply, which answers;
  2. an error, which fails the command, through `Dialect::error`;
  3. the completion reply, which is noted for the move that waits;
  4. anything else, such as an echo, which is skipped.
- **Connecting** sends `init`, then `identify` if the dialect has it, for `DeviceInfo`'s model,
  firmware and serial. Then it reads the status.
- **The status:**
  - it is polled at `motion.poll_hz` by a task that ends when the drive is dropped;
  - each reply is timestamped at its arrival less half the round trip;
  - a disconnected link sets the state to `Disconnected`.
- **The state:**
  - the device's `{moving}` flag says whether the drive moves;
  - what the driver last asked says which kind of motion: `Moving`, `Jogging` or `Homing`;
  - a status without the flag takes the driver's word;
  - `homed` is the device's `{homed}` flag, or the driver's knowledge from `home()`.
- **A move:**
  - it sends `move` with `{target}` and `{speed}` (the dialect's default speed if none);
  - with `done = "poll"`, it polls until the flag clears;
  - with `done = "reply"`, it waits for `complete`, between commands, with the link held 20 ms at
    a time, then reads a fresh status;
  - the deadline, on the clock, is twice the time at the speed, plus 2 s;
  - a target outside `motion.travel` is refused before anything is sent.
- **The unidirectional approach** arrives going up. A move that would go down, or stay, first
  goes `motion.overshoot` below the target, clamped to the travel.
- **`stop`:**
  - it counts the stops, so that a move or a queued command that sees a new one fails `Stopped`;
  - it sends `stop`, then waits for a still status, for up to 5 s;
  - a stop that another overtakes still succeeds.
- **`jog(v)`** sends `jog` with `{velocity}`, and `jog(0)` stops. **`home`** sends `home` and waits
  up to `timeouts.home`.
- **`move_by`** is relative to the latest status.
- **Verified by** `siq-ascii`'s `driver` tests, against the emulator by poll and by reply, with
  pairs of dialects that disagree for the errors and timeouts.
