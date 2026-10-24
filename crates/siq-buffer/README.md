# siq-buffer

SpectrumIQ's frame pool. Slots are allocated once, page-aligned and optionally locked in RAM.

- The camera's DMA, or the simulator, fills a slot under a `FillGuard`.
- Consumers read the published frame through cheap, shared `FrameLease`s.
- When the last lease drops, the slot goes back to the grabber, or to the free list.

No allocation and no copy happen on the frame path. The design is in
[ADR-0021](../../docs/adr/0021-frame-pool.md).

## Place in the architecture

- **Layer:** L1 Core.
- **Depends on:** `siq-types` (`FrameMeta`, image views, `Clock`), `siq-telemetry` (metrics),
  `windows-sys` (`VirtualAlloc`, `VirtualLock`, the working set), `thiserror`.
- **Used by:**
  - P1.2's fan-out, in this crate;
  - P1.7's simulated camera;
  - P2.1's acquisition engine;
  - P10.1's Euresys driver.
- **Phase:** P1.1.

## API

| Item | What |
|---|---|
| `FramePool::new(&PoolConfig, clock, registry)` | Allocates `slots` slots of at least `slot_bytes` each (rounded to pages), from `Backend::Heap` or `Backend::WindowsLocked` |
| `try_acquire_free()`, `take_all_free()` | `FillGuard`s for Free slots: to fill, or to announce to a grabber |
| `FillGuard` | `bytes_mut`, `pixels_mut::<u8 or u16>`, `image_mut(width, height)`, `as_mut_ptr` (for DMA), and `publish(meta)` → `FrameLease`. Dropping it frees the slot |
| `FrameLease` | `meta`, `as_bytes`, `image::<u8 or u16>()`. `clone` is an atomic increment. The last drop goes to the release sink |
| `ReleaseSink`, `ReturnToPool` | Where released slots go; `set_release_sink` and `reset_release_sink` |
| `stats()` → `PoolStats` | Free, filling, leased, and the high-water mark |
| `working_set()` | The process's minimum and maximum working set |

```rust
let pool = FramePool::new(&config, clock, Some(&registry))?;
let mut guard = pool.try_acquire_free().expect("a free slot");
render_into(guard.image_mut::<u16>(width, height)?);
let lease = guard.publish(meta)?;                 // shared, read-only from here
consumers.send(lease.clone());
```

With a registry, the pool reports `<name>.free_slots`, `<name>.in_use_high_water` and
`<name>.lease_ms`.

## Examples

| Example | What it shows | Run |
|---|---|---|
| `pool_locked` | Locks a real-size pool, 12 × 194 MB (2330 MB, about 1 s on the development PC), runs frames through every slot, and shows the working set before, with, and after the pool. Smoke mode uses 4 × 8 MB | `cargo run --release -p siq-buffer --example pool_locked` |

## Features

None.

## Testing

`cargo test -p siq-buffer` runs five integration-test binaries:

| Binary | What |
|---|---|
| `pool` | Lifecycle, release sink, refused publishes, views, alignment, sizes, metrics, and leases outliving the pool |
| `state_machine` | Property test: random operations against a model. A stamp per fill must stay readable through every lease |
| `concurrency` | A producer and two consumer threads, 2000 frames |
| `no_alloc` | A counting global allocator, which checks itself, sees no allocation in 1000 steady-state cycles with metrics on. It is the test files' one `unsafe` exception, needed for an allocator |
| `windows_locked` | Real `VirtualLock`, and the working set raised then restored |

**Miri** could check the Heap backend's unsafe code:
`cargo +nightly miri test -p siq-buffer --test pool --test state_machine`. It hasn't been run yet,
because no nightly toolchain is installed.

## Limitations

- **The DMA rule is the driver's.** While a grabber may write a queued slot, don't touch it through
  its guard. The types can't see DMA.
- **Release sinks run on the thread that drops the last lease.** A grabber sink must be quick.
- **Large pages are deferred** (ADR-0021).
- **The working-set maximum stays raised** after a pool is dropped. That is harmless.
