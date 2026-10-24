# ADR-0022: Fan-out consumer policies and backpressure

- **Status:** Accepted
- **Date:** 2026-10-23
- **Phase:** P1.2
- **Related:** ADR-0005, ADR-0021, REQ-ACQ-005, REQ-NFR-003

## Context

- **One producer per camera, many consumers.** The previews, statistics, live stitching, the
  recorder and workflow taps all read the same frames, at different paces and with different needs.
  - A preview wants only the newest frame.
  - Stitching wants a short, ordered run of frames.
  - The recorder wants every frame flagged to save, and nothing else.
- **Consumers hold slots by holding leases** (ADR-0021). A slow consumer that keeps leases starves
  the pool, and the grabber then loses frames.
- **The producer must never wait for a consumer.** The grabber's buffer queue, and every other
  consumer, would stall behind the slowest one.
- **Lost frames must be counted where they are lost** (REQ-ACQ-005).

## Decision

### One mailbox per subscriber, with a policy

`Fanout::subscribe(name, policy, filter)` returns a `Subscriber`, with its own mailbox.

| Policy | Keeps | When full | For |
|---|---|---|---|
| `LatestOnly` | 1 frame | the new frame replaces the unread one | previews, focus metrics, live statistics |
| `Bounded { capacity, overflow }` | up to `capacity` frames, in order | `DropOldest` or `DropNewest` | stitching, short histories |
| `Lossless` | every frame | never full: the pool runs dry first | the recorder, workflow taps |

- **A filter on `FrameMeta`** chooses which frames a subscriber is offered: for example, the
  recorder takes only frames with `flags.save`. It runs on the producer's thread, so it must be
  cheap.
- **Mailboxes are allocated at subscription,** with room for their capacity. A lossless mailbox has
  room for the pool's slot count, the most frames that can exist. Publishing allocates nothing.

### Backpressure is through slots only

- **`publish` never waits for a consumer.** It clones the lease into each mailbox that takes it, a
  single atomic increment each.
- **A dropped frame is counted at its mailbox:** `dropped` in that subscriber's statistics.
- **A lossless consumer that falls behind** keeps its frames, so it keeps their slots. The pool runs
  dry, and the producer loses the next frames. These are counted at the producer (the acquisition
  engine in P2.1, the grabber in P10.1) as lost frames, not hidden as memory growth.
- **Pool sizing must cover the subscribers.** A subscriber can hold its queue plus the frame it is
  working on:
  - `LatestOnly`: 2 slots;
  - `Bounded`: `capacity + 1` slots;
  - `Lossless`: as many as it falls behind.

  The pool needs those, plus the frames queued at the grabber. P2.1 checks this at start-up.

### Never blocking, precisely

- **`publish` takes two kinds of lock,** each for constant work:
  - the subscriber list's read lock, for the whole call;
  - each mailbox's mutex, to push or replace one frame.
- **No consumer code runs under either lock.** Wakers and condition variables are notified after
  the mailbox is unlocked, and frames a policy drops are released after it too.
- **Subscribing and unsubscribing** take the list's write lock, briefly. They are rare.
- **A frame a policy drops is released on the producer's thread.** If it was the frame's last lease,
  the pool's release sink runs there, so the sink must not subscribe or unsubscribe.

### Receiving

- **From threads:** `try_recv`, `recv_blocking` and `recv_timeout`.
- **From async code:** `recv()` is a future written on `std::task::Waker`, so it runs on any
  executor. `siq-buffer` (L1) doesn't depend on tokio.
- **One reader per subscriber:** the receiving methods take `&mut self`.

### Closing and unsubscribing

- **When the last `Fanout` handle drops, or on `close()`,** subscribers read what is queued, and then
  get `None` (`RecvError::Closed` from `recv_timeout`). A subscriber made after closing is closed
  from the start.
- **Dropping a `Subscriber` unsubscribes it.** The frames still in its mailbox are released; frames
  it has taken stay valid.

### Statistics

`SubscriberStats` holds:

- `received`: the frames the filter took;
- `dropped`: the frames the policy dropped, unread;
- `taken`: the frames read;
- `max_queued`: the most frames waiting at once, the brief's "max lag".

`received = taken + dropped + queued` always holds. `Fanout::stats()` lists every subscriber's
statistics by name.

### Brief deviations

- **`Bounded { n, drop: Oldest | Newest }`** became `Bounded { capacity: NonZeroUsize, overflow:
  Overflow::DropOldest | DropNewest }`. An empty mailbox can't be asked for, and the field isn't
  called `drop`.
- **"Max lag"** is `max_queued`, alongside `taken`.
- **`publish` returns** how many subscribers took the frame.
- **The soak test runs a fixed number of frames** (3000), with two slow consumers and a lossless
  one. The timed soak, at the camera's rate and frame size, is the `fanout_soak` example.

### Deferred

**Metrics in the registry** (`<subscriber>.received`, `.dropped`, `.max_queued`) are left to P2.1,
which names the subscribers and owns the acquisition metrics.

## Alternatives considered

- **`tokio::sync::broadcast`:** one capacity and one lossy policy for every receiver, no lossless
  mode, and a dependency on tokio in L1.
- **A channel per subscriber (std or crossbeam):** a bounded channel blocks or fails when full. It
  can't drop the oldest frame without the receiver's side.
- **Blocking the producer on a lossless subscriber:** the grabber's queue drains and frames are lost
  there anyway, uncounted by us, and every other consumer stalls.
- **Lock-free rings per mailbox:** not needed at 9 frames/s. Even at 1000 frames/s with an ROI, a
  mutex costs about 100 ns per subscriber. They are revisited if P10.2 shows contention.

## Consequences

- **The recorder's losses show as lost frames at the producer,** where REQ-ACQ-005 counts them.
- **Filters and release sinks run on the producer's thread.** Both must be quick.
- **Pool size and subscriber policies are sized together** (P2.1).

## Verification

- `siq-buffer` tests (`tests/fanout.rs`):
  - each policy's semantics, and frames going back to the pool as they are replaced;
  - the filter;
  - unsubscribing with frames queued and taken, and subscribers coming and going while a producer
    thread publishes;
  - closing: draining, then `None`, from every receiving path;
  - a waiting thread, and an async consumer on a thread-park executor, woken per frame;
  - a soak of 3000 frames with two slow consumers and a lossless one: the counts add up, the slow
    ones drop and the lossless one doesn't, and every slot comes back.
- `the_steady_state_allocates_nothing` also runs 1000 cycles through three subscribers, one per
  policy, by `try_recv`, `recv_timeout` and `recv`, with no allocation.
- The `fanout_soak` example, for 60 s at 9 frames/s with 12 × 194 MB slots (smoke: 2 s at
  100 frames/s with small frames).
