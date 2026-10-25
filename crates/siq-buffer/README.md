# siq-buffer

SpectrumIQ's frame pool and fan-out. Slots are allocated once, page-aligned and optionally locked
in RAM.

- The camera's DMA, or the simulator, fills a slot under a `FillGuard`.
- Consumers read the published frame through cheap, shared `FrameLease`s.
- The `Fanout` hands each frame to its subscribers, each with a policy and a filter, and never
  makes the producer wait.
- When the last lease drops, the slot goes back to the grabber, or to the free list.

No allocation and no copy happen on the frame path. The designs are in
[ADR-0021](../../docs/adr/0021-frame-pool.md) (the pool) and
[ADR-0022](../../docs/adr/0022-fanout-consumer-policies-and-backpressure.md) (the fan-out).

## Place in the architecture

- **Layer:** L1 Core.
- **Depends on:** `siq-types` (`FrameMeta`, image views, `Clock`), `siq-telemetry` (metrics),
  `windows-sys` (`VirtualAlloc`, `VirtualLock`, the working set), `thiserror`.
- **Used by:**
  - P1.7's simulated camera;
  - P2.1's acquisition engine;
  - P10.1's Euresys driver.
- **Phases:** P1.1 (the pool), P1.2 (the fan-out).

## API

### The pool

| Item | What |
|---|---|
| `FramePool::new(&PoolConfig, clock, registry)` | Allocates `slots` slots of at least `slot_bytes` each (rounded to pages), from `Backend::Heap` or `Backend::WindowsLocked` |
| `try_acquire_free()`, `take_all_free()` | `FillGuard`s for Free slots: to fill, or to announce to a grabber |
| `FillGuard` | `bytes_mut`, `pixels_mut::<u8 or u16>`, `image_mut(width, height)`, `as_mut_ptr` (for DMA), and `publish(meta)` → `FrameLease`. Dropping it frees the slot |
| `FrameLease` | `meta`, `as_bytes`, `as_padded_bytes` (rounded up to whole pages, for direct I/O; P4.1), `image::<u8 or u16>()`. `clone` is an atomic increment. The last drop goes to the release sink |
| `ReleaseSink`, `ReturnToPool` | Where released slots go; `set_release_sink` and `reset_release_sink` |
| `stats()` → `PoolStats` | Free, filling, leased, and the high-water mark |
| `working_set()` | The process's minimum and maximum working set |
| `process_memory()` → `ProcessMemory` | The process's working set, its peak, and its private bytes (since P2.4, for `/api/siq/system/memory`) |

With a registry, the pool reports `<name>.free_slots`, `<name>.in_use_high_water` and
`<name>.lease_ms`.

### The fan-out

| Item | What |
|---|---|
| `Fanout::new(&pool)` | A fan-out for the pool's frames. Clones share it; it closes when the last one drops, or on `close()` |
| `subscribe(name, policy, filter)` → `Subscriber` | A mailbox with a `Policy` (`LatestOnly`, `Bounded { capacity, overflow: DropOldest or DropNewest }`, `Lossless`), offered the frames `filter(&FrameMeta)` takes |
| `publish(&lease)` | Offers the frame to every subscriber, and returns how many took it. It never waits for a consumer, and allocates nothing |
| `Subscriber` | `try_recv`, `recv_blocking`, `recv_timeout` → `RecvError::{Timeout, Closed}`, and the future `recv()`, for any executor. Dropping it unsubscribes it |
| `SubscriberStats` | `received`, `dropped`, `taken` and `max_queued`, from `Subscriber::stats()` or `Fanout::stats()` |

```rust
let pool = FramePool::new(&config, clock, Some(&registry))?;
let fanout = Fanout::new(&pool);
let mut preview = fanout.subscribe("preview", Policy::LatestOnly, |_| true);
let mut recorder = fanout.subscribe("recorder", Policy::Lossless, |meta| meta.flags.save);

let mut guard = pool.try_acquire_free().expect("a free slot");
render_into(guard.image_mut::<u16>(width, height)?);
fanout.publish(&guard.publish(meta)?);            // shared, read-only from here

while let Some(frame) = recorder.recv().await { save(&frame) }
```

**A lossless subscriber that falls behind keeps its slots,** so the pool runs dry and the producer
loses frames. That is the backpressure, by design (ADR-0022). Size the pool for the subscribers:
2 slots for `LatestOnly`, `capacity + 1` for `Bounded`, plus the frames queued at the grabber.

## Examples

| Example | What it shows | Run |
|---|---|---|
| `pool_locked` | Locks a real-size pool, 12 × 194 MB (2330 MB, about 1 s on the development PC), runs frames through every slot, and shows the working set before, with, and after the pool. Smoke mode uses 4 × 8 MB | `cargo run --release -p siq-buffer --example pool_locked` |
| `fanout_soak` | 60 s at the camera's 9 frames/s with 12 × 194 MB slots, to a slow preview, a slower statistics view and a lossless writer of one frame in ten. It prints what each received, read and dropped, and checks the counts and the pool. Smoke mode runs 2 s at 100 frames/s with small frames | `cargo run --release -p siq-buffer --example fanout_soak` |

## Features

None.

## Testing

`cargo test -p siq-buffer` runs six integration-test binaries:

| Binary | What |
|---|---|
| `pool` | Lifecycle, release sink, refused publishes, views, alignment, sizes, metrics, and leases outliving the pool |
| `state_machine` | Property test: random operations against a model. A stamp per fill must stay readable through every lease |
| `concurrency` | A producer and two consumer threads, 2000 frames |
| `fanout` | Each policy, the filter, unsubscribing with frames in flight, closing, waking threads and async consumers, and a 3000-frame soak with two slow consumers and a lossless one |
| `no_alloc` | A counting global allocator, which checks itself, sees no allocation in 1000 steady-state pool cycles with metrics on, then in 1000 fan-out cycles through all three policies. It is the test files' one `unsafe` exception, needed for an allocator |
| `windows_locked` | Real `VirtualLock`, and the working set raised then restored |

**Miri** could check the Heap backend's unsafe code:
`cargo +nightly miri test -p siq-buffer --test pool --test state_machine`. It hasn't been run yet,
because no nightly toolchain is installed. The fan-out has no unsafe code.

## Limitations

- **The DMA rule is the driver's.** While a grabber may write a queued slot, don't touch it through
  its guard. The types can't see DMA.
- **Release sinks run on the thread that drops the last lease:** a consumer's, or the producer's for
  a frame a policy drops. A sink must be quick, and must not subscribe or unsubscribe.
- **Filters run on the producer's thread.** They must be cheap.
- **Fan-out metrics aren't in the registry yet.** P2.1 adds them (ADR-0022).
- **Large pages are deferred** (ADR-0021).
- **The working-set maximum stays raised** after a pool is dropped. That is harmless.
