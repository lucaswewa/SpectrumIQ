# Architecture decision records

ADRs follow [0000-template.md](0000-template.md).

- **Numbering:** take the next free number when writing one.
- **Phase briefs** name ADR topics, not numbers.
- **Amendments** are appended to the ADR they refine.
- **ADRs 0001–0016** record the decisions made during planning on 2026-10-23.

| # | Title | Status | Phase |
|---|---|---|---|
| [0001](0001-record-decisions-and-traceability.md) | Record decisions and keep traceability (IVD-ready) | Accepted | planning |
| [0002](0002-core-as-separate-service.md) | The core runs as a separate service; desktop and browsers are clients | Accepted | planning |
| [0003](0003-teta-wot-as-service-framework.md) | teta-wot v0.1.0 as the service framework, extended inside SpectrumIQ (amended in P2.3: the route layout and merge order) | Accepted | planning |
| [0004](0004-euresys-via-cpp-shim-and-bindgen.md) | Euresys eGrabber through a C++ shim and bindgen (amended in P2.5: the shim's API, its build, and Playlink) | Accepted | planning |
| [0005](0005-memory-model-and-out-of-core.md) | Memory model: pinned frame pool, streaming and out-of-core processing | Accepted | planning |
| [0006](0006-data-formats.md) | Data formats: OME-Zarr master, raw scratch, OME-TIFF and DZI derived (amended in P0.2: exact float round trips) | Accepted | planning |
| [0007](0007-pure-rust-wgpu-and-unsafe-policy.md) | Pure Rust + wgpu for algorithms; unsafe and native-code allow-list (amended in P1.1: test-only global allocators) | Accepted | planning |
| [0008](0008-frontend-stack-and-microscope-reuse.md) | Frontend: Vue + PrimeVue (unstyled) skinned with `microscope`'s tokens, copied from `microscope@0640f99` (amended in P3.1: Reka UI stays, since PrimeVue 5 isn't MIT; the files imported) | Accepted | planning |
| [0009](0009-visualization-stack.md) | Visualization: OpenSeadragon first, WebGL2 16-bit later; VTK.js; optional native viewer | Accepted | planning |
| [0010](0010-streaming-tiers.md) | Streaming tiers, browser connection budget, and staged local fast paths (amended in P3.2: as built, ADR-0037) | Accepted | planning |
| [0011](0011-security-model.md) | Security: local accounts, roles, Hardware Control Token, audit; LAN only | Accepted | planning |
| [0012](0012-simulation-strategy.md) | Simulation: protocol emulators, image-formation camera, ground-truth worlds (amended in P1.10b: the peripheral simulators, and the rig) | Accepted | planning |
| [0013](0013-2d-acquisition-and-stitching-strategy.md) | 2D acquisition and stitching strategy | Accepted | planning |
| [0014](0014-3d-scope-and-pluggability.md) | 3D reconstruction scope and pluggability | Accepted | planning |
| [0015](0015-phase-sizing-and-deliverables.md) | Phase sizing and per-phase deliverables (amended in P0.1: exact counting rules) | Accepted | planning |
| [0016](0016-hardware-baseline.md) | Hardware baseline and device roles | Accepted | planning |
| [0017](0017-workspace-toolchain-and-xtask.md) | Workspace layout, toolchain pin, and `cargo xtask` as the quality gate | Accepted | P0.1 |
| [0018](0018-coordinate-frames-units-and-handedness.md) | Coordinate frames, units and handedness (amended in P1.7: waiting on the clock) | Accepted | P0.2 |
| [0019](0019-logging-and-metrics.md) | Logging and metrics conventions | Accepted | P0.3a |
| [0020](0020-instrument-configuration.md) | Instrument configuration format and layering | Accepted | P0.3b |
| [0021](0021-frame-pool.md) | Frame pool design | Accepted | P1.1 |
| [0022](0022-fanout-consumer-policies-and-backpressure.md) | Fan-out consumer policies and backpressure | Accepted | P1.2 |
| [0023](0023-device-abstraction.md) | Device abstraction: dyn-compatible async traits, capability descriptors, status watches | Accepted | P1.3 |
| [0024](0024-synthetic-worlds-and-ground-truth.md) | Synthetic world model and ground-truth conventions | Accepted | P1.4 |
| [0025](0025-cpu-kernel-strategy.md) | CPU kernel strategy: rayon row bands, plain loops first, caller-owned buffers | Accepted | P1.5 |
| [0026](0026-simulated-camera-fidelity-and-the-noiseless-cache.md) | Simulated-camera fidelity versus performance, and the noiseless cache (amended in P1.7: timing, triggering and `TrueState`) | Accepted | P1.6 |
| [0027](0027-protocol-level-emulation-and-the-serial-transport.md) | Protocol-level emulation, and the serial transport abstraction (amended in P1.9: resets as GRBL's) | Accepted | P1.8 |
| [0028](0028-licence-exceptions-mpl-and-ijg.md) | Licence exceptions: MPL-2.0 for unmodified dependencies, and IJG with attribution (amended in P3.1: 0BSD and BlueOak-1.0.0) | Accepted | P1.8 |
| [0029](0029-grbl-motion-completion-polling-stop-and-recovery.md) | GRBL motion completion, status polling, and stop and recovery | Accepted | P1.9 |
| [0030](0030-data-driven-ascii-dialects.md) | Data-driven ASCII dialects: templates for both directions, and the dialect emulator (amended in P1.10b: the focus driver) | Accepted | P1.10a |
| [0031](0031-frame-pose-association-and-context-stamping.md) | Frame↔pose time association, and context stamping | Accepted | P2.1 |
| [0032](0032-preview-stream-design.md) | Preview stream design: two streams, server-side display mapping, latest-only, and rate caps | Accepted | P2.2 |
| [0033](0033-thing-decomposition-and-naming.md) | Thing decomposition and naming: camera, display, stage and focus (amended after P2.5: a refused setting is a 422) | Accepted | P2.3 |
| [0034](0034-sim-and-real-device-selection.md) | Simulated or real devices, chosen per device by the configuration (amended in P2.4: the peripherals) | Accepted | P2.3 |
| [0035](0035-the-channel-model-and-its-devices.md) | The channel model, and how it maps onto the devices | Accepted | P2.4 |
| [0036](0036-serving-and-embedding-the-web-app.md) | Serving and embedding the web app | Accepted | P3.1 |
| [0037](0037-observations-over-websocket-and-the-stream-budget.md) | Observations over WebSocket, and the stream budget | Accepted | P3.2 |
| [0038](0038-live-view-layout-and-stream-rules.md) | The Live view's layout, and when its streams open and close | Accepted | P3.3 |
| [0039](0039-input-mapping-and-motion-control-safety.md) | Input mapping and safety rules for motion control | Accepted | P3.4 |
| [0040](0040-desktop-shell-loads-the-local-service-s-web-app.md) | The desktop shell loads the web app from the local service | Accepted | P3.5 |
| [0041](0041-dataset-layout-and-raw-scratch-format.md) | Dataset layout, and the raw scratch format | Accepted | P4.1 |
| [0042](0042-the-ome-zarr-profile.md) | The OME-Zarr profile | Accepted | P4.2 |
| [0043](0043-job-scheduling-and-memory-admission.md) | Job scheduling and memory admission | Accepted | P4.3 |
| [0044](0044-pyramid-scheme.md) | The pyramid scheme: filter, level count and chunk alignment | Accepted | P4.4 |
| [0045](0045-on-demand-dzi-from-ome-zarr.md) | Deep-zoom tiles rendered on demand from OME-Zarr | Accepted | P4.5 |
| [0046](0046-the-ome-tiff-export-profile.md) | The OME-TIFF export profile | Accepted | P4.6 |
| [0047](0047-synthetic-datasets-and-ground-truth.md) | Synthetic datasets and their ground-truth files | Accepted | P4.9 |
| [0048](0048-the-registration-method.md) | The registration method | Accepted | P5.1 |
| [0049](0049-the-stage-camera-calibration.md) | The stage–camera calibration: its model, storage and versioning | Accepted | P5.2 |
| [0050](0050-the-shading-correction.md) | The shading correction, and the BaSiC simplification | Accepted | P5.3 |
| [0051](0051-calibration-ux-and-gating.md) | The calibration UX, and the gating rules | Accepted | P5.4 |
| [0052](0052-the-workflow-engine.md) | The workflow engine | Accepted | P6.1 |
