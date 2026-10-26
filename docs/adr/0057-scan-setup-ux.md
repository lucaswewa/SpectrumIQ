# ADR-0057: Scan setup UX

- **Status:** Accepted
- **Date:** 2026-10-26
- **Phase:** P6.6
- **Related:** ADR-0039, ADR-0051, ADR-0053, ADR-0054, ADR-0055, ADR-0056, REQ-WF-001, REQ-WF-002,
  REQ-WF-003

## Context

- **A tile scan is planned in stage space:** a region over the slide, tiles along the camera's
  axes (ADR-0053), focus found on a map or at each tile (ADR-0054), and maybe a z-stack at each
  (ADR-0055). The user plans it by eye, against where the stage is and the places saved.
- **The planner and the estimate are the server's** (`scan::plan`, `scan::estimate`). A second
  copy in the browser would drift from them, and the estimate needs the instrument: the
  calibration in force, the camera's region, the stage's speed, the disk.
- **The calibrations gate a scan** (ADR-0051): without the objective's stage–camera calibration,
  tiles can't be placed.
- **The estimate's timings were the simulator's** (ADR-0053). P6.2 left the configured stage's
  for later, and z-stacks and autofocus weren't in it.
- **Saved positions were kept in the browser** (ADR-0039) until the server kept them (P6.5).
- **The live mosaic** (M7) will be the map's background.

## Decision

- **The Scan page** (`/#/scan`, and `/#/dev/scan` against a fake instrument): a stage map beside
  a pane with the tools, the plan's form, its estimate and Start.
- **The stage map** is an SVG in stage µm, x right and y down, as the camera's image is:
  - **drawn:** the stage's travel; the frame where the stage is, by the calibration in force
    (`workflow.field`); the saved positions; the region; the focus map's places; and the
    estimate's tiles, each its frame, joined in their order;
  - **tools:** Pan; Rectangle, dragged corner to corner; Polygon, clicked vertex by vertex and
    closed at its first; Places, clicked to add a focus map's place, or on one to remove it. The
    wheel zooms about the pointer. *Whole travel* and *The region* fit the view;
  - **its geometry** is pure (`scan/geometry.ts`): stage to map and back, fitting, zooming about a
    point, panning.
- **The form:** the objective in use (chosen on the Live page), the name, the channels (the one
  in use if none), the overlap, the order; the focus as it is, at one focus, on a focus map
  (thin-plate, its places picked on the map), or autofocused at each tile; and a z-stack at each
  tile, its range, step and direction.
- **Checks before the server is asked** (`problems`): what the engine's `Plan::check` would say
  of the form, worded for it. Then the server answers for the rest: its 422s and the plan's
  refusals are shown as the estimate's reason.
- **The estimate is the server's,** `workflow.estimate_tile_scan`, asked 300 ms after the plan
  settles, the answer to an older plan dropped. It gives the tiles and a frame's corners, so the
  map draws exactly what will run, then the frames, the time, the bytes, the disk's room, and
  warnings: the disk too small, a frame larger than the pool's slots, tiles outside the travel, a
  z-stack leaving the focus drive's travel, an overlap under 5 %. Memory is the frame pool's,
  which a scan doesn't grow (ADR-0053).
- **The estimate counts focusing:** each plane's frames and focus moves, an autofocus's 24
  frames at each tile, and a focus map's places. **The stage's speed and acceleration** are the
  stage's own when its driver says (`Stage::kinematics`, the GRBL driver's `$11x` and `$12x`), the
  simulator's otherwise.
- **Start** needs the gate open, no problems, an estimate that fits, and no run under way. It
  starts `workflow.tile_scan`, and the pane shows the run's progress; the run monitor is P6.7's.
- **Saved positions are the server's,** the `workflow` Thing's: the Live page saves, renames
  (Enter, or leaving the name), goes to and forgets them there, and the map shows them.

## Alternatives considered

- **A canvas, not SVG:** faster for thousands of tiles, but SVG is a DOM the tests can read, and
  the scans planned here are hundreds of tiles. The live mosaic (M7) may draw beneath it on a
  canvas.
- **The planner in the browser:** an instant preview, but a second implementation to keep equal
  to the server's, without the instrument's facts.
- **Validating the form with the plan's JSON Schema at run time:** the TD's schema holds no
  steps (ADR-0056), and a validator would be a new dependency. The form's checks mirror the
  engine's, the tests check its requests against the OpenAPI schema, and the server's 422 is the
  last word.
- **The map with y up, as stage drawings often are:** the live image has y down, and the map should
  look as the camera sees.
- **Start disabled while a warning stands:** warnings are judgements, such as a small overlap or a
  stale calibration. Only what would fail blocks.

## Consequences

- **What the map shows is what runs:** the tiles are the server's, from its calibration.
- **Every change asks the server again**, once the plan settles: an action invocation each,
  without the global lock, so it works during a run.
- **The scan's time on the simulator** was within 9 % of its estimate, with stacks, after the focus
  move's time was measured: 0.15 s, the drive seen to stop and a frame waiting for the new
  position.
- **The browser's old saved positions** are left in its storage, unused.
- **Background:** until M7, the map is plain.

## Verification

- `web/tests/unit/scan/geometry.spec.ts`: the map's geometry.
- `web/tests/unit/scan/plan.spec.ts`: the form's requests valid against the OpenAPI schema for
  every focusing, its checks, and the estimate's wording.
- `web/tests/unit/scan/ScanPage.spec.ts`: a rectangle drawn, its estimate asked and shown, its
  tiles drawn, and Start posting the same request; Start blocked without a calibration.
- `web/tests/unit/control/control.spec.ts`: a position saved on the server, then gone back to.
- `crates/siq-server/tests/workflow.rs`: a scan estimated, its frame the field of view's, then run
  within 20 % of its estimate.
- `crates/siq-workflow/tests/scan.rs`: the stack's and autofocus's terms.

## Amendment (P6.7, 2026-10-26): run control, recovery, and sequences

- **The run panel,** below the Scan page's pane, follows the run under way wherever it was started:
  its steps done of the plan's, the time left at the pace so far, the step just done; thumbnails of
  its latest eight frames; Pause or Resume; and Cancel, once a dialog confirms it ("It stops after
  the frame under way; the frames taken so far are kept"). Once the run ends, how it ended, until
  closed.
- **Thumbnails come from the overview stream,** which shows every frame the acquisition publishes,
  a run's snaps included: each frame taken (`tile_acquired`) is paired with the stream's next
  image, shrunk to 160 pixels at once, as the stream closes its images. No server code; a frame
  taken before the stream showed the last is paired with the next image, the latest waiting.
- **Recovery is a policy's choice** (ADR-0052, amended): a plan's `on_error` may `ask`. A failed
  step, once tried again as often as it says, leaves the run `asking`, the step and its error as
  `workflow.problem`; the panel offers Retry and Skip, and Cancel still works. The form's "On a
  failure" is "End the run", "Skip the step" or "Wait, and ask".
- **Notifications:** each run's `finished` event is a toast, wherever the run was started:
  success, cancelled with the frames kept, or failed with the reason.
- **Sequences,** after `microscope`'s Sequence tab: the Scan page's second mode. Saved positions,
  or where the stage is; channels; an autofocus and a z-stack at each position; and a time-lapse,
  every so many seconds, so many times; started through `workflow.run` as a `time_lapse` of a
  `multi_position` (ADR-0055). Its frames are counted; the server checks the rest.
- **Alternatives considered:**
  - **Thumbnails served by the workflow Thing:** a property of the last frame, made small on the
    server, would be every client's at once, but is server code for what the stream already has.
  - **Retry and Skip after the run has failed:** the run would have to start again and find its
    place. Waiting keeps the run, its dataset and the global lock.
  - **A run monitor page of its own:** the Scan page is where runs start, and the panel follows
    runs from scripts too. A rail destination can show it later.
- **Verification:** `web/tests/unit/scan/RunPanel.spec.ts` (the states, Pause, Resume, Retry,
  Skip, the end; Cancel only once confirmed), `thumbnails.spec.ts`, `sequence.spec.ts`;
  `web/tests/unit/stores/stores.spec.ts` (a toast for each end); `crates/siq-workflow/tests/engine.rs`
  and `crates/siq-server/tests/workflow.rs` (a failed step asked about, retried, skipped);
  `/#/dev/run`.
