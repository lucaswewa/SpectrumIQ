# ADR-0055: The sequence model

- **Status:** Accepted
- **Date:** 2026-10-26
- **Phase:** P6.4
- **Related:** ADR-0052, ADR-0053, ADR-0054, REQ-WF-003, REQ-WF-004; `microscope`'s ADR-0031

## Context

- **Multi-dimensional acquisition** takes frames along time, position, focus and channel: a
  time-lapse of several positions, each a tile scan, each tile a z-stack in two channels. The
  shapes nest, and any of them may be left out.
- **The engine (ADR-0052)** runs plans as steps, and numbers each frame by `t`, `c`, `z` and `p`,
  which name its raw file (`t0000_c00_z000_p00012`). A key must be unique in a dataset.
- **A tile scan (ADR-0053)** numbers its tiles `p` from 0, so two scans in one plan collided.
- **The focus drive arrives from one side** when asked to (P1.9): the ASCII driver goes up to its
  target, overshooting below it first on the way down, so its backlash is the same at every
  arrival.
- **Time-lapses run for hours.** A slow step, or a pause, mustn't shift every point after it, nor
  bunch late points together. `microscope`'s sequences (its ADR-0031) run a late point within its
  own interval, skip one whose interval has passed, never run points back to back to catch up,
  and record what they skipped.

## Decision

- **Three steps that hold steps,** composing in any order:
  - **`z_stack { planes, steps }`:** `steps` at each plane about where the focus is, a frame if
    none, then the focus back to the middle. `planes` is a range, a step and a direction: a whole
    number of steps across, at most the range, centred on the focus, at most 1000 planes.
    `z` numbers the planes from the bottom, whichever way they are taken. A tile scan's `z_stack`
    takes one at each tile, in place of its frame, its channels at each plane.
  - **`multi_position { positions, autofocus, steps }`:** at each position, the stage moved there,
    the focus to the position's own if it has one, an autofocus if asked, then `steps`, a frame
    if none. **The places of its steps are about each position:** a move, a tile scan's region, a
    focus map's places. So "a 2 × 2 mm scan about each saved position" is one plan.
  - **`time_lapse { interval_s, count | duration_s, overrun, steps }`:** `steps` at points
    `interval_s` apart from the first, numbered `t`: `count` of them, or as many as begin within
    `duration_s`, whichever is fewer.
- **The direction of a stack:** up, by default, takes each plane from the one below, the way the
  focus drive arrives, with no overshoot between planes. Down takes them from the top, each
  overshot and approached from below. Every focus move is unidirectional.
- **`p` numbers the plan's places in order:** each position and each tile takes the next, when
  the run starts. A position whose steps tile it takes none of its own; its tiles do. A
  time-lapse repeats the same places, so a place keeps its `p` at every point.
- **The schedule:** point `k` is due `k · interval_s` after the time-lapse began, on tokio's clock.
  It waits, while a cancel stops it at once, and a pause holds it past its time.
  - **`skip`,** the default: a late point runs within its own interval, and is skipped once that
    has passed. The schedule never moves.
  - **`shift`:** a late point runs at once, and the points after it keep the interval from it:
    the schedule slides by the lateness.
  - **A skipped point's frames** leave a gap in `t`, and its steps leave the progress's total.
- **What a run records,** in `layout.json`: the tile scans' grids with each one's first `p`, the
  positions with theirs, the focus maps, and each time-lapse's points, each `t`, when it was due
  and when it began, and the points skipped. A dataset is a `tile_scan` if it has a scan, a
  `z_stack` if stacks are all it has, and a `sequence` otherwise.
- **Refused before any dataset:** a stack of no planes or more than 1000, and a time-lapse with no
  interval, or neither a count nor a duration.

## Alternatives considered

- **One step per shape with fixed nesting,** as Micro-Manager's MDA orders time, position,
  slice and channel: simpler to show, but a plan's shape is its nesting, and the UI (P6.6, P6.7)
  can build any of them from these.
- **Stacks resolved into focus steps when the run starts,** as tile scans are: a stack is about
  the focus as it is then, after an autofocus or on a surface, which only the run knows.
- **`z` numbered in the order taken:** a stack taken down would then be upside down in the
  dataset, and readers would need its direction.
- **Each tile scan numbering its own `p`:** two scans, or one at each of several positions,
  would collide. A `p` per place in the plan's order needs nothing more of a reader than
  `layout.json`.
- **Positions in absolute coordinates only:** a scan about each of several positions would need
  one plan step per position, each with its own region.
- **Catching up late points back to back:** the intervals would no longer mean what they say,
  as `microscope`'s ADR-0031 found.
- **The schedule on the devices' clock:** it is the system's, so a test would wait in real time.
  Tokio's clock is the same in a run, and pauses in a test, so hours of schedule are tested in
  milliseconds.

## Consequences

- **Any composition counts:** the progress's total is known before the run, each repeat and point
  counted out, less any point skipped.
- **A down stack moves twice a plane** on a drive that arrives from below.
- **Positions are a plan's,** given with it: the saved positions that the UI offers are P6.5's.
- **A point that always overruns** with `skip` runs every other point, and with `shift` runs back
  to back, its intervals stretched. The record shows which.
- **An autofocus at a position that finds nothing** fails as any step does, and its policy
  decides.

## Verification

- `crates/siq-workflow/tests/engine.rs`, on tokio's paused clock:
  - `a_time_lapse_keeps_its_schedule_through_a_pause_skipping_the_points_it_missed`, and a cancel
    while it waits stopping it then;
  - `a_late_point_is_skipped_once_its_interval_has_passed_or_shifts_the_schedule`;
  - `a_multi_position_numbers_its_positions_and_takes_their_focus`.
- `crates/siq-workflow/tests/sequence.rs`, on the simulated instrument:
  - `a_z_stack_takes_its_planes_in_order_each_approached_the_same_way`, up and down;
  - `a_time_lapse_of_positions_scanned_in_z_stacks_takes_every_frame_once`: 2 points × 2
    positions × 2 tiles × 3 planes, 24 frames, every key once, 68 steps as counted;
  - as a property, a stack's planes a step apart about the middle.
- **`timelapse_sim`:** three points every 4 s at two positions, a stack of five at each, on
  schedule.
