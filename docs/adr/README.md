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
| [0003](0003-teta-wot-as-service-framework.md) | teta-wot v0.1.0 as the service framework, extended inside SpectrumIQ | Accepted | planning |
| [0004](0004-euresys-via-cpp-shim-and-bindgen.md) | Euresys eGrabber through a C++ shim and bindgen | Accepted | planning |
| [0005](0005-memory-model-and-out-of-core.md) | Memory model: pinned frame pool, streaming and out-of-core processing | Accepted | planning |
| [0006](0006-data-formats.md) | Data formats: OME-Zarr master, raw scratch, OME-TIFF and DZI derived (amended in P0.2: exact float round trips) | Accepted | planning |
| [0007](0007-pure-rust-wgpu-and-unsafe-policy.md) | Pure Rust + wgpu for algorithms; unsafe and native-code allow-list (amended in P1.1: test-only global allocators) | Accepted | planning |
| [0008](0008-frontend-stack-and-microscope-reuse.md) | Frontend: Vue + PrimeVue (unstyled) skinned with `microscope`'s tokens, copied from `microscope@0640f99` | Accepted | planning |
| [0009](0009-visualization-stack.md) | Visualization: OpenSeadragon first, WebGL2 16-bit later; VTK.js; optional native viewer | Accepted | planning |
| [0010](0010-streaming-tiers.md) | Streaming tiers, browser connection budget, and staged local fast paths | Accepted | planning |
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
| [0028](0028-licence-exceptions-mpl-and-ijg.md) | Licence exceptions: MPL-2.0 for unmodified dependencies, and IJG with attribution | Accepted | P1.8 |
| [0029](0029-grbl-motion-completion-polling-stop-and-recovery.md) | GRBL motion completion, status polling, and stop and recovery | Accepted | P1.9 |
| [0030](0030-data-driven-ascii-dialects.md) | Data-driven ASCII dialects: templates for both directions, and the dialect emulator (amended in P1.10b: the focus driver) | Accepted | P1.10a |
