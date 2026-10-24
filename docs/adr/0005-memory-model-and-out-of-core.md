# ADR-0005: Memory model: pinned frame pool, streaming and out-of-core processing

- **Status:** Accepted
- **Date:** 2026-10-23
- **Phase:** planning
- **Related:** REQ-ACQ-002, REQ-NFR-002, REQ-NFR-003, REQ-STI-003

## Context

- **Frames:** 200 MB each, 1.8 GB/s at full rate.
- **Workstation (fixed, already working):**
  - mini-ITX, AMD CPU;
  - **32 GB RAM**;
  - **RTX 3060 8 GB**.
- **Data sizes:**
  - a 50-slice z-stack is 10 GB;
  - a 500 MP mosaic is 1 GB per channel and plane.
- **Runtime allocations** on the frame path cause latency spikes.

## Decision

- **Frame pool:** N slots (default 12 = 2.4 GB), allocated once, page-aligned (`VirtualAlloc`), and
  locked (`VirtualLock`, after raising the working-set minimum). The slots are the buffers announced
  to the grabber, so the pool is the ring buffer.
- **Leases and fan-out:** frames move as reference-counted leases, and consumers have policies
  (`LatestOnly`, `Bounded`, `Lossless`). Nothing is allocated or copied in steady state.
- **No algorithm assumes its dataset fits in RAM.**
  - DFF keeps running maps instead of the stack.
  - Deconvolution works on overlapping blocks.
  - Compositing works per output chunk.
  - Pyramids are built level by level.
  - Raw tiles are read through memory maps.
- **Jobs declare their RAM and VRAM estimates.** The scheduler admits them within the configured
  budget (architecture §5).
- **GPU work** is banded or tiled to respect binding-size limits and the VRAM budget.

## Alternatives considered

- **A byte ring buffer with memcpy:** costs 1.8 GB/s of copies.
- **In-memory processing:** impossible at 32 GB for the target data sizes.

## Consequences

- More complex algorithms, which are designed blockwise from the start.
- The architecture also scales to gigapixel mosaics, and to bigger machines when available.

## Verification

- P1.1/P1.2: no allocation in steady state.
- P8.3, P8.5, P11.1, P12.2: peak memory tests within the budget.
