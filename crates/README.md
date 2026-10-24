# crates

SpectrumIQ's libraries, one folder per crate, named `siq-<area>`. They are organised in layers,
listed in [architecture §3](../docs/plan/architecture.md#3-crate-architecture).

| Layer | Crates |
|---|---|
| L0 Foundation | `siq-types`, `siq-config`, `siq-telemetry` |
| L1 Core | `siq-buffer`, `siq-imgproc`, `siq-gpu`, `siq-synth`, `siq-storage`, `siq-register`, `siq-mfenc`, `siq-shm` |
| L2 Devices | `siq-devices`, `siq-serial`, `siq-grbl`, `siq-ascii`, `siq-euresys-sys`, `siq-euresys`, `siq-sim` |
| L3 Processing | `siq-pyramid`, `siq-calib`, `siq-stitch-live`, `siq-stitch`, `siq-dff`, `siq-deconv`, `siq-section`, `siq-lightfield` |
| L4 Engines | `siq-acquire`, `siq-preview`, `siq-jobs`, `siq-workflow`, `siq-auth` |
| L5 Service | `siq-server` |

Each crate is created by its phase in [the roadmap](../docs/plan/README.md#3-roadmap). Every crate:

- has a README written from [the template](../docs/templates/crate-README.md);
- uses the workspace lints (`[lints] workspace = true`);
- is listed in `xtask/src/layers.rs`. `cargo xtask layers` fails for an unlisted crate, and for a
  dependency that breaks the layering.
