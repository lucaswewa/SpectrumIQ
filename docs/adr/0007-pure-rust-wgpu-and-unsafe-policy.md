# ADR-0007: Pure Rust + wgpu for algorithms; unsafe and native-code allow-list

- **Status:** Accepted
- **Date:** 2026-10-23
- **Phase:** planning
- **Related:** REQ-NFR-005

## Context

The project is MIT open source on Windows. OpenCV would bring a large native build (plus LLVM for
its bindings) onto every machine. Most of the needed algorithms are tractable in Rust: FFT, phase
correlation, least squares, blending, DFF, Richardson–Lucy, and light-field resampling.

## Decision

- **Algorithms are pure Rust** (`ndarray`, `rustfft`/`realfft`, `nalgebra`, `rayon`) plus **wgpu**
  compute shaders (WGSL). Each kernel has a CPU reference.
- **Native code is allowed only for:**
  1. the Euresys shim (ADR-0004);
  2. Windows OS APIs through `windows`/`windows-sys` (memory locking, thread priority, ACLs,
     Media Foundation, shared memory);
  3. `zstd` (through `zstd-sys`), for OME-Zarr.

  Any other native library needs an ADR.
- **`unsafe_code = "deny"` workspace-wide.** It is allowed only in:
  - `siq-euresys-sys` and `siq-euresys`;
  - `siq-buffer::{alloc, slot}`;
  - `siq-shm`;
  - small Windows-API modules, listed in their crate READMEs.

  Every block carries a `// SAFETY:` comment, behind a safe public API.

## Alternatives considered

- **OpenCV:** SIFT/ORB and warps for free, but heavy and native.
- **Case-by-case:** this ADR already permits exceptions through new ADRs.

## Consequences

- Some algorithms (a feature matcher, the GPU FFT) are written in-house (P8.6, P12.3).
- Builds stay simple: MSVC + Rust, and LLVM only to regenerate bindings.

## Verification

- `cargo deny` license checks.
- `cargo xtask layers`.
- Review of unsafe blocks in each phase that adds them.

## Amendment (P1.1, 2026-10-23): test-only global allocators

A test that proves code allocates nothing needs a counting `#[global_allocator]`, which can only be
written with `unsafe`. Such a test may allow `unsafe_code` in its own file, if:

- it is the only test in its binary;
- the allocator forwards every call to `System` unchanged;
- each block has a `SAFETY` comment.

The first is `siq-buffer`'s `tests/no_alloc.rs`. Production code is unaffected.

## Amendment (P4.1, 2026-10-25): mapping recorded frames

`siq-storage::raw::map` may use `unsafe` to map a recorded raw frame with `memmap2`, and to view
its bytes as `u16`. ADR-0041 gives the argument: the file is open sharing reading only for as long
as it is mapped, so nothing can change it meanwhile. `siq-storage::disk` is a small Windows-API
module, which this ADR already allows. Both are listed in the crate's README.
