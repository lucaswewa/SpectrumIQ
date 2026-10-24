# M2 — Acquisition, live preview and the first server

**Outcome:** `spectrumiq-server -c config/sim.toml` serves the simulated rig over teta-wot:

- live MJPEG overview and ROI streams;
- stage and focus control;
- peripherals and channels;
- metrics.

The Euresys shim also compiles against the installed SDK. This is an early risk spike: the full
driver comes in M10.

**Exit criteria:**

- a Python script moves the stage and receives preview frames whose content changes with the move;
- no frames are lost over 10 minutes of simulated live view at full rate, with both previews
  running;
- `cargo build -p siq-euresys-sys --features sdk` succeeds, and the `eg_discover` example runs.

---

### P2.1 — Acquisition engine (`siq-acquire`)

**Budget** ~800 · **Depends on** P1.2, P1.3, P1.7 · **Requirements** REQ-ACQ-004, REQ-ACQ-005, REQ-ACQ-006, REQ-STG-003

**Goal.** Own the camera, pool and fan-out. Give every frame its pose and context.

**Scope**

- **`Acquisition::new(camera, pool_cfg, clock)`**, with these modes:
  - `Live`;
  - `Snap` (software trigger, settle-aware);
  - `Sequence { n }`, with a per-frame hook.

  Changing ROI or binning runs stop → reconfigure → start.
- **`PoseTimeline`:** a ring of stage and focus `AxisStatus` samples. `pose_at(t)` returns
  `(StagePose, PoseQuality)`: settled if every axis is idle and the sample is newer than `t`,
  interpolated otherwise, with uncertainty = velocity × sample spacing.
- **`AcquisitionContext`** (channel, objective, calibration id), set by the UI and workflows, is
  stamped into `FrameMeta`. `capture_next(n, save: bool)` marks frames for the recorder.
- **Stats:** measured fps, lost frames (camera side and pool exhaustion), and lag per consumer.

**Tests**

- With SimCamera and the GRBL driver↔emulator: the pose on stationary frames equals the true pose
  within the injected error.
- Interpolation (proptest).
- A mode switch keeps the sequence numbering consistent.
- Lost-frame accounting.

**Example.** `acquire_live`: prints fps, lost frames and the pose while the stage moves.

**ADR.** Frame↔pose time association, and context stamping.

---

### P2.2 — Preview pipelines (`siq-preview`)

**Budget** ~700 · **Depends on** P1.5, P2.1 · **Requirements** REQ-PRV-001, REQ-PRV-002, REQ-PRV-003, REQ-PRV-004

**Goal.** Turn frames into compressed previews and statistics, within the frame budget of
architecture §4.2.

**Scope**

- **`LatestOnly` consumers** on the frame-path rayon pool:
  - `Overview`: ≤ 1920×1080, aspect preserved;
  - `Roi`: a sensor-pixel region, zoom 1:1 or 2:1, clamped and alignment-aware;
  - `Stats`: histogram and focus measure on the ROI.
- **`DisplaySettings`** (watch channel): `Manual { min, max, gamma }` or `Auto { low_pct,
  high_pct }`, plus a colormap.
- **Encoding:** JPEG with `jpeg-encoder` at configurable quality.
- **Sinks:** `PreviewSink::push_jpeg(stream, bytes, meta)` and `StatsSink`. The teta-wot adapters
  come in P2.3.
- **Rate cap** per stream.
- **Latency measurement:** frame `host_ts` → JPEG ready, exported as a histogram metric.

**Tests**

- Output geometry.
- LUT application.
- Auto-contrast percentiles.
- The rate cap.
- A latency measurement path with SimCamera (target ≤ 60 ms at 100 MP on the target PC; recorded,
  not asserted in CI).

**Example.** `preview_to_files`: writes overview and ROI JPEGs for 3 s of simulated live view.

**ADR.** Preview stream design: two streams, server-side LUT, latest-only, and rate caps.

---

### P2.3 — Service core: camera, display, stage and focus Things (`siq-server`, `apps/spectrumiq-server`)

**Budget** ~900 · **Depends on** P0.3b, P1.9, P1.10b, P2.2 · **Requirements** REQ-NET-001, REQ-NET-002, REQ-SIM-006, REQ-PRV-001

**Goal.** The first runnable service, built on teta-wot v0.1.0.

**Scope**

- **Device factory** from `InstrumentConfig`, choosing sim or real per device:
  - SimCamera;
  - GRBL over `DuplexTransport` to the emulator, or over a COM port;
  - the ASCII focus drive over the emulator or a COM port.
- **Engines:** acquisition and preview.
- **`ThingServer::builder()`** with `.api_prefix("/api/v1")`, `.global_lock(true)`, and
  `.service(...)` for shared engines. `ThingServer::router()` is merged with SpectrumIQ's router,
  which serves `/api/siq/system/health`.
- **Things:**
  - `camera`: properties per architecture §6.1; `start_live`, `stop_live`, `snap`; `MjpegStream`
    `overview` and `roi`, fed by the `PreviewSink` adapter;
  - `display`;
  - `stage`: `stop` with `global_lock = false`;
  - `focus`.
- **CLI (clap):**
  - `run -c <config>` (the default);
  - `--print-openapi`, as in `microscope`, for generated client types;
  - `--print-config-schema`.
- **Graceful shutdown:** stop live, then stop motion.

**Tests**

- teta-wot `TestClient` integration:
  - TDs are listed;
  - `move_to` via HTTP while observing `position`;
  - `start_live`, then read two MJPEG frames;
  - a `GlobalLockBusy` path.
- Insta snapshots of the OpenAPI document and the TD fragments.

**Examples.**

- `cargo run -p spectrumiq-server -- -c config/sim.toml`.
- `examples/python/live_and_move.py` (httpx): moves the stage and saves two preview frames.

**ADR.**

- Thing decomposition and naming.
- Sim/real device selection.
- An amendment to ADR-0003 with the concrete route layout and the merge order.

---

### P2.4 — Peripherals, channels and system routes

**Budget** ~600 · **Depends on** P2.3 · **Requirements** REQ-DEV-001, REQ-SIM-003, REQ-NFR-002

**Goal.** Expose the peripherals and the channel model. Make the service observable.

**Scope**

- **Things:** `illumination`, `filter_wheel`, `objective`, and `modality`, backed by the simulators.
- **`imaging` Thing:** `ChannelConfig` (illumination source and intensity, filter position,
  exposure, display colour) from the config. `select_channel` applies a channel atomically, in
  order and with settle delays, and updates `AcquisitionContext`.
- **Routes:** `/api/siq/system/metrics` (telemetry snapshot) and `/api/siq/system/memory` (pool,
  caches, process working set).

**Tests**

- Channel switching order and timing with paused time.
- Metrics JSON snapshot.
- The HTTP integration of each new Thing.

**Example.** `examples/python/channels.py`: cycles through the channels and saves a snap of each.

**ADR.** The channel model, and how it maps onto devices.

---

### P2.5 — Euresys shim and bindings: risk spike (`siq-euresys-sys`)

**Budget** ~700 (C++ ~400, Rust ~300) · **Depends on** P0.1 · **Requirements** REQ-ACQ-001, REQ-NFR-005

**Goal.** Prove the toolchain early: a C ABI over the installed eGrabber C++ API, and bindgen
bindings that contain only our own declarations (ADR-0004).

**Scope**

- **`shim/siq_egrabber.h`:** plain C types and opaque handles only. Functions:
  - `siq_eg_discover` (counts and info strings);
  - `siq_eg_open` and `siq_eg_close`;
  - get/set feature (int64, double, string) per module (interface, device, stream, remote);
  - `siq_eg_execute`;
  - `siq_eg_announce_user_buffers(ptrs, size, count)`;
  - `siq_eg_start(n)` and `siq_eg_stop`;
  - `siq_eg_pop(timeout_ms, *SiqBufferInfo {index, ptr, size, timestamp, frame_id, width, height,
    pixfmt, flags})`;
  - `siq_eg_requeue(index)`;
  - `siq_eg_counters`;
  - `siq_eg_last_error(buf, len)`.
- **`shim/siq_egrabber.cpp`:**
  - uses `EGrabber<CallbackOnDemand>`, `UserMemory`, `pop`, and `push`;
  - catches every exception and maps it to an error code and message;
  - never lets an exception unwind across the ABI.
- **`build.rs`, with feature `sdk` (off by default):**
  - finds the SDK through `EGRABBER_SDK_DIR` (default `C:\Program Files\Euresys\eGrabber`) and
    checks that `include/EGrabber.h` exists;
  - compiles with `cc` (MSVC, `/std:c++17 /EHsc`).

  At run time, the eGrabber header loads `egrabber.dll` through `EURESYS_EGRABBER_LIB64`, which
  the SDK installer sets.
- **Bindings:**
  - `src/bindings.rs` is generated by bindgen from **our** header and committed;
  - `cargo xtask regen-bindings` regenerates it (needs LLVM/libclang, which is not installed
    today);
  - CI and dev builds need no libclang.
- **No SDK files in the repo.**

**Tests**

- Without the SDK: the crate builds with the feature off, and the layout tests of the bindings
  pass.
- With the SDK, `#[ignore = "hardware"]`: discovery returns without error.
- Optional: a `playlink.cti` smoke test, if Playlink can provide a virtual device. Record the
  findings.

**Example.** `eg_discover`: lists interfaces, devices and cameras.

**ADR.** An amendment to ADR-0004 with the concrete shim API, the build and runtime discovery, and
the Playlink findings.
