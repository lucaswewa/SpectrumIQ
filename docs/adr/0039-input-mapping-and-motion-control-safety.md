# ADR-0039: Input mapping and safety rules for motion control

- **Status:** Accepted
- **Date:** 2026-10-24
- **Phase:** P3.4
- **Related:** ADR-0008, ADR-0029, ADR-0033, ADR-0038, REQ-STG-001, REQ-STG-002, REQ-SEC-004,
  REQ-DEV-001

## Context

- **The stage** (GRBL: x, y and the coarse z, in µm) has these actions:
  - `move_to` and `move_by` hold the global lock and run to their end;
  - `jog(vx, vy, vz)` starts a move at a velocity. The move runs until it is stopped, jogged
    again, or reaches the edge of travel (ADR-0029);
  - `home`;
  - `halt`, which passes the lock (ADR-0033).
- **The focus drive** has `move_to`, `move_by` and `halt`.
- **The lock allows one hardware action at a time:** a second `move_by` while one runs is
  refused.
- **The server has no jog watchdog:** a jog runs until something stops it.
- **`microscope`'s controls,** which ADR-0008 allows importing:
  - they renew a relative jog every 300 ms, since OpenFlexure has no velocity jog;
  - their click-to-move uses a calibrated camera–stage mapping. SpectrumIQ calibrates in M5.
- **The simulator's geometry:** a sample point P shows at pixel = centre + (P − stage) ×
  magnification / pixel size. There is no flip, so the image's x and y are the stage's.

## Decision

### The input mapping

| Input | Motion |
|---|---|
| ← → ↑ ↓, or the d-pad, tapped | `stage.move_by` one XY step. The default is 100 µm, with presets of 10, 100 and 1000 µm |
| The same, held for 300 ms | Once the step has ended, `stage.jog` at 4 steps a second towards everything held (↑ and → go diagonally) until all is released, then `stage.halt` |
| PgUp / PgDn, or the coarse-Z buttons | The same, on the stage's coarse z (50 µm a step) |
| The wheel over the images, or Focus − / + | `focus.move_by` a focus step (1 µm) a notch: turned away from you is up |
| A double-click on the overview | `stage.move_by` share × region × pixel size / magnification on each axis, which brings the point to the centre |
| Esc, or STOP | `stage.halt` and `focus.halt` |

- **Directions are as the image shows them:**
  - ↑ shows what is above, which is −y;
  - → is +x.
- **Inverting x or y** is kept in this browser, for set-ups whose camera sees the stage turned or
  mirrored. It applies to click-to-move as well.
- **Click-to-move is nominal until calibration in M5.** It uses the objective in place's
  magnification, or 10× if there is none.
- **The steps are kept in this browser.**

### One move at a time

- **Taps and wheel notches made while a move runs are added up,** and sent as one move when it
  ends (`Stepper`), so the lock never refuses them.
- **A hold becomes a jog only once its step has ended.** Each change of what is held sends a new
  jog, which replaces the last.
- **Moves are cut to the travel,** from the last position known, and never turned the other way:
  - a step at a limit goes nowhere, rather than being refused;
  - click-to-move goes as far towards the point as the travel allows;
  - with no travel known, the server decides.
- **Click-to-move ignores double-clicks while its move runs.**

### Safety

1. **STOP is always visible and never disabled.**
   - It sits in the Live page's toolbar, outside the pane, so collapsing a section doesn't hide it.
   - It stays enabled in every state: still, moving, jogging, homing, holding, in alarm,
     disconnected, and with no server.
   - It forgets the steps still to come, then halts the stage and the focus. Both halts pass the
     global lock, and halting nothing is harmless.
2. **Esc is STOP from anywhere on the page, fields and dialogs included.**
   - It is a capture-phase listener on the window, so it acts before any component sees the key.
   - It doesn't go through the shortcut registry, which leaves dialogs and fields alone.
   - It closes a dialog too.
3. **A jog ends** when every press is released, when the window loses focus, when the page is
   hidden or left, or when the page unmounts. The key-ups would be missed otherwise.
4. **Some moves ask first.**
   - **Home:** the stage travels to its reference switches.
   - **Changing the objective:** the turret turns under the sample. The confirmation gives:
     - the difference between the two objectives' parfocal offsets;
     - the new objective's immersion.

     The client doesn't move the focus.
5. **The stage's state is shown:** still, moving, jogging, homing or holding; an alarm in red,
   with its code; "not homed". The server's refusals are shown as toasts.

### Saved positions

- **Saved positions are kept in this browser** (`spectrumiq.positions`) until P6.5 moves them to
  the server.
- **Each has a name,** renamed in place, and the stage's x, y and z, with the focus if it was known.
- **Going to one** moves the stage, then the focus.

## Alternatives considered

- **`microscope`'s renewed relative jogs:**
  - each `move_by` holds the lock until it ends, so a renewal during it is refused;
  - each renewal would also stop and restart the stage.

  A velocity jog is smooth and needs one request per change.
- **A client heartbeat that renews the jog:** without a watchdog on the server, it doesn't make a
  client that dies any safer.
- **Esc in the shortcut registry:** the registry ignores keys in dialogs and fields, and STOP must
  not.
- **Coarse z on the wheel:** the wheel is for fine focus, and coarse z moves far.
- **Moving the focus by the parfocal offset when the objective changes:** that's an automatic move
  near the sample, and possibly in oil. The confirmation states the offset instead.

## Consequences

- **A client that dies mid-jog leaves the stage jogging.** This covers a crash, a lost network or
  a machine that sleeps before `pagehide`. The stage then runs until the edge of travel, or
  until another client's STOP. A watchdog on the server is a follow-up: a jog would last a few
  hundred ms unless renewed.
- **Click-to-move is off by any error in the nominal magnification,** and by any rotation, until
  M5.
- **The arrow keys and PgUp/PgDn belong to the stage while the Live page is shown.** Fields,
  sliders and lists keep their own arrow keys.
- **`RoiSelector` now captures the pointer only once a press has become a drag.** This is ADR-0038's
  amendment: a captured pointer's double-click went to the selector, not the image.

## Verification

- `web/tests/unit/control/jog.spec.ts`:
  - a tap, a hold and its jog, a hold released before its step ended;
  - taps added up, presses combined and cancelling out, repeats, release on blur;
  - cancel;
  - the stepper;
  - cutting to the travel.
- `web/tests/unit/control/navigation.spec.ts`:
  - the steps and their inversion;
  - the wheel's notches and focus;
  - click-to-move's maths;
  - the saved positions.
- `web/tests/unit/control/control.spec.ts`, on the Live page:
  - STOP enabled, halting both, in each of the 7 stage states and with no server;
  - Esc with Home's confirmation open, and Home only once confirmed;
  - the keys, the coarse-Z button and the wheel;
  - a step at the edge of travel not sent;
  - going to a saved position;
  - the objective's confirmation, with its parfocal offset and immersion.
- `web/tests/unit/live/LivePage.spec.ts`: a double-click on the overview moves the stage.
- **By hand, in Edge,** against the simulator, then on `/#/dev/control`:
  - a tap moved 100 µm;
  - a hold jogged, and stopped on release;
  - the wheel focused;
  - a double-click moved as far as the travel allowed;
  - STOP halted a running jog;
  - no console errors.
