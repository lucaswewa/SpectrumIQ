# ADR-0052: The workflow engine

- **Status:** Accepted
- **Date:** 2026-10-26
- **Phase:** P6.1
- **Related:** ADR-0031, ADR-0035, ADR-0041, ADR-0049, REQ-WF-005, REQ-DATA-001; `microscope`'s
  ADR-0029 and ADR-0031

## Context

- **M6's acquisitions differ in shape:** tile scans, z-stacks, multi-position and time-lapse,
  and their compositions (a time-lapse of a multi-position of tile scans with z-stacks).
- **They share everything else:**
  - a dataset with its plan and what it was made with;
  - frames keyed by time point, channel, plane and position (ADR-0041);
  - progress, pausing and cancelling (REQ-WF-005);
  - what to do when a step fails;
  - the hardware left safe however a run ends.
- **`microscope`'s scan engine** (its ADR-0029) has a workflow Thing per kind of scan, each
  running its own loop at each site, and a sequence model (its ADR-0031) on top. A Thing per kind
  repeats the loop's bookkeeping in each.
- **P6.1's brief:** one executor for every plan, the plan a serde and JSON Schema enum of
  composable steps.

## Decision

- **A plan is data:** `Plan { name, steps, on_error }`, serde with its JSON Schema, which the
  dataset's manifest records whole. Its steps:
  - **device steps:** `move_to` (the stage, approached from the same side), `focus`, `channel`
    (put in place as ADR-0035 says), `snap`, `wait`;
  - **structure:** `repeat` (a count, and the index it numbers, `t`, `z` or `p`, within it),
    `index` (sets an index for the steps after it, among its siblings), and `policy` (steps with
    an error policy of their own).

  P6.2–P6.4 add their steps (a tile scan, autofocus, a z-stack) to the same enum, so that every
  plan composes with every other.
- **The frames' keys** come from the indices in force and the channel last applied. A key may be
  taken once in a run: a second snap at it fails, saying to number the snaps.
- **The executor, `run(devices, target, plan, control, observe)`:**
  - **before the run:** validates the plan (a name, a step, known channels, waits of a length),
    and refuses it before any dataset is made;
  - **the dataset:** made with the plan, the instrument's configuration and the stage–camera
    calibration in force (REQ-DATA-001). Its snaps are recorded there by the raw writer, each
    under its key, queued as it is snapped;
  - **the steps,** in order, each reported as `progress` (done, total, ETA at the pace so far, the
    step), with `frame` events for each snap;
  - **the report:** how it ended (`finished`, `cancelled`, `failed` with why), the steps done and
    in the plan, and the frames written. Once the dataset is made it is always reported, even when
    the run failed.
- **Control** (`RunControl`, shared by its clones): `pause`, `resume` and `cancel`.
  - **Pause** waits before the next device step, with `paused` and `resumed` events.
  - **Cancel** stops before the next step. A move under way is abandoned at once, since moves can
    take seconds. A channel change, a capture or a wait under way finishes first: they are short,
    and a capture cut off would leave the camera mid-frame.
  - **However a run stops early,** the stage and the focus are halted, so they are still when it
    returns.
- **Error policy** (`ErrorPolicy { retries, skip }`), the plan's or a `policy` group's: a failed
  step is tried again up to `retries` times, then skipped if `skip`, or else it ends the run.
  Each failure is a `problem` event, saying what comes next.
- **`DeviceSet`:** shared handles to every device, the acquisition engine, the channels and the
  calibrations, which the server makes from its instrument for each run. Putting a channel in
  place moves here from the server, so that the workflows and the `imaging` Thing share it.

## Alternatives considered

- **A Thing per kind of scan, as `microscope` has:** each would repeat the dataset, the keys, the
  progress, the control and the safe end. One executor does them once.
- **Flattening the plan into a list of device steps before running it:** simple to report on, but
  a time-lapse of a large tile scan has millions of steps. The executor walks the plan's tree,
  and counts its steps without listing them.
- **Keying frames by arrival order, as a recording does:** a skipped or retried step would shift
  every key after it. Keys from the plan's indices stay where the plan puts them.
- **Cancelling a capture mid-way:** the camera would be left armed for a frame that no one takes.
  Waiting for the exposure costs at most one exposure.
- **Returning the stage to where it started when a run ends,** as `microscope` does: a plan that
  wants it ends with a `move_to`. A cancelled run stops where it is, which is where the user can
  see what happened.
- **`tokio::time::pause` for the tests,** as the brief has it: the simulated camera renders on a
  thread of its own in real time, and paused time would run past its waits. The tests run in real
  time, on mock motion that arrives at once and the simulated camera at 128 × 96, and take about
  3 s.

## Consequences

- **Every M6 acquisition is a plan:** the `workflow` Thing (P6.5) validates one, runs it while it
  holds the global lock, maps `RunControl` to pause, resume and the invocation's cancellation, and
  its events to properties and Thing events.
- **Plans are portable:** the manifest's copy is the plan that ran, and runs again as it is.
- **A plan's keys must be unique;** a plan that snaps twice at one place without numbering the
  snaps stops at the second.
- **The run holds the hardware** for its length. A device's own failure is the step's, and its
  policy decides what follows.
- **The acquisition engine is `Clone`,** as a `DeviceSet` holds it.

## Verification

- `crates/siq-workflow/tests/engine.rs`:
  - steps in order, and frames keyed by their indices;
  - pause and resume;
  - a cancelled run abandoning its move, the stage halted, and its frames kept;
  - a capture under way finishing before a cancelled run stops, the camera ready after;
  - the error policies, the plan's and a group's;
  - the manifest's plan, instrument and calibration, and plans refused before any dataset.
- **`run_plan_sim`:** a small time-lapse of a multi-position acquisition in two channels on the
  simulated instrument.
