# siq-euresys-sys

Euresys eGrabber, for the Coaxlink CoaXPress grabber, behind a C ABI of our own (ADR-0004). It
holds a C++17 shim over eGrabber's header-only C++ API, and the Rust bindings of the shim's header.
No SDK file is in this crate. With the `sdk` feature, the shim is compiled against the installed
SDK.

## Place in the architecture

- **Layer:** L2 Devices, as raw FFI. One of the two crates allowed `unsafe` (ADR-0007).
- **Depends on:** nothing at run time. `cc` builds the shim, with `sdk` only. With `sdk`, it needs
  the eGrabber SDK and MSVC.
- **Used by:** `siq-euresys`, the safe `Camera` (P10.1).
- **Phase:** P2.5, the risk spike.

## API

The header, [`shim/siq_egrabber.h`](shim/siq_egrabber.h), documents each function; ADR-0004's
amendment summarizes them.

| Group | Functions |
|---|---|
| ABI, errors | `siq_eg_abi_version`, `siq_eg_last_error`; in Rust, `last_error()` and `status_name()` |
| System | `siq_eg_system_open(producer)`, `siq_eg_system_close` |
| Discovery | `siq_eg_discover` → `SiqEgCounts`, `siq_eg_discovery_info` → `key=value` lines |
| Grabber | `siq_eg_open(camera)`, `siq_eg_close` |
| Features | `siq_eg_get_*`, `siq_eg_set_*` (int, float, string), `siq_eg_execute`, in `SIQ_EG_INTERFACE`, `_DEVICE`, `_STREAM` or `_REMOTE` |
| Buffers | `siq_eg_announce_user_buffers(ptrs, size, count)`, `siq_eg_requeue(index)` |
| Acquisition | `siq_eg_start(frames)`, `siq_eg_stop`, `siq_eg_pop(timeout_ms)` → `SiqBufferInfo`, `siq_eg_cancel_pop` |
| Counters | `siq_eg_counters` → `SiqEgCounters` |

- **Statuses:** every function but the closers returns a `SiqEgStatus`: `SIQ_EG_OK`, `_TIMEOUT`,
  `_ABORTED`, `_TRUNCATED`, `_INVALID_ARGUMENT`, `_GENTL_ERROR` or `_ERROR`.
- **Exceptions:** none crosses the ABI.
- **Threads:** a grabber is used from one thread at a time, except `siq_eg_cancel_pop`.
- **Closing:** every grabber is closed before its system, and the announced memory outlives the
  grabber.
- **The bindings,** [`src/bindings.rs`](src/bindings.rs), are committed. `cargo xtask
  regen-bindings` regenerates them, with bindgen and libclang; nothing else needs libclang.

## Examples

| Example | What it shows | Run |
|---|---|---|
| `eg_discover` | The interfaces, grabbers and cameras a GenTL producer sees, with what each says about itself. With `--open`, it reads each camera's size and pixel format, changing nothing | `cargo run -p siq-euresys-sys --features sdk --example eg_discover [--open] [producer.cti]` |

`eg_discover` needs the `sdk` feature, so `cargo xtask ci` skips it.
- With no producer, it loads the default one.
- `%EURESYS_PLAYLINK_GENTL64_CTI%` loads Playlink, Euresys's virtual producer.
- On this PC, it finds a Coaxlink Quad CXP-12 and an Adimec D-103A12c (11264 × 9200,
  `BayerGB8`).

## Features

- **`sdk`** compiles the shim against the eGrabber SDK found at `EGRABBER_SDK_DIR` (default
  `C:\Program Files\Euresys\eGrabber`), with MSVC. If the SDK isn't there, the build stops with a
  message.
  - At run time, the SDK's header loads `egrabber.dll` from `EURESYS_EGRABBER_LIB64`, which the
    installer sets.
  - Without `sdk`, the types and constants build with no SDK, and the functions aren't linked.

## Testing

- **`cargo test -p siq-euresys-sys`** needs no SDK. It runs `layout`:
  - the structs' sizes and field offsets, as the shim's `static_assert`s fix them;
  - every constant and function of the header bound, with the same values and names.
- **`cargo test -p siq-euresys-sys --features sdk`** runs `sdk` too:
  - the shim's ABI version;
  - bad arguments refused with their message, kept per thread.
- **Ignored, run with `-- --ignored`:**
  - `discovery_returns_without_error`, with the default producer;
  - `playlink_fills_our_buffers_and_takes_them_back`: eight frames from Playlink into our own
    page-aligned buffers, requeued, and counted.

## Limitations

- **The bindings were written by hand in P2.5,** in bindgen's form, because libclang isn't
  installed. `tests/layout.rs` checks them against the header. The first run of `cargo xtask
  regen-bindings` should change only formatting.
- **No safe wrapper yet:** `siq-euresys` comes in P10.1.
- **A camera is opened only from a discovery.** Index-based opening would conflict with
  `EGrabberDiscovery`'s exclusive control of the device lists.
- **Playlink numbers no frames:** its `frame_id` is always 0.
