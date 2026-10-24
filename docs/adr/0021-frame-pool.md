# ADR-0021: Frame pool design

- **Status:** Accepted
- **Date:** 2026-10-23
- **Phase:** P1.1
- **Related:** ADR-0005, ADR-0007, REQ-ACQ-002, REQ-NFR-002, REQ-NFR-003

## Context

- **The camera delivers 200 MB frames at 9 fps,** DMA'd by the Coaxlink grabber into buffers the
  application announces (eGrabber `UserMemory`).
- **Many consumers read each frame at once:** the previews, statistics, live stitching, the
  recorder, and workflow taps.
- **Nothing on this path may allocate or copy** (ADR-0005).
- **DMA buffers should stay resident.** A page-out under a running DMA, or on first touch, costs
  latency.
- **Rust's aliasing rules** must hold between a writer (the grabber or the simulator) and many
  readers on other threads.

## Decision

### Pool slots are the ring buffer

- **The pool allocates N slots once** (default 12): each a whole number of 4 KiB pages,
  page-aligned and zero-filled.
- **With a grabber,** each slot is one announced buffer. There is no byte ring and no copy: the
  DMA target is the frame consumers read.

### Ownership protocol

A slot is in exactly one state:

| State | Held by | Access |
|---|---|---|
| Free | the pool's free list | none |
| Filling | exactly one `FillGuard` (the simulator, or a grabber driver while the buffer is queued) | write (`bytes_mut`, `pixels_mut`, `image_mut`, `as_mut_ptr` for DMA) |
| Leased | one or more `FrameLease`s, and no guard | read (`as_bytes`, `image::<u8 or u16>()`) |

- **Guards are made only** by taking a slot from the free list, or when a slot's last lease
  drops.
- **Leases are made only** by `FillGuard::publish(meta)`, which consumes the guard, and by cloning
  a lease.
- **The brief's "Ready" state** is the instant of publishing: the first reader is the lease that
  `publish` returns.
- **Publish checks the image fits the slot.** It fails, giving the guard back, if
  `width × height × bytes per pixel` exceeds it.
- **The lease count** is a per-slot `AtomicU32`, with `Arc`'s orderings: a relaxed increment on
  clone; a decrement with `Release` and an `Acquire` fence on the last drop. A clone is one atomic
  increment and a copy of the `Copy` metadata.

### Release

When a slot's last lease drops, the slot goes to the pool's `ReleaseSink` as a new `FillGuard`, on
the dropping thread.

- **The default sink** (`ReturnToPool`) drops the guard, which returns the slot to the free list.
- **A grabber driver installs a sink** that requeues the buffer. It must be quick and must not
  block, which may mean handing the guard to the acquisition thread.
- **`reset_release_sink`** goes back to the default when the camera stops.

### Brief deviations, toward safety

- **`slot_ptrs()` became `take_all_free()`,** which returns guards. A pointer to announce
  (`FillGuard::as_mut_ptr`) is only available with ownership of the slot.
- **`ReleaseSink::release` takes a `FillGuard`,** not an index, so the sink owns what it may
  refill.

### Unsafe

Unsafe code is confined to `alloc.rs` (heap, `VirtualAlloc`, `VirtualLock`, the working set) and
`slot.rs` (raw slices under the protocol). Every block has a `SAFETY` comment, and the public API is
safe.

**The DMA rule is the driver's.** While a buffer is queued at the grabber, the driver doesn't touch
its slot through the guard.

### Locking

- **`Backend::WindowsLocked`:**
  - each slot is `VirtualAlloc`'d, so pages are zero-filled;
  - the process's minimum working set is raised by the pool's size plus 64 MiB, under a global lock,
    since pools share it;
  - each slot is then `VirtualLock`ed;
  - a failure is an error that names `[memory] pool_slots`;
  - dropping the pool unlocks and frees the slots, and lowers the minimum again.
- **`Backend::Heap`:** page-aligned `alloc_zeroed`, for tests, tools and Miri.

### Metrics

With a registry, a pool reports:

- `<name>.free_slots` and `<name>.in_use_high_water` (gauges);
- `<name>.lease_ms` (a histogram of first-publish-to-last-drop times, from the shared `Clock`).

`stats()` gives free, filling and leased counts, and the high-water mark.

### Deferred

**Large pages** (`MEM_LARGE_PAGES`, 2 MiB) would cut TLB misses on 200 MB frames. They need the
`SeLockMemoryPrivilege` granted to the service account, and contiguous physical memory. They are
revisited if P10.2's measurements show TLB pressure.

## Alternatives considered

- **A byte ring buffer with a copy out of the grabber's buffers:** 1.8 GB/s of `memcpy` for nothing.
- **`Arc<[u8]>` per frame:** an allocation per frame, and it can't be announced to a grabber.
- **State flags plus raw pointers (`slot_ptrs()`):** the soundness would depend on every caller,
  rather than on the types.

## Consequences

- **Consumers hold slots by holding leases.** A slow consumer that keeps leases starves the pool,
  and the grabber loses frames. The fan-out (P1.2) owns that policy.
- **Release sinks run on consumer threads,** so a grabber sink must be cheap.
- **The working-set maximum stays raised** after a pool is dropped. That is harmless.

## Verification

- `siq-buffer` tests:
  - lifecycle, release sink, refused publishes, views, alignment, invalid sizes, metrics, leases
    outliving the pool;
  - the property test `slots_are_never_shared_with_a_writer`: random sequences against a model,
    with a stamp per fill that every lease must keep reading;
  - a producer with two consumer threads over 2000 frames;
  - `the_steady_state_allocates_nothing`: a counting global allocator, with a self-check that it
    counts;
  - `locked_slots_are_aligned_and_the_working_set_is_raised_then_restored`, on the real system.
- `pool_locked` locked 12 × 194 MB (2330 MB) in 945 ms on the development PC.
