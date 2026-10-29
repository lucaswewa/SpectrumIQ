# ADR-0070: Euresys acquisition: threading, buffer ownership, the feature map, and raw Bayer frames

- **Status:** Accepted
- **Date:** 2026-10-29
- **Phase:** P10.1
- **Related:** ADR-0004, ADR-0005, ADR-0007, ADR-0022, ADR-0031, REQ-ACQ-001, REQ-ACQ-002,
  REQ-ACQ-003, REQ-ACQ-005

## Context

P2.5 put eGrabber behind a C ABI of our own (ADR-0004) and found the camera on this PC: an Adimec
D-103A12c on a Coaxlink Quad CXP-12. The frame pool was made for a grabber (P1.1): its slots can be
announced, and a release sink can requeue a slot when its last lease drops. P10.1 makes the camera
a `siq-devices` `Camera` that fills those slots by DMA.

Reading the camera's features found that they differ from GenICam SFNC's:

- it has no `TriggerMode`: it runs free in `ExposureMode` `Timed`, at `AcquisitionFramePeriod`, and
  is triggered in `TimedTriggerControl`;
- its `TriggerSource` is the grabber's CoaXPress trigger (`Trigger`) or `IO_Connector`, and its own
  `TriggerSoftware` makes no frame;
- its gain is a factor, not dB;
- its pixel formats are `BayerGB8`, `10` and `12`.

It is a colour camera. On 2026-10-29 the owner settled the plan's open question 1: it is the
instrument's camera, and its frames stay raw Bayer, demosaiced only for display.

## Decision

- **Raw Bayer frames:**
  - a frame is one sample a pixel, of the colour its filter passes;
  - `PixelFormat` stays the samples' depth: `BayerGB12`, which the grabber unpacks into the low bits
    of `u16` (`UnpackingMode` `Lsb`), is `Mono12`;
  - `siq_types::Cfa` names the 2 × 2 filter cell at an image's top left. `CameraCapabilities` gives
    the sensor's, and `FrameMeta` the frame's: the sensor's, shifted by the region's odd offsets.
    Old frames, and mono ones, have none.
  - Binned 2 × 2, this camera sums same-colour pixels and keeps the pattern: a binned frame keeps
    its `cfa` (measured, below).
- **Buffer ownership:**
  - `start` announces every pool slot, each at its own index, and is refused while a slot is held;
  - a slot is queued to the grabber only while the camera holds its `FillGuard`, so the grabber
    never writes a frame that is being read;
  - a slot goes back to the camera's thread when its last lease drops (the release sink sends it),
    or when it is dropped unpublished (the free list), and is requeued there.
- **One thread for the grabber:**
  - it requeues, pops with a 20 ms tick, and reads the counters;
  - the grabber is used from one thread at a time, as the shim requires, through a mutex;
  - a caller beside the thread (a software trigger, a change of exposure or gain) counts itself
    waiting, and the thread steps aside until it has had the grabber, within a tick. A plain mutex
    isn't fair: the thread retook it at once, and a trigger waited forever on the camera.
- **What a frame is:**
  - a frame incomplete, or of another size than the settings say, is lost (`lost_device`) and its
    slot requeued;
  - one the grabber had no slot for is an underrun (`lost_no_slot`);
  - its `device_ts` is the grabber's timestamp;
  - the middle of its exposure, on the host's clock, is estimated from when it was popped: the
    readout and half the exposure before. P10.2 maps the grabber's clock.
- **Teardown,** in order, also when the thread panics: the grabber stops, revokes the slots
  (announcing none), and only then do they go back to the pool. The grabber is closed before its
  GenTL system, and the pool's memory outlives the grabber's announcement of it.
- **A feature map per camera model,** in `config/cameras/<model>.toml`:
  - what differs from SFNC's names: the pixel formats by entry, the exposure (µs), the gain (dB or
    a factor), the free-running period, the features each trigger mode sets, the trigger's command
    and module;
  - what GenICam doesn't give: the pixel pitch, the colour filter array, the readout's model;
  - the model it is for, checked against `DeviceModelName`;
  - a setting names its module first if it isn't the camera's: `device:CameraControlMethod`.
- **The Adimec's triggers:**
  - free-running is `ExposureMode` `Timed` with the grabber's camera controller off (`NC`). The
    period is set to the exposure, or to the readout if that is longer;
  - a software trigger is the grabber's camera controller (`RC`, `CycleTriggerSource`
    `StartCycle`). It sends a CoaXPress trigger, and the camera exposes for its `ExposureTime`;
  - external is the camera's `IO_Connector`.
- **Setting the timing:** the camera shortens an exposure to fit its period, so the period is set
  before the exposure and again after it.
- **The service:** `siq-server`'s `euresys` feature (and `spectrumiq-server`'s) builds
  `[camera] kind = "euresys"`, with its pool locked in RAM (ADR-0005).

## Alternatives considered

- **`siq_eg_cancel_pop` to wake the thread for a caller:** a cross-thread FFI call for what a 20 ms
  tick already bounds.
- **A command channel to the thread:** every caller would wait on a reply, for the same bound.
- **Demosaicing in the grabber (`BayerMethod`) or as frames arrive:** three samples a pixel triple
  the pool, the recorder's bandwidth and every job's memory. The owner chose raw.
- **A `PixelFormat` for each Bayer pattern and depth:** twelve variants where two facts are
  independent, and a region's offset changes the pattern, not the depth.
- **SFNC's names only:** this camera's triggering and gain differ, and the next camera's may.

## Consequences

- **M10 adds colour phases:** the previews and the viewer demosaic for display, and registration,
  stitching and the live mosaic work on luminance. Until then, raw Bayer frames flow through as mono.
- **`unsafe`** is in `siq-euresys` too, as ADR-0007 allows.
- **The frame-rate model** (overhead and line time) is the map's. Binning doesn't shorten this
  camera's readout, which `FrameRateModel` assumes; the camera keeps its own minimum period. P10.2
  measures each mode.
- **A software trigger and the free run change the grabber's camera controller,** so whatever
  else uses the grabber sees it changed.

## Verification

- `crates/siq-euresys/tests/camera.rs`, over a fake grabber that writes each frame's number into
  the slot it fills:
  - capabilities and settings as the Adimec's map names them;
  - a map for another model refused;
  - the region in binned pixels, the triggers, ten triggers while the thread pops;
  - every slot requeued only once its last lease drops, an underrun, the slots back after a stop;
  - an incomplete buffer lost and filled again.
- On the camera, `tests/hardware.rs`:
  - 100 full frames in sequence, at 12.28 frames/s, a frame every 81.43 ms ± 0.002 ms, none lost;
  - one frame for each of 5 software triggers;
  - a 4096 × 2048 region at 51 frames/s, binned 2 × 2 at its size. Its cell's phases measured
    192, 130, 137 and 192 unbinned, and 770, 522, 549 and 770 binned: green, blue, red, green, four
    times brighter.
- `eg_live`: 122 full frames in 10 s at 12.28 frames/s, jitter 0.001 ms rms, none lost.
- **The service on the camera,** with the simulated stage and focus:
  - live view at 12.28 frames/s, 185 frames and none lost;
  - both previews streaming, 150 frames each in 15 s;
  - 149 ms from mid-exposure to the overview's JPEG on average, 21 ms of the overview's work and
    16 ms of the ROI's;
  - a snap through a software trigger.
- After every run, the camera's features were as first found (`eg_features --save`).
