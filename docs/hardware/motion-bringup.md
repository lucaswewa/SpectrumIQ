# Motion bring-up runbook

How to bring up the XYZ stage's GRBL or grblHAL controller and the focus drive, from the serial
port to the service, and what to measure (P10.3; ADR-0029 and ADR-0030, amended). P10.3 was built
and checked on the emulators. The instrument's stage and focus drive are still simulated: each
step's numbers below are the emulator's, until the owner runs them on the real ones. Run each step
from the repository's root.

## 1. What is needed

- The controller's USB serial adapter plugged in, and its driver installed.
- The stage clear to move: nothing on it that a move could crush, and its limit switches wired.
- A textured slide under the objective, in focus, for the repeatability (step 6).
- For the focus drive: its vendor's protocol document. Its dialect file is written from it
  (step 7).

## 2. Find the ports

```text
cargo run --release -p spectrumiq-server -- --list-ports
```

Each port's name and what it is: a USB adapter's vendor and product ids, its maker and its product.
Unplugging the controller and listing again tells which one it is. On this PC, on 2026-10-29:

```text
COM7     USB 0403:6001 FTDI USB Serial Port (COM7)
```

COM7 is an FTDI adapter. It wasn't opened: the owner chose to keep the stage and the focus drive
simulated for now. The web app's **Settings** page shows the same list, with the device the
configuration puts on each port, and the configured ports the computer doesn't have.

## 3. Talk to the controller

```text
cargo run --release -p siq-grbl --example grbl_drive -- --port COM7
```

It connects, prints the firmware's banner, the travel, the status, and the controller's `$$` as a
`[stage.expect]` section; then the status's round trip over 2 s of polling. It doesn't move or
home. Check the banner says `Grbl` or `GrblHAL`, and that `$13=0`: the driver refuses a controller
that reports in inches.

The round trip is how long a `?` takes to bring its status report. The driver stamps each report
at its arrival less half the round trip, so a long or uneven one shows in the frames' poses. On
the emulator it is 10.2–10.8 ms: its injected 5 ms, and the driver's 5 ms step reading the link.
That is against the 200 ms between polls at 5 Hz.

## 4. Check the controller's settings

Copy the printed `[stage.expect]` into the instrument's configuration, under its `[stage]`, and
keep the settings that matter:

| Setting | What | Why it is checked |
|---|---|---|
| `$100`–`$102` | Steps/mm | A wrong value moves the stage the wrong distance, and no error says so |
| `$110`–`$112` | Maximum rates, mm/min | The driver's move timeouts come from them |
| `$120`–`$122` | Accelerations, mm/s² | As are its moves' durations |
| `$130`–`$132` | Travel, mm | The stage's limits, and the soft limits' |
| `$20` | Soft limits on | A move past the travel is refused rather than run into the switch |
| `$22` | Homing on | Positions mean something only after homing |

Each value is the stage's maker's, or measured (step 6). On connecting, the service compares the
controller's `$$` with them, and refuses a controller that differs, naming every difference:

```text
the controller's settings aren't as expected: $100 is 1600, not 800; $999 isn't one of its
settings. Set them, or change the configuration's [stage.expect]
```

A controller's settings change with a firmware update, or by hand from a terminal. The check
catches it before the first move. `config/sim.toml` expects the emulator's values.

## 5. Home, and move

```text
cargo run --release -p siq-grbl --example grbl_drive -- --port COM7 --move
```

It moves 1 mm along −X and back, from where the stage is, without homing. Then, with the service
running on the instrument's configuration (`[stage] kind = "grbl"`, `port = "COM7"`):

1. Take the token, and home from the Live page's stage controls, or with
   `POST /api/v1/stage/home`. Watch the stage reach its switches on each axis, and back off.
2. Move to a few places within the travel. Check the position the status gives against a ruler or
   the stage's scale.
3. Start a long move, and press **STOP**. The stage holds and keeps its position: the status says
   `idle`, still homed.
4. Jog with the arrow keys, and let go: the jog stops.

## 6. Repeatability and backlash

With a textured slide in focus, and the stage–camera calibration done for the objective (M5):

```text
cargo run --release -p siq-server --example motion_repeatability -- -c <instrument.toml> --repeats 20
```

One place is approached 20 times from −x −y and from +x +y, a frame at each arrival:

- **the repeatability** is the standard deviation, on each stage axis, of the arrivals from the
  same side, each registered against the first;
- **the backlash** is the mean shift between arriving from either side.

Shifts are turned into µm by the calibration in force.

The stage–camera calibration measures both too, from 4 repeats, and keeps them in the
calibration registry: the record's `backlash_um` and `repeatability_um`. Set
`[stage.motion] overshoot` above the backlash, so that a backlash-consistent move takes it up.

On the simulated stage (`config/sim.toml`: 2 µm of backlash and 0.5 µm of position noise), with 20
repeats:

| What | Found | Injected |
|---|---|---|
| Repeatability, x and y | 0.61 and 0.57 µm | 0.5 µm |
| Backlash, x and y | 2.05 and 1.87 µm | 2 µm |

## 7. The focus drive

The focus drive speaks an ASCII protocol, described by a dialect file (ADR-0030): its commands,
replies, units and errors. `config/dialects/example-focus.toml` is the emulator's.

1. Write `config/dialects/<device>.toml` from the vendor's protocol document. Only what the
   document says goes in it.
2. Check it: `cargo run -p siq-ascii --example focus_dialect_demo -- <dialect>` runs a session
   against an emulator made from the same file.
3. Set `[focus] kind = "ascii"`, its `port`, its `baud` and its `dialect`, and move it from the
   Live page: up, down, and stop.

**Waiting:** the owner hasn't provided the focus drive's protocol document yet. Until then it stays
simulated.

## 8. Troubleshooting

| Symptom | Cause | What to do |
|---|---|---|
| `the stage's controller didn't answer` | Wrong port or baud, or a board still restarting | `--list-ports`; GRBL's baud is 115 200; wait, and connect again |
| `the controller's settings aren't as expected` | `$$` changed, or `[stage.expect]` is wrong | Set the controller's, or the configuration's |
| `the controller reports in inches` | `$13=1` | `$13=0` from a terminal |
| A move fails with `Alarm` | A limit switch, or the soft limits | Home; then check the target is within the travel |
| Positions drift over a scan | Missed steps: too fast, or too much acceleration | Lower `$110`–`$112` and `$120`–`$122`, and measure again |
| A large repeatability | A loose coupling, or the stage settling | Raise `settle_ms` in the scan; check the mechanics |
