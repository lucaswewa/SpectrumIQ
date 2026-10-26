# ADR-0056: The workflow API: invocation semantics, locking and cancellation

- **Status:** Accepted
- **Date:** 2026-10-26
- **Phase:** P6.5
- **Related:** ADR-0003, ADR-0033, ADR-0052, ADR-0055, REQ-WF-005, REQ-WF-006, REQ-NET-002;
  `microscope`'s ADR-0029

## Context

- **The engine runs plans** (ADR-0052, ADR-0055), but only in process. The UI (P6.6, P6.7) and
  scripts need them over HTTP: started, followed, paused and cancelled.
- **teta-wot's actions** answer a POST with an invocation: 201, then `pending`, `running`,
  `completed`, `error` or `cancelled`. Cancelling is a `DELETE` of the invocation, which sets
  its cancel token. An input that doesn't fit the action's schema, or doesn't deserialize, is
  refused with 422 before any invocation.
- **The global lock** (ADR-0003) is taken by every hardware action for its whole run, and waits
  50 ms for it; a second action finds it busy as `GlobalLockBusy`.
- **A Thing Description can't hold a recursive schema:** teta-wot inlines every reference, up to
  a limit. A plan's steps nest (`repeat`, `z_stack`, …), so a `Plan` can't be an action's
  input as its type stands.
- **`microscope`'s scan Things** (its ADR-0029) run one scan at a time under the global lock,
  with the scan's settings fixed at its start, a record of how it ended, and its progress as a
  property.
- **M8 stitches a scan's dataset** once it is complete, so something must tell it.

## Decision

- **A `workflow` Thing:**
  - **actions that run:** `run(name, steps, on_error)` for a plan of any shape, and `tile_scan`,
    `z_stack`, `multi_position` and `time_lapse`, each with its step's fields and a `name`, making
    a plan of its shape. `tile_scan` takes a `focus_map` made first. `multi_position` given no
    positions visits the saved ones. Each answers with how its run ended: `Finished { name,
    dataset, outcome, error, steps, frames }`;
  - **actions that don't run:** `pause` and `resume`, and the positions' `save_position`,
    `rename_position` and `remove_position`;
  - **properties:** `state` (`idle`, `running`, `paused`) and `progress`, the engine's, both
    observable; `positions`; and `plan_schema`, a plan's whole JSON Schema;
  - **events:** `tile_acquired`, each frame's `t`, `c`, `z` and `p`, and `finished`.
- **422 for a plan that couldn't run:** each action's input is a request whose deserializing
  makes its plan and checks it with `Plan::check`, the engine's checks that need no instrument.
  Its refusal is the 422's message. A plan that can't run on this instrument, for want of a
  channel or a calibration, fails its invocation at its start, before any dataset or motion.
- **The steps' schema is opaque in the Thing Description:** `steps` are objects there, and
  `plan_schema` has them all.
- **One run at a time, under the global lock:** a run's action holds it from start to end, so a
  manual move during a run, or another run, is `GlobalLockBusy`. `pause`, `resume` and the
  positions' actions don't take it: a run can be paused, and a position saved, while it runs.
- **Cancelling:** a `DELETE` of the invocation sets its token, which cancels the run's control.
  The engine abandons a move under way, lets a capture finish, then halts the stage and focus,
  and writes what was taken (ADR-0052). The invocation ends as `cancelled`; a failed run ends as
  `error` with the engine's reason.
- **Saved positions** are a setting of the server: `positions.json` in the settings folder,
  written whole and renamed into place. A position is saved by a unique name, where the stage and
  focus are unless given.
- **`DatasetHook::on_dataset_complete(instrument, report)`:** the instrument's hooks are told of
  each run's dataset, however it ended, before the action answers. They queue work, as M8's
  stitching will; they don't do it.
- **The simulator's sample sits within the focus drive's travel:** `[sim] focus_at` (2000 µm in
  `config/sim.toml`), where the simulated drive starts. Before, it was in focus at the bottom of
  the travel, so an autofocus or a z-stack there would have left it.

## Alternatives considered

- **A plan's schema expanded to a fixed depth:** the TD would describe nested steps, but only so
  deep, and teta-wot's validation would refuse a plan nested deeper. The engine checks them all.
- **Every check at the invocation's start:** simpler, but a script would learn of a typo only
  from the invocation's state. 422 is what the API answers for any other bad input.
- **A run as a background job, its action answering at once:** the invocation is already the
  run's handle, with its state, its cancel and its lock. A job would need its own of each.
- **A lock of the workflow's own:** a manual move would then still be allowed during a scan. The
  global lock refuses it, as it refuses a move during a calibration.
- **Saved positions as a teta-wot `#[setting]`:** that would need its settings folder, which
  would also start persisting every other Thing's settings. A file of its own, beside the
  calibrations, changes nothing else.
- **The simulator in focus at the drive's 0, its travel's bottom:** what a real drive is not, and
  an autofocus sweep below it would be clamped.

## Consequences

- **The UI and scripts have one API for every shape:** the named actions for the usual ones,
  `run` for the rest.
- **A long run holds the lock for hours,** so only `pause`, `resume`, the halts and the read-only
  affordances work meanwhile. The UI must show why the rest are refused.
- **The TD doesn't validate steps:** a client learns of a bad step from the 422.
- **The run's dataset isn't the `datasets` Thing's:** it is listed in the catalog, but no
  `datasets.created` event says so; `workflow.finished` does. The gallery can listen to both
  (P6.7).
- **On the simulator, the focus drive reads 2000 µm in focus**, not 0.

## Verification

- `crates/siq-server/tests/workflow.rs`, over HTTP on the simulated instrument:
  - `a_tile_scan_holds_the_global_lock_and_reports_its_progress`: its progress and state while it
    runs, a manual move refused as `GlobalLockBusy`, 12 frames in 33 steps, listed, and the hook
    told;
  - `a_time_lapse_pauses_resumes_and_is_cancelled_by_deleting_its_invocation`;
  - `plans_that_could_not_run_are_refused_with_422`, and one this instrument can't run failing at
    once;
  - `saved_positions_are_kept_and_a_multi_position_visits_them`, across a restart.
- **The scripts** `examples/python/tile_scan.py`, `z_stack.py` and `timelapse.py`, against the
  simulated server.
