# ADR-0004: Euresys eGrabber through a C++ shim and bindgen

- **Status:** Accepted
- **Date:** 2026-10-23
- **Phase:** planning
- **Related:** REQ-ACQ-001, REQ-ACQ-002, REQ-NFR-005

## Context

eGrabber 25.02.0.40 is installed at `C:\Program Files\Euresys\eGrabber`.

- **The public API** (`include/EGrabber.h`) is C++ and header-only (templates over callback
  models).
- **The header loads `egrabber.dll` at run time,** through `EURESYS_EGRABBER_LIB64`, which the
  installer sets to `…\cti\x86_64`.
- **A flat C API exists** (`include/internal/generated/cEGrabber.generated.h`, about 8,000 mangled
  symbols such as `euEGCOD_popFu64`). It is internal and undocumented.
- **The features SpectrumIQ needs exist:**
  - `announceAndQueue(UserMemory)`, for DMA into our pool;
  - `pop(timeout)` with `CallbackOnDemand`;
  - buffer info (timestamp, frame id).
- **The EULA (§4)** forbids copying the software except as expressly allowed. Nothing grants
  redistribution of SDK files.
- **libclang** (needed by bindgen) is not installed on the development machine.

## Decision

- **A small C++17 shim** (`crates/siq-euresys-sys/shim/`) wraps `EGrabber<CallbackOnDemand>`
  behind a plain C header with opaque handles. It catches every exception and turns it into an
  error code plus a message. No exception crosses the ABI.
- **bindgen runs only on our shim header.** The generated `bindings.rs` is committed, and
  `cargo xtask regen-bindings` (which needs LLVM) regenerates it. Normal builds need no libclang.
- **`build.rs` compiles the shim** with `cc` (MSVC) against the installed SDK
  (`EGRABBER_SDK_DIR`, defaulting to the install path), only with the `sdk` feature. The default
  build needs no SDK, and works with the simulator.
- **The safe wrapper** is `siq-euresys`, which implements the `Camera` trait (P10.1).
- **Acquisition uses one dedicated thread** with `pop(timeout)` (CallbackOnDemand), not SDK
  callback threads. Ownership is then explicit, and nothing re-enters across FFI.
- **No SDK file is committed.**
- **The risk spike comes first:** P2.5 builds the shim early. `playlink.cti` (Euresys' virtual
  device) is evaluated for hardware-free testing of the real code path.

## Alternatives considered

- **bindgen on the internal C API:** no C++ needed, but the API is undocumented, and its mangled
  names may change between SDK versions.
- **bindgen on the raw GenTL C API:** standard, but GenICam feature access would have to be
  reimplemented.
- **The `cxx` crate:** heavier build integration for a small surface.

## Consequences

- A C++ compiler (MSVC) is needed for builds with the `sdk` feature.
- SDK upgrades are absorbed by the C++ header behind our stable shim API.
- The shim is about 400 lines of C++ to maintain.

## Verification

- P2.5: `eg_discover` runs against the installed SDK.
- P10.1 and P10.2: hardware tests (grab, no lost frames, timestamps).
