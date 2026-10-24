# apps

SpectrumIQ's binaries, one folder per app. Apps may depend on any crate in [`../crates`](../crates/README.md).
No crate depends on an app.

| App | What | Phase |
|---|---|---|
| `spectrumiq-server` | The service: hardware, data, processing, and the HTTP API and web app | P2.3 |
| `spectrumiq-desktop` | The Tauri 2 desktop shell around the web app | P3.5 |
| `siq-datagen` | Synthetic datasets with ground truth | P4.9 |
| `spectrumiq-viewer3d` | The native winit + wgpu volume viewer (optional) | P14.6 |

The Tauri app keeps its Rust crate in `spectrumiq-desktop/src-tauri/`, which the `apps/*` member
glob doesn't reach. P3.5 adds it to the workspace members explicitly.
