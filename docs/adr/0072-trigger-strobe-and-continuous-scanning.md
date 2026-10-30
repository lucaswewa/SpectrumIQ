# ADR-0072: Trigger and strobe, and a design for continuous scanning

- **Status:** Accepted. The continuous scanning is a design only.
- **Date:** 2026-10-29
- **Phase:** P10.4
- **Related:** ADR-0029, ADR-0049, ADR-0053, ADR-0070, REQ-ACQ-006, REQ-STG-005

## Context

A snap and a tile scan's frames are software-triggered (ADR-0070). A trigger goes through the
Coaxlink's camera and illumination controller (CIC), in `CameraControlMethod` `RC`: the
`StartCycle` command starts a cycle, which sends the camera a CoaXPress trigger, and the camera
exposes for its own `ExposureTime` (`TimedTriggerControl`).

P10.4 asks for two things:

- a strobe to the illumination during each exposure, and the trigger's latency measured;
- a design for scanning continuously, the camera triggered at positions while the stage moves.

The stage is a GRBL controller, still simulated (ADR-0029, amended).

Reading the Coaxlink Quad CXP-12 found:

- **The CIC, in `RC`,** has its own `ExposureTime`, `StrobeDelay` and `StrobeDuration`, in µs. The
  strobe is timed from the exposure's start.
- **The I/O toolbox** puts the CIC's strobe, `Device0Strobe`, on an output line:
  - `IOUT11`–`IOUT22`, opto-isolated outputs;
  - `TTLIO11`–`TTLIO22`, TTL lines set as outputs.
- **A line input tool** (`LIN1`–`LIN8`) can take an output line as its source, so an output's
  edges are counted with no wiring.
- **The device counts its events:** `CameraTriggerRisingEdge`, `StrobeRisingEdge`, and so on.
- **A quadrature decoder** (`QDC1`), a divider, a multiplier-divider and delay tools can each be a
  cycle's trigger source (`CycleTriggerSource`). Each line input tool can be one too.

Measured on the camera with `strobe_timing`, the acquisition thread's 20 ms `pop` tick set the
software trigger's latency:

- the trigger call waited for the grabber, which the camera's thread held, up to 20.7 ms;
- the exposure started 0.40–20.9 ms after the call, 6.6 ms on average.

## Decision

- **The strobe** (`siq_euresys::Strobe`, configured as `[camera.strobe]`) has three settings:
  - the output line, such as `IOUT11`;
  - a delay from the exposure's start;
  - a duration: the exposure's if not given.

  When triggered, the camera sets the CIC's `ExposureTime` to its own, its `StrobeDelay` and
  `StrobeDuration`, and the line's `LineSource` to `Device0Strobe`. Each change of the exposure
  follows. Free-running, there is no CIC cycle, and the line's source is `Low`.
- **The camera's thread's tick is 2 ms,** not 20. A software trigger then waits at most 2 ms for
  the grabber. Measured over 40 triggers:
  - the call takes 0.06–0.93 ms;
  - the exposure starts 0.45–1.05 ms after it, 0.60 on average, with 0.13 ms rms.

  The thread wakes 500 times a second when no frame comes, each time in a blocking wait. Every
  hardware test still passes: the modes' rates, and the stamps.
- **The check needs no wiring:**
  - the line input tool `LIN1` reads the strobe's line back, and the interface counts its rising
    edges;
  - the device counts its triggers and strobes;
  - each trigger must make one of each.
- **`EuresysCamera::with_grabber`** runs a closure on the grabber beside the camera's thread, for
  such counts in a test or a bring-up.

### Continuous scanning: the design

Stop and go, a tile costs a move, a settle, an exposure and a readout. With the GRBL emulator's
50 mm/s and 200 mm/s², a 3 mm step takes about 0.25 s; with a 0.1 s settle that makes about 0.36 s
a tile, 2.8 tiles a second. The camera's full frame comes 12.28 times a second, so a stage that
never stops could go about four times as fast.

The scan would move at a constant speed along each row, and trigger a frame at each tile's
position:

- **The trigger:** a position, not a time. The camera's CIC takes its cycle from a line, or from
  the quadrature decoder, so the frame's place is the stage's own, whatever the host's latency.
  Two sources would do:
  1. **Encoders on the stage** (A/B quadrature) into the Coaxlink's differential inputs, decoded
     by `QDC1`. A divider sets the pitch in counts. This is the most accurate: the position is
     measured, not commanded. It needs encoders the stage may not have.
  2. **The controller's synchronised output.** grblHAL sets an output in step with the planner,
     at the start of the next move (`M62`/`M63`). A row becomes short moves with an output pulse
     between them, wired to a Coaxlink input. GRBL 1.1 has no such output: its spindle and coolant
     commands wait for the motion to finish. This needs grblHAL, and the position is the commanded
     one, as stop and go's is.
- **The blur:** the stage moves during the exposure. With a strobe of length `t` at speed `v`,
  the blur is `v·t`. At 0.32 µm a pixel, half a pixel is 0.16 µm:
  - at 10 mm/s, `t` must stay under 16 µs;
  - at the camera's full rate, a 3 mm step at 12.28 frames a second, `v` is 37 mm/s, and `t`
    must stay under 4.3 µs.

  The CIC's `StrobeDuration` goes down to microseconds. The illumination must then give a frame's
  light in that pulse.
- **The pose:** each frame's pose is its trigger's position: the encoders', or the commanded one
  at the output. The registration and the solve (ADR-0061, ADR-0062) refine it, as they refine
  stop and go's.
- **What isn't decided:** the encoders, the illumination's pulse, and the controller's firmware
  are the owner's. The phase that builds it waits for them.

## Alternatives considered

- **A shorter tick only while software-triggered.** Free-running, a caller rarely waits. One tick
  for both is simpler, and costs only wakeups.
- **Cancelling the pending `pop` for a caller.** The shim has no way to interrupt a `pop`, and
  eGrabber is one thread's at a time.
- **The strobe from the camera** (its own `LineSource` outputs). The CIC's strobe is timed with
  the trigger that the CIC sends, on the grabber's lines, beside the other I/O tools.
- **Continuous scanning by time:** the camera free-running, and each frame's position computed
  from the stage's speed and the status reports. GRBL reports at 5 Hz, stamped within half a
  10 ms round trip. At 10 mm/s that is about 50 µm, far beyond the registration's search.

## Consequences

- With a strobe configured, a snap's light can be pulsed. The illumination's strobe input must be
  wired to the line, which the owner does.
- A software trigger's exposure starts within about 1 ms of the call. The frames' times are
  still the grabber's stamps (P10.2).
- The camera's thread wakes every 2 ms when idle.

## Verification

- `siq-euresys` `tests/camera.rs`:
  `a_strobe_follows_the_triggered_exposure_and_stays_low_free_running`. Triggered, the CIC's delay
  and duration are set and the line carries the strobe. The duration follows a new exposure.
  Free-running, the line goes low.
- `siq-euresys` `tests/hardware.rs`: `each_trigger_strobes_its_line_and_exposes_soon_after`, on the
  camera. 15 triggers make 15 camera triggers, 15 strobes and 15 edges on `IOUT11`, and each
  exposure starts within 2 ms of its call. The five tests of P10.1 and P10.2 pass with the 2 ms
  tick. Every setting of the camera and the grabber is put back.
- `strobe_timing`, with 40 triggers, prints the latencies above, and 40 of each count.
