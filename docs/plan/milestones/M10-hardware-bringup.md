# M10 — Hardware bring-up: Euresys camera, GRBL stage, focus drive

**Outcome:** the real instrument runs everything the simulator ran. Only the configuration
changes.

**Exit criteria:**

- full-frame live at the camera's native rate for 10 minutes, with both previews, the live mosaic
  and the recorder active, and 0 lost frames;
- ROI and binning modes reach their rated fps;
- a real 3×3 tile scan stitches with residuals comparable to the simulator's;
- the bring-up runbooks are written.

**A driver phase is Done only after the owner has run its "How to try it" on the instrument.** CI
can't reach hardware.

---

### P10.1 — Euresys safe wrapper and `Camera` implementation (`siq-euresys`)

**Budget** ~900 · **Depends on** P2.5, P1.1, P1.3 · **Requirements** REQ-ACQ-001, REQ-ACQ-002, REQ-ACQ-003, REQ-ACQ-005

**Goal.** Grab 100 MP frames from the Coaxlink straight into the frame pool.

**Scope**

- **`Grabber`:** RAII over the shim; open by index or serial; typed feature access per module.
- **Camera setup** with GenICam SFNC names, kept in a per-camera feature map file because the
  actual names must be verified on the device:
  - `PixelFormat = Mono16`;
  - `Width`, `Height`, `OffsetX`, `OffsetY`;
  - `BinningHorizontal`, `BinningVertical`;
  - `ExposureTime`;
  - `TriggerMode`, `TriggerSource` (Software, Line);
  - `AcquisitionMode`.
- **Buffers:**
  - the slot size comes from the stream's payload size;
  - pool slots are announced as user memory;
  - the release path requeues by buffer index, thread-safely.
- **Acquisition thread:**
  - `pop(timeout)`;
  - `FrameMeta` built from the frame id and timestamp;
  - the lease is published to the fan-out;
  - lost and rejected frame counters come from the eGrabber events.
- **Ordered teardown:** stop → flush → revoke → close, also on panic paths.
- **`Camera` trait**, including `capabilities()` queried from the device.
- **Feature `sdk`,** off by default.

**Tests**

- Unit tests for the feature map and error mapping.
- Hardware tests, `#[ignore = "hardware"]`, enabled by `SIQ_EURESYS_DEVICE`: open, configure,
  grab 100 frames, check the sequence and timestamps.
- A Playlink smoke test, if P2.5 found it usable.

**Example.** `eg_live`: grabs for N seconds and prints fps, lost frames, and timestamp jitter.

**ADR.** Acquisition threading and buffer ownership with eGrabber.

---

### P10.2 — Camera bring-up and performance tuning

**Budget** ~500 · **Depends on** P10.1 · **Requirements** REQ-NFR-003, REQ-NFR-004, REQ-ACQ-003

**Goal.** Meet the frame budget on the real workstation.

**Scope**

- **Thread priority and affinity** for the acquisition thread (`windows-sys`).
- **Working-set and lock sizing checks** at startup.
- **Measurements:**
  - fps per mode (full, ROI presets, binning);
  - DMA throughput;
  - 10-minute lost-frame test under full consumer load;
  - exposure-end → preview latency;
  - device versus host timestamp semantics, validated and mapped.
- **A camera feature-map file** for this camera, in `config/cameras/`.
- **Runbook:** `docs/hardware/camera-bringup.md`.

**Tests.** Hardware tests for each measurement, with thresholds from the requirements.

**Example.** `hw_acquire_bench`.

**ADR.** An amendment to the P10.1 ADR with the measurements, the chosen settings, and the
timestamp mapping.

---

### P10.3 — Real stage and focus drive bring-up

**Budget** ~500 · **Depends on** P1.9, P1.10b, P5.1 · **Requirements** REQ-STG-001–005

**Goal.** Accurate, safe motion on the real controllers.

**Scope**

- **COM port discovery** (`serialport::available_ports`) in the config and Settings UI.
- **GRBL checks:**
  - `$$` against the expected values (steps/mm, rates, acceleration, soft limits, homing);
  - the homing procedure;
  - the measured status round-trip latency.
- **Focus drive:** write `config/dialects/<device>.toml` from the vendor protocol document. **The
  owner must provide that document.**
- **Repeatability and backlash routine:** uses registration on a textured target. Results go into
  the calibration registry.
- **Runbook:** `docs/hardware/motion-bringup.md`.

**Tests.** Hardware tests: move accuracy, repeatability, backlash, and stop behaviour.

**Example.** `motion_repeatability`.

**ADR.** An amendment to the GRBL and dialect ADRs with the real-device findings.

---

### P10.4 — (Optional) Hardware trigger and strobe

**Budget** ~600 · **Depends on** P10.2 · **Requirements** REQ-ACQ-006

**Goal.** Deterministic exposure timing, and groundwork for continuous scanning later.

**Scope**

- **Coaxlink I/O toolbox / camera trigger configuration:** software trigger with a hardware
  strobe output to the illumination; trigger-to-exposure latency measured.
- **A design-only ADR** for continuous position-triggered scanning: GRBL limitations,
  encoder/trigger hardware options, and timing.

**Tests.** Hardware timing tests.

**Example.** `strobe_timing`.

**ADR.** Trigger and strobe configuration, plus the continuous-scanning design.

---

### P10.5 — Peripheral drivers (template, one phase per device when models are chosen)

**Budget** ≤ 900 each · **Depends on** P1.10b · **Requirements** REQ-DEV-001

**Goal.** Replace a peripheral simulator with its real driver.

**Scope**

- A dialect file, if the device speaks ASCII; otherwise a custom driver behind the same trait.
- An emulator at the protocol boundary, for CI.
- A bring-up runbook.
- Config examples.

**Tests.** Driver↔emulator tests, and hardware tests.

**Example.** `<device>_demo`.

**ADR.** One ADR per device: protocol notes in our own words, and the safety constraints.

---

## Amendment (P10.1, 2026-10-29): the owner's answers, and the colour phases

Asked when M10 began, the owner answered:

- **The camera:** the Adimec D-103A12c/CXP-T01 on this PC is the instrument's camera. It is colour
  (Bayer GB), 11264 × 9200, 12 bits, and runs at 12.28 frames/s full frame. Its frames stay raw
  Bayer, 12 bits in a 16-bit container, as they are recorded and stitched. They are demosaiced only
  for display, and the processing works on the raw samples or on a luminance plane. That answers
  the plan's open question 1, and changes REQ-ACQ-001.
- **Hardware tests:** the agent may run them on the Coaxlink and the camera, putting back every
  setting it changes. The owner still runs each driver phase's "How to try it".
- **The stage and the focus drive:** they stay simulated for now. P10.3 is built against the GRBL
  and focus emulators: COM port discovery, the `$$` checks, homing, the status round trip, and the
  repeatability and backlash routine. The focus drive's dialect waits for its protocol document.
  The exit criterion of a real 3 × 3 tile scan waits for the real stage.
- **Scope:** P10.1 to P10.4. P10.5 waits for the peripherals' models.

**Two colour phases** are added, since raw Bayer frames meet every consumer:

### P10.6 — Colour I: a Bayer sensor in the simulator, and colour for display

**Budget** ~900 · **Depends on** P10.1 · **Requirements** REQ-ACQ-001, REQ-PRV-001, REQ-VIS-005

- The simulator's sensor gains a colour filter array (`[camera] cfa` for `kind = "sim"`), so that
  colour is tested in CI.
- The previews demosaic: the overview from whole filter cells, the ROI by interpolation at 1:1,
  with white balance, and the display mapping on each colour.
- The viewer's tiles demosaic, from the raw levels: the pyramid keeps the cells whole.
- The statistics and the histogram on luminance, or per colour.

### P10.7 — Colour II: luminance for registration, focus and stitching

**Budget** ~900 · **Depends on** P10.6 · **Requirements** REQ-CAL-001, REQ-STI-001, REQ-STI-002

- Registration, autofocus, the stage–camera calibration and the shading on luminance, or on one
  colour plane, never across the pattern.
- The live mosaic paints luminance, from whole cells.
- The stitch's registration on luminance, and its composite in colour: tiles placed at sub-pixel
  offsets can't keep a Bayer pattern.
- Datasets keep the raw frames, with their filter array in the metadata.

**The order of M10's phases:** P10.1, P10.2, P10.6, P10.7, P10.3, P10.4.
