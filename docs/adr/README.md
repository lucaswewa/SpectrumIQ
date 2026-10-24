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
| [0006](0006-data-formats.md) | Data formats: OME-Zarr master, raw scratch, OME-TIFF and DZI derived | Accepted | planning |
| [0007](0007-pure-rust-wgpu-and-unsafe-policy.md) | Pure Rust + wgpu for algorithms; unsafe and native-code allow-list | Accepted | planning |
| [0008](0008-frontend-stack-and-microscope-reuse.md) | Frontend: Vue + PrimeVue (unstyled) skinned with `microscope`'s tokens, copied from `microscope@0640f99` | Accepted | planning |
| [0009](0009-visualization-stack.md) | Visualization: OpenSeadragon first, WebGL2 16-bit later; VTK.js; optional native viewer | Accepted | planning |
| [0010](0010-streaming-tiers.md) | Streaming tiers, browser connection budget, and staged local fast paths | Accepted | planning |
| [0011](0011-security-model.md) | Security: local accounts, roles, Hardware Control Token, audit; LAN only | Accepted | planning |
| [0012](0012-simulation-strategy.md) | Simulation: protocol emulators, image-formation camera, ground-truth worlds | Accepted | planning |
| [0013](0013-2d-acquisition-and-stitching-strategy.md) | 2D acquisition and stitching strategy | Accepted | planning |
| [0014](0014-3d-scope-and-pluggability.md) | 3D reconstruction scope and pluggability | Accepted | planning |
| [0015](0015-phase-sizing-and-deliverables.md) | Phase sizing and per-phase deliverables | Accepted | planning |
| [0016](0016-hardware-baseline.md) | Hardware baseline and device roles | Accepted | planning |
