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

## Amendment (P2.5, 2026-10-24): the shim's API, its build, and Playlink

### The C ABI (`crates/siq-euresys-sys/shim/siq_egrabber.h`)

| Group | Functions | Notes |
|---|---|---|
| ABI and errors | `siq_eg_abi_version`, `siq_eg_last_error(buf, len)` | The message is the last failure's on the calling thread |
| System | `siq_eg_system_open(producer, &system)`, `siq_eg_system_close` | One GenTL producer (`.cti`), or the default one (`EURESYS_DEFAULT_GENTL_PRODUCER`) |
| Discovery | `siq_eg_discover(system, &counts)`, `siq_eg_discovery_info(system, found, index, buf, len, &needed)` | Counts of interfaces, grabbers and cameras, and `key=value` lines for each |
| Grabber | `siq_eg_open(system, camera, &grabber)`, `siq_eg_close` | A discovered camera, opened as `EGrabber<CallbackOnDemand>` |
| Features | `siq_eg_get_int`, `_float`, `_string`, `siq_eg_set_int`, `_float`, `_string`, `siq_eg_execute` | By name, in one of four modules: interface, device, stream, remote |
| Buffers | `siq_eg_announce_user_buffers(ptrs, size, count)`, `siq_eg_requeue(index)` | Our memory, filled by DMA. The user pointer carries our index |
| Acquisition | `siq_eg_start(frames)`, `siq_eg_stop`, `siq_eg_pop(timeout_ms, &info)`, `siq_eg_cancel_pop` | `info` is a `SiqBufferInfo`; 0 frames means until stopped |
| Counters | `siq_eg_counters(&counters)` | GenTL's `STREAM_INFO_NUM_*`: `underrun` is frames lost for want of a buffer |

- **Every function but the closers returns a `SiqEgStatus`:**
  - `OK`;
  - `TIMEOUT` and `ABORTED`, for `pop`;
  - `TRUNCATED`, when a string didn't fit;
  - `INVALID_ARGUMENT`;
  - `GENTL_ERROR`, with GenTL's code in the message;
  - `ERROR`.
- **Each entry point runs inside a guard** that catches every exception, `...` included.
- **Strings are copied into the caller's buffer,** with the size needed reported, so no memory
  crosses the ABI.
- **Discovery opens cameras, not indexes.** `EGrabberDiscovery` takes exclusive control of the
  producer's device lists, so grabbers are made from its `EGrabberCameraInfo`.
- **`SiqBufferInfo`** is 64 bytes: index, pointer, bytes filled, timestamp, frame id, width,
  height, pixel format and its GenTL namespace, and flags.
  - The timestamp is in ns: from `BUFFER_INFO_TIMESTAMP_NS`, or else µs × 1000.
  - The pixel format is interpreted through its namespace: 4 is PFNC. A code with bit 31 set is a
    custom PFNC code.
- **The layout is asserted on both sides:** `static_assert`s in the C++, and the bindings' own
  layout checks in Rust.

### The build, and finding the SDK at run time

- **With the `sdk` feature,** `build.rs` looks for `include/EGrabber.h` under `EGRABBER_SDK_DIR`
  (default `C:\Program Files\Euresys\eGrabber`). If it isn't there, the build stops with a
  message.
- **The shim is compiled with `cc`,** using MSVC with `/std:c++17 /EHsc` and `/W4`. The SDK's
  headers are `/external:W0`, so only the shim's own warnings show.
- **Nothing is linked but the shim.** The SDK's header loads `egrabber.dll` at run time, from
  `EURESYS_EGRABBER_LIB64`, and the GenTL producer from its `.cti` path.
- **Without `sdk`,** the types and constants build, and the functions aren't linked.
- **`src/bindings.rs` is committed.** `cargo xtask regen-bindings` regenerates it with
  `bindgen-cli`, which needs libclang. The options are:
  - allowlists for `siq_eg_*`, `Siq*` and `SIQ_EG_*`;
  - enums as constants with their C names;
  - one `unsafe extern` block, for Rust 2024.
- **`cargo xtask ci` fails if a tracked file has an SDK file's name** (REQ-NFR-005): its headers,
  `.cti` producers, or `egrabber` libraries.

### Findings on this PC

- **Hardware is present:** a Coaxlink Quad CXP-12 (`PC3603`, 1-camera firmware), on
  `coaxlink.cti`, with an Adimec D-103A12c/CXP-T01.
  - Its features, read through the shim (`eg_discover --open`): 11264 × 9200 (103.6 MP), pixel
    format `BayerGB8`, and a payload of 103,628,800 bytes.
  - It is a colour (Bayer) camera, not the 14-bit mono camera of REQ-ACQ-001. This is open
    question 1 of the plan.
- **Playlink (`playlink.cti`) works without hardware.** It offers one virtual camera, "Logo":
  - 720 × 720 at 30 frames/s, in `CustomJFIF` (PFNC namespace, custom code `0x83000001`);
  - its licence status is "NotOK", which didn't stop acquisition;
  - it fills user memory, pops, requeues and counts like a real grabber;
  - its timestamps are real, 33.3 ms apart, but its frame ids are all 0.

  So Playlink can test the real code path from P10.1 on, except for frame numbering, and for
  pixel formats, DMA timing and losses, which need the camera.
