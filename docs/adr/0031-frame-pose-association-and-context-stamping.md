# ADR-0031: Frame↔pose time association, and context stamping

- **Status:** Accepted
- **Date:** 2026-10-24
- **Phase:** P2.1
- **Related:** ADR-0005, ADR-0018, ADR-0022, ADR-0023, ADR-0029, ADR-0030, REQ-ACQ-004,
  REQ-ACQ-005, REQ-ACQ-006, REQ-STG-003

## Context

- **Every frame must say where the stage and the focus were** during its exposure, and how well
  that is known (REQ-ACQ-004). Stitching trusts settled poses, and treats moving ones as guesses.
- **The stage and focus are polled, not streamed.** The GRBL driver reads at 5 Hz, and the ASCII
  focus drive at 10 Hz. Each reading is timestamped on the shared clock, at its arrival less half
  the round trip (ADR-0029, ADR-0030). The camera stamps each frame's mid-exposure on the same
  clock.
- **A reading says nothing about what happened after it.** A move may start just after a still
  reading. A frame is only known to be settled once a still reading comes after it, which can be
  a polling period later.
- **Live preview must not wait** (the P2.2 budget is ≤ 60 ms). Snaps and saved frames can wait,
  for a pose that won't change.
- **The context** (channel, objective, calibration) is set by the UI and the workflows, between
  frames.

## Decision

### The pose timeline

`PoseTimeline` keeps the last 256 readings of each group it follows: the stage's x, y and z, and
the focus. A watch of each status feeds it, and readings that arrive a little out of order go into
their place. `pose_at(t)` is a pure function of the readings:

| Readings around `t` | Pose | Quality |
|---|---|---|
| One at or before `t` and one after, both still, at the same position | That position | `Settled` |
| One at or before and one after, otherwise | Linear between them | `Interpolated`, with the uncertainty the distance between them: speed × spacing |
| Only one at or before: past the last reading | Carried on at the speed between the last two, or held if the last was still | `Interpolated`, with the uncertainty the last speed × spacing: 0 if still |
| None at or before | None | `Unknown` |

- **Combining the groups:**
  - the pose takes x, y and z from the stage, and the focus from the focus drive, or 0 without
    one;
  - it is settled only if both groups are;
  - otherwise, its uncertainty is the larger of the two;
  - a focus drive that is followed but has no reading makes the pose `Unknown`.
- **A frame is flagged `moving`** unless both readings either side of it are still at the same
  place.
- **`decided(t)`** says that every followed group has a reading after `t`, so `pose_at(t)` won't
  change.

### When a frame is stamped

The acquisition engine is the camera's `FrameSink` (ADR-0023). Each frame, on the camera's thread,
goes through these steps:

1. It is renumbered. The camera counts from 0 at each start, and the engine adds the number it had
   reached. So numbers go on across mode switches and reconfigurations, and a gap means lost
   frames, and only that.
2. It is stamped with the context in force at its mid-exposure. `set_context` records each change
   with its time on the clock, and the engine keeps the last 8. So a frame exposed before a
   change, but arriving after it, keeps the old context.
3. If a capture is under way, it is marked (`flags.save`), and kept for the capture.
4. It is queued, in arrival order:
   - **A live frame** that isn't captured goes at once, with the pose as known now.
   - **A captured frame, a snap and a sequence's frame** wait until the pose at their
     mid-exposure is decided. They wait at most `POSE_WAIT`, 500 ms on the clock: two readings at
     5 Hz, and a little more. They then go with the pose as far as it is known.
5. Frames leave the queue in order, stamped with `pose_at(mid-exposure)`, and are published to the
   fan-out. A frame that waits holds up those behind it, so consumers always see numbers in
   order.

The queue is checked on each frame, on each reading, and every 50 ms. Its capacity is the pool's
slot count, so the frame path never allocates.

### Modes

- **Live:** the camera free-running.
- **Snap, settle-aware:**
  1. the camera is software-triggered;
  2. the snap waits until the last reading of every followed group is still;
  3. it triggers;
  4. it takes the frame through a one-frame capture, once its pose is decided.
- **Sequence:** `n` snaps, with a hook called with each frame before the next, to step the focus
  for a z-stack, say. Free-running bursts come from `capture_next` in live view.
- **Switching** between free-run and trigger stops and restarts the camera.
- **A change of region or binning** that the camera refuses while acquiring (`Busy`) stops it,
  reconfigures it and restarts it in the same mode. The exposure and gain change at once.
- **One operation at a time:** starts, stops, snaps, sequences and reconfigurations take turns.

### Captures and statistics

- **`capture_next(n, save)`** marks the next `n` frames to arrive, and hands them over on a
  channel once published. They hold their slots until they are dropped.
- **`stats()`:**
  - the mode;
  - the rate over the last 16 frames;
  - the frames published;
  - the frames lost for want of a slot, and by the device, since the engine was made;
  - the frames waiting for their pose;
  - each consumer's fan-out statistics: received, dropped, taken and the most it has queued.

## Alternatives considered

- **Stamp every frame at once:** then a snap taken right after a settle could never be `Settled`,
  and a frame exposed as a move began would be.
- **Make every frame wait:** that adds up to a polling period to live preview.
- **Stamp later, in each consumer:** every consumer would need the timeline, and the recorder and
  stitching could disagree about a frame's pose.
- **Ask the drivers for a status at each frame:** that is 9 queries a second on a serial link
  that GRBL suggests polling at 5 Hz, and still not after the exposure.
- **Extrapolate with a configured maximum speed** for the uncertainty past the last reading: more
  conservative, but it needs per-axis limits that the engine doesn't have. The quality is
  `Interpolated` either way, so consumers that need certainty wait for `Settled`.

## Consequences

- **A snap's pose is final, and `Settled` when the stage was still,** at the cost of up to a
  polling period of latency.
- **A live frame's pose can be carried on past the last reading.** At the end of a move it may
  overshoot the target, by up to its stated uncertainty, until the next reading.
- **A context change takes effect for frames exposed after it.** A channel switch (P2.4) sets the
  context once its devices have settled. A frame exposed while they were changing is stamped with
  the old channel, so P2.4 drops it, or switches with live stopped.
- **The engine must be stopped before it is dropped,** or the camera runs on, discarding frames.

## Verification

- **`siq-acquire` `timeline` tests,** with proptests:
  - interpolation, and any path between two readings lying within the uncertainty;
  - settled only between still readings at the same place;
  - carrying on past the last reading;
  - the focus joining the stage;
  - order and capacity.
- **`siq-acquire` `acquire` tests,** with the simulated camera and the GRBL driver against its
  emulator:
  - a still frame's pose within the injected error, with its context;
  - the context in force at each frame's mid-exposure;
  - moving frames interpolated within their uncertainty;
  - numbering across modes and a reconfiguration;
  - captures, and lost-frame accounting.
- **The `acquire_live` example.**
