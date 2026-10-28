# ADR-0051: The calibration UX, and the gating rules

- **Status:** Accepted
- **Date:** 2026-10-25
- **Phase:** P5.4
- **Related:** ADR-0008, ADR-0037, ADR-0039, ADR-0049, ADR-0050, REQ-CAL-001, REQ-CAL-002;
  `microscope`'s calibration wizard (its P29)

## Context

- **The calibrations exist on the server** (ADR-0049, ADR-0050): the stage–camera calibration of
  each objective, and the dark and flat of each objective and channel, run by the `calibration`
  Thing, each of the objective and channel in use.
- **A new user doesn't know they are needed,** nor in which order, nor what each needs in the
  field: a sample with detail for the stage–camera calibration, a blank slide for the flat, and
  the right objective and channel in use for both.
- **What must hold:**
  - M5's exit criterion: the first-run wizard guides a new user to a calibrated state;
  - P5.4's brief: a stage–camera wizard (choose the objective, run, see the residuals, accept or
    redo), a dark and flat wizard, the objective setup, a status page per objective × channel, a
    first-run prompt, and gating: scans need a valid calibration.
- **`microscope` has a wizard:** a dialog of tasks, each a series of steps, that opens once per
  connection when a Thing says it needs calibrating. Its steps either guide (text, the live image,
  the d-pad) or calibrate (a Thing's panel, or a button). Next says Skip while the step's Thing
  still needs calibrating.

## Decision

- **The wizard is `microscope`'s, adapted** (`web/src/calibration/`): the same dialog, Back and
  Next, Skip until a step has done its part, and closing early asked first.
- **Its tasks are per objective, or per objective and channel**, built from the `calibration`
  Thing's `status`, not from Things' `calibration_required`:
  - **stage–camera:** the objective put in place, a guide to a sample in focus with the live image
    and the d-pad, then the run;
  - **dark and flat:** the objective and channel put in place, the dark, a guide to a blank slide,
    then the flat.
- **Three kinds of step:**
  - **guide:** text, with the live image and the d-pad when they help. The d-pad leaves the arrow
    keys to the page beneath;
  - **place:** turns the turret to the task's objective if a turret is fitted, and applies its
    channel; else asks for it by hand;
  - **calibrate:** runs the calibration, shows what it found, and offers Again. *Accept* is going
    on: each run that passes the server's checks is kept, and the newest is in force (ADR-0049).
    A refusal shows as a toast, with the server's reason.
- **The first-run prompt:** the wizard opens by itself, after a welcome, once in each page's life
  for each objective that comes into use lacking a calibration. That is the objective at the
  turret, or the only one configured. It offers that objective's stage–camera calibration, then
  the flat of each of its channels.
- **The Calibration page** (`/#/calibration`):
  - **Objectives:** each with its turret position and parfocal offset, its stage–camera
    calibration (pixel size, rotation, date), and whether scans may run with it;
  - **Dark and flat:** each objective and channel's, with its level and the flat's range;
  - **actions:** a task for any row, and *Calibrate what's missing*, every objective's.
- **The gating rules** (`calibration/gating.ts`), for a scan with an objective in some channels:
  - **blocked** without the objective's stage–camera calibration: tiles can't be placed. Also
    while the calibrations aren't known, or for an objective not configured;
  - **warned** for each channel without a flat (the mosaic may show its seams), and for a
    stage–camera calibration over 90 days old;
  - the Calibration page shows each objective's gate now, and the Scan page (M6) will hold its
    Start to it. The workflow engine (M6) applies the same blocking rule on the server, so that a
    plan sent over the API can't skip it.

## Alternatives considered

- **`calibration_required` on each Thing, as `microscope` has:** SpectrumIQ calibrates per
  objective and channel, not per Thing. `status` already says what is in force for each.
- **A calibration kept only once accepted:** the server would hold an unkept record between two
  actions, or take one back from the client. Keeping every run, the newest in force, is simpler
  and loses nothing: a bad run is followed by a better one.
- **A first-run wizard for every objective and channel at once:** 3 objectives and 4 channels
  are 15 tasks, most for objectives not in place. Offering each objective's tasks when it comes
  into use keeps each wizard short, and still leads to a calibrated state.
- **A flat required for scans:** a scan without one is still placed right. Its seams are a
  matter of looks that the retrospective estimate (ADR-0050) can fix after the fact.
- **A Settings page entry, as `microscope` has:** the rail already has a Calibration
  destination (architecture §11), and the status belongs beside its actions.

## Consequences

- **A new user meets the wizard** on first connecting, for the objective in place, and again for
  each other objective as it comes into use. They can skip any step, and finish from the
  Calibration page.
- **Gating is in the UI from now,** and enforced on the server from M6.
- **The arrow keys stay the page's** while the wizard shows the d-pad: `useJog({ keys: false })`.
  Esc is still STOP, wherever the keyboard is (ADR-0039).
- **Status is polled every 10 s** and read after each calibration, since it is computed on the
  server, not observed: a flat a job estimates appears within 10 s.

## Verification

- `web/tests/unit/calibration/wizard.spec.ts`:
  - the tasks each objective lacks, and the summaries of what a calibration found;
  - the first-run prompt for the objective in use, through each step, run, run again, skipped and
    finished;
  - no second prompt for the same objective; closing early asked first;
  - the Calibration page's rows and gate, and a task from it, turning the turret.
- `web/tests/unit/calibration/gating.spec.ts`: blocked, warned, and stale.
- **`/#/dev/calibration`:** the page and the wizard against a fake instrument.

## Amendment (P9.4, 2026-10-28): the first-run prompt, for a session that may calibrate

- **The wizard opens by itself only for a session that may change the instrument** (ADR-0068): an
  operator or an admin holding the control token, or anyone where the service doesn't enforce.
  An objective lacking a calibration is offered when that becomes true, such as when an operator
  takes control. A viewer is never asked.
- **The Calibration page's buttons** need control of the instrument, as the calibrations do.
- `web/tests/unit/calibration/wizard.spec.ts`: no prompt for an operator without the token, then
  the welcome once it holds it.
