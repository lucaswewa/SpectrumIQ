# M1 — Frame memory, devices and simulation

**Outcome:**

- the zero-allocation frame pool and fan-out;
- device traits;
- the synthetic specimen worlds;
- a simulated 100 MP camera;
- protocol-level emulators for the GRBL stage and the focus drive, driven by the **real** drivers.

**Exit criteria:**

- a simulated rig (SimCamera + GRBL driver↔emulator + focus driver↔emulator) runs from
  `config/sim.toml` in an example;
- frames reflect the true stage pose and focus;
- fan-out sustains 9 fps × 200 MB with zero steady-state allocations.

---

### P1.1 — Frame pool (`siq-buffer::pool`)

**Budget** ~800 · **Depends on** P0.2 · **Requirements** REQ-ACQ-002, REQ-NFR-002, REQ-NFR-003

**Goal.** The pre-allocated, page-locked buffer pool that the grabber DMAs into (ADR-0005).

**Scope**

- **Backends:**
  - `WindowsLocked`: `VirtualAlloc` (page-aligned) per slot; `SetProcessWorkingSetSize` raised to
    cover the pool plus a margin; `VirtualLock`; a clear error if locking fails.
  - `Heap`: aligned allocation for tests and Miri.
- **`FramePool::new(slots, slot_bytes, backend)`.** The slot state machine is
  `Free → Filling → Ready → Leased(n) → Free`, with atomic transitions.
- **Producer API:**
  - `try_acquire_free() -> Option<FillGuard>`, for the simulator;
  - `slot_ptrs()`, for announcing slots to the grabber;
  - `publish(slot, FrameMeta) -> FrameLease`.
- **`FrameLease`:**
  - cheap clone;
  - `as_bytes()` and `image::<u16>()`;
  - the last drop calls `ReleaseSink::release(slot)`, which requeues the slot to the grabber or
    frees it for the simulator.
- **Metrics:** free slots, high-water mark, lease durations.
- **Unsafe** stays inside `alloc.rs` and `slot.rs`, with `SAFETY` reasoning: Filling is exclusive to
  the producer; Ready and Leased are shared read-only.

**Interfaces.** `FramePool`, `FrameLease`, `ReleaseSink`, `FillGuard`. P10.1 (Euresys) and P1.7
(SimCamera) depend on them.

**Tests**

- State machine (proptest over random operation sequences).
- Release callbacks.
- Lease clone and drop counting.
- **No allocation after warm-up**, checked with a counting global allocator in an integration test.
- Optional `cargo +nightly miri test -p siq-buffer`, with the Heap backend.

**Example.** `pool_locked`: allocates and locks 12 × 200 MB (smoke: 4 × 8 MB), then reports the
working set.

**ADR.** Frame pool design:

- pool slots are the announced buffers, rather than a byte ring;
- lease semantics;
- locking strategy;
- large pages as a deferred option.

---

### P1.2 — Frame fan-out (`siq-buffer::fanout`)

**Budget** ~600 · **Depends on** P1.1 · **Requirements** REQ-ACQ-005, REQ-NFR-003

**Goal.** Distribute each lease to many consumers without blocking the producer.

**Scope**

- **`Fanout::subscribe(name, Policy, filter)`** returns a `Subscriber` with async `recv()` and a
  blocking `recv_blocking()`.
- **Policies:**
  - `LatestOnly`: a single-slot mailbox; a replaced lease drops;
  - `Bounded { n, drop: Oldest | Newest }`;
  - `Lossless`: keeps every lease; pool exhaustion then appears as lost frames at the producer.
- **Filter:** a predicate on `FrameMeta`. For example, the recorder takes only `flags.save`.
- **`publish(lease)`** never blocks. Per-subscriber stats: received, dropped, max lag.

**Tests**

- Policy semantics.
- Filter.
- Unsubscribe while frames are in flight.
- A soak run of 9 fps × 200 MB (smoke: 2 s at small sizes) with two slow consumers: no allocation,
  and drops counted correctly.

**Example.** `fanout_soak`: throughput and per-consumer drops.

**ADR.** Consumer policies and backpressure.

---

### P1.3 — Device traits (`siq-devices`)

**Budget** ~700 · **Depends on** P1.1 · **Requirements** REQ-DEV-001, REQ-STG-003

**Goal.** Driver-independent async traits for every device. Engines depend only on these.

**Scope**

- **Dyn-compatible traits** with `BoxFuture` methods: `Camera`, `Stage` (XYZ coarse),
  `FocusDrive`, `Illumination`, `FilterWheel`, `ObjectiveTurret`, `ModalityUnit` (widefield,
  light-field MLA, confocal disk, SIM pattern and phase).
- **`Camera`:**
  - `capabilities()`: sensor, pixel formats, binning, ROI alignment, exposure range, fps model;
  - `configure(CameraSettings)`;
  - `start(pool, ReleaseSink)`, `stop()`, `trigger()`;
  - `snap()` for software-triggered single frames;
  - `stats()`.
- **`Stage` and `FocusDrive`:**
  - `move_to` and `move_by`, with `MoveOpts { speed, approach }`;
  - `jog(velocity)`, `stop()`, `home()`;
  - `status() -> watch::Receiver<AxisStatus { pose, state, ts }>`;
  - `limits()`.
- **Common types:** `DeviceInfo`, `DeviceError` (Timeout, Disconnected, Fault, InvalidArgument,
  Busy, Unsupported, Alarm(code)).
- **`mock` module:** an immediate-completion mock of each trait, for engine tests.

**Tests**

- Object safety (compile).
- Mock behaviour.
- Status watch semantics.

**Example.** `mock_rig`: moves a mock stage, snaps from a mock camera, and prints the status
stream.

**ADR.** Device abstraction: async traits via `BoxFuture`, capability descriptors, and status
watches instead of polling.

---

### P1.4 — Synthetic specimen worlds (`siq-synth`)

**Budget** ~900 · **Depends on** P0.2 · **Requirements** REQ-SIM-004, REQ-SIM-005

**Goal.** Deterministic virtual specimens with exact ground truth, for the camera simulator and
dataset generation.

**Scope**

- **`World` trait:**
  - `sample(region: RectUm, scale_um_per_px, out: &mut ImageViewMut<f32>)`, giving in-focus
    intensity in 0..1;
  - `surface_z(x, y) -> Option<Micrometers>`, for 2.5D worlds;
  - `density(x, y, z)`, for 3D worlds.
- **Implementations:**
  - `Cells`: seeded multi-scale value noise plus elliptical cell SDFs with nuclei and texture;
  - `Grains`: Voronoi grains for reflected-light materials;
  - `HeightField`: a relief with a known z, for DFF;
  - `Beads`: a 3D Gaussian bead volume, for deconvolution and light field;
  - `ImageWorld`: a large 16-bit PNG or TIFF placed at a given pixel size, so real images can serve
    as the world.
- **Seam consistency:** sampling adjacent regions independently gives identical values at shared
  coordinates.
- **Ground-truth export:** height maps, bead lists and world metadata, as JSON plus raw f32.

**Tests**

- Determinism by seed.
- Seam consistency (proptest).
- Statistics within expected ranges.
- `ImageWorld` scaling.

**Example.** `render_world`: renders a region of each world to PNG.

**ADR.** Synthetic world model and ground-truth conventions.

---

### P1.5 — CPU image kernels (`siq-imgproc`)

**Budget** ~850 · **Depends on** P0.2 · **Requirements** REQ-PRV-003, REQ-CAL-002, REQ-NFR-003

**Goal.** Fast, allocation-free CPU kernels used by the simulator, the previews, calibration and
storage.

**Scope**

- **Correction:**
  - dark and flat correction: u16 in → u16 out, with an f32 gain map, in place or into an output
    buffer.
- **Resampling:**
  - area-average downsample to an arbitrary target size;
  - 2ⁿ binning (sum or mean);
  - crop/copy;
  - separable Gaussian blur with a σ-dependent kernel;
  - bilinear upsample.
- **Statistics and display:**
  - a 14-bit histogram (4096 bins) on a strided subsample;
  - percentiles and auto-contrast;
  - a LUT from u16 to u8 (window, level, gamma; a precomputed 64 Ki table);
  - colormaps (grey, inverted, and channel colours).
- **Focus measures** on a ROI: variance of Laplacian, Tenengrad, normalized variance.
- **Parallelism and buffers:** rayon over row bands. Every kernel has a caller-provided-buffer
  variant.

**Tests**

- Every kernel against a naive reference (proptest on small random images).
- Edge cases: odd sizes, 1-pixel images, and saturation.

**Example.** `kernels_bench`: timings at 100 MP (smoke: 1 MP). criterion benches go in `benches/`.

**ADR.** CPU kernel strategy: rayon row bands, autovectorisation first, explicit SIMD only on
measured need, and caller-owned buffers.

---

### P1.6 — Image formation model (`siq-sim::optics`)

**Budget** ~800 · **Depends on** P1.4, P1.5 · **Requirements** REQ-SIM-001

**Goal.** A pure function that renders a realistic 14-bit frame from a world, a true pose, and the
optics and sensor models.

**Scope**

- `form_image(world, pose, affine, OpticsModel, SensorModel, exposure, roi, binning, rng, out:
  &mut [u16])`.
- **Defocus:** Gaussian σ = f(|z − surface|, NA, DOF). Worlds are sampled at a reduced resolution
  when σ is large, then upsampled.
- **Sensor:**
  - vignetting (cos⁴ or polynomial);
  - dark offset;
  - fixed-pattern noise (seeded per sensor);
  - shot noise (Poisson, with a Gaussian approximation above about 20 e⁻);
  - read noise;
  - gain in e⁻/ADU;
  - 14-bit clip in Mono16.
- **Performance strategy.** A noiseless render is cached for the last pose. When the pose is
  unchanged, only noise is regenerated (about 20 ms at 100 MP, parallel).
  - Targets: stationary live at 9 fps for 100 MP; moving ≥ 3 fps for 100 MP; ≥ 9 fps for the
    `fast` profile at 25 MP.

**Tests**

- Sharpness decreases monotonically with |dz|.
- Vignetting profile.
- Noise mean and variance against Poisson.
- ROI and binning equal crop and sum.
- 14-bit clipping.
- Determinism by seed.

**Example.** `render_frame`: writes a raw frame plus a JPEG preview for a given pose.

**ADR.** Simulated-camera fidelity versus performance, and the noiseless cache.

---

### P1.7 — Simulated camera device (`siq-sim::camera`)

**Budget** ~600 · **Depends on** P1.2, P1.3, P1.6 · **Requirements** REQ-SIM-001, REQ-ACQ-003, REQ-ACQ-006

**Goal.** `SimCamera` implements `Camera`, filling pool slots with rendered frames on a realistic
schedule.

**Scope**

- **`TrueState`** (`siq-sim::state`): the true stage pose (with injected errors), focus,
  illumination, filter, objective and modality. Shared, lock-free reads.
- **Render thread:**
  - an fps model per ROI and binning (9 fps full frame, faster for ROI and binning);
  - the `Clock` drives timing;
  - free-run and software-trigger modes;
  - when no slot is free, the frame is counted as lost.
- **`FrameMeta`:** device timestamp (simulated clock), mid-exposure host timestamp, sequence.

**Tests**

- fps accuracy with `ManualClock`.
- Trigger latency.
- Lost frames when consumers hold leases.
- Settings change takes effect at the next frame.

**Example.** `sim_live`: runs 5 s (smoke: 1 s) and prints fps, lost frames, and mean intensity per
pose.

**ADR.** An amendment to the P1.6 ADR covering timing, triggering, and `TrueState`.

---

### P1.8 — Serial transport and GRBL emulator (`siq-serial`, `siq-sim::grbl`)

**Budget** ~850 · **Depends on** P1.7 (TrueState) · **Requirements** REQ-SIM-002, REQ-STG-001

**Goal.** A line-oriented async transport, and a GRBL 1.1 emulator that the real driver talks to.

**Scope**

- **`siq-serial`:**
  - `LineTransport` trait: `write_line`, `write_realtime(u8)`, `read_line(timeout)`;
  - `SerialTransport` (tokio-serial, with configurable framing) and `DuplexTransport`
    (`tokio::io::duplex`);
  - TX/RX tracing at debug level.
- **GRBL emulator, protocol subset:**
  - motion and modes: `G0`, `G1`, `G4`, `G20/G21`, `G90/G91`;
  - system commands: `$H`, `$X`, `$J=…`, `$$` (configurable settings);
  - realtime: `?`, `!`, `~`, `0x18`, `0x85`;
  - replies `ok`, `error:N`, `ALARM:N`, and the startup banner;
  - status reports of the form `<Idle|MPos:…|FS:…>`.
- **Motion model:** trapezoidal moves per axis from `$110–$122`, soft limits, homing.
- **Injected errors:** backlash per axis, scale error, random positioning error, optional missed
  steps, and reply latency.
- **TrueState:** the emulator updates the true pose on clock ticks.

**Tests**

- Scripted protocol conformance (golden transcripts).
- Motion timing with `ManualClock`.
- Alarm on a soft-limit violation.
- Feed hold and resume.
- Soft reset.

**Example.** `grbl_emulator_repl`: drives the emulator with scripted lines and prints the replies.

**ADR.** Protocol-level emulation, and the serial transport abstraction.

---

### P1.9 — GRBL stage driver (`siq-grbl`)

**Budget** ~800 · **Depends on** P1.3, P1.8 · **Requirements** REQ-STG-001, REQ-STG-003, REQ-STG-004, REQ-STG-005

**Goal.** A robust `Stage` implementation for GRBL and grblHAL.

**Scope**

- **Connect:** soft reset, wait for the banner (`Grbl 1.1` or `GrblHAL`), handle alarms (configured
  `$X`), read `$$` (steps/mm, max rates, travel → limits), then `G21 G90`.
- **Command channel:** send-and-wait-for-`ok`, which suits stop-and-go. Character-counting
  streaming is a documented follow-up.
- **Status polling task:** `?` at a configurable rate (default 5 Hz; higher on grblHAL if verified),
  feeding the `AxisStatus` watch. The timestamp is the receive time minus half the measured
  round-trip.
- **`move_to`:** `G1 … F…`, then `G4 P0` (`ok` arrives after the motion completes), then a confirmed
  `Idle` status, with a timeout derived from distance and speed.
- **Backlash-consistent approach:** an overshoot along the configured direction.
- **Jog:** `$J=` and jog-cancel.
- **`stop`:** feed hold, `Hold` state, then soft reset and re-sync. A reset during motion loses
  position; the driver reports that it needs homing.
- **Units:** µm↔mm conversion at the boundary.

**Tests**

- Parser proptests (status and settings).
- Against the emulator: move, jog, home, stop mid-move, alarm recovery, timeouts, backlash
  approach.

**Example.** `grbl_drive`: scripted moves against the emulator. `--port COM5` drives a real
controller; that path is manual.

**ADR.** GRBL motion completion, polling, stop and recovery strategy, including the caveat that
the stage is open-loop.

---

### P1.10 — ASCII dialect driver and peripheral simulators (`siq-ascii`, `siq-sim::peripherals`)

**Budget** ~850 · **Depends on** P1.3, P1.8 · **Requirements** REQ-STG-002, REQ-DEV-001, REQ-SIM-003

**Goal.** Support the focus drive (and later ASCII peripherals) from a data-driven dialect file, and
simulate every peripheral.

**Scope**

- **Dialect TOML:**
  - terminators;
  - init sequence;
  - command templates (`move_abs = "MA {pos:steps}\r"`) with unit conversions;
  - reply regexes with named groups;
  - error patterns;
  - timeouts;
  - motion-done detection (poll query or completion reply).
- **`AsciiFocusDrive<T: LineTransport>`** implements `FocusDrive`.
- **`DialectEmulator`** generates the device side from the same dialect, adds a motion model, and
  updates `TrueState.focus`.
- **Peripheral simulators** (trait-level, with realistic delays): `SimIllumination`,
  `SimFilterWheel`, `SimObjectiveTurret`, `SimModalityUnit`.
- **Example dialect:** `config/dialects/example-focus.toml`. The real device's dialect follows in
  P10.3.

**Tests**

- Dialect validation errors.
- Template rendering (proptest units).
- Driver↔emulator round trips.
- Timeouts.
- Peripheral simulators.

**Example.** `focus_dialect_demo`.

**ADR.** Data-driven ASCII dialects.
