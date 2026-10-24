# Software of unknown provenance (SOUP) register

This register lists every third-party component SpectrumIQ ships or builds with (REQ-NFR-006).

- **Rows are added** by the phase that introduces the component.
- **`cargo xtask soup`** (P15.3) generates the full list from `cargo metadata` and `npm ls`, and
  diffs it against this file.
- **Licences** must follow the policy in [plan/conventions.md §7](plan/conventions.md#7-dependency-policy).

| Component | Version | Licence | Purpose | Used by | Introduced in | Notes |
|---|---|---|---|---|---|---|
| teta-wot | v0.1.0 (`135f9ff`) | MIT | Web of Things server framework | `siq-server` | P2.3 | Git dependency (ADR-0003) |
| Euresys eGrabber | 25.02.0.40 | Proprietary (Euresys EULA) | Coaxlink frame grabber SDK | `siq-euresys-sys` | P2.5 | Build-time and runtime prerequisite. Never vendored (ADR-0004) |
| microscope web app | `0640f99` | MIT | Starting point of the frontend (theme, ui, host, api/wot, app shell) | `web/` | P3.1 | Copied and adapted (ADR-0008) |
| Rust toolchain | 1.99.1 | MIT OR Apache-2.0 | Compiler, rustfmt, clippy | everything | P0.1 | Pinned in `rust-toolchain.toml` (ADR-0017) |
| anyhow | 1.0.104 | MIT OR Apache-2.0 | Error handling in tools | `xtask` | P0.1 | Binaries and xtask only (conventions §5.1) |
| clap | 4.6.7 | MIT OR Apache-2.0 | Command-line parsing | `xtask` | P0.1 | Same version as teta-wot |
| serde | 1.0.229 | MIT OR Apache-2.0 | Serialization | `xtask` | P0.1 | Reads `cargo metadata` and `package.json` |
| serde_json | 1.0.151 | MIT OR Apache-2.0 | JSON | `xtask`; `siq-types` (dev) | P0.1 | 1.0.152–1.0.154 were under two weeks old; pinned back. `float_roundtrip` on since P0.2 (ADR-0006) |
| tempfile | 3.27.0 | MIT OR Apache-2.0 | Scratch folders in tests | `xtask` (dev) | P0.1 | Test-only |
| chrono | 0.4.45 | MIT OR Apache-2.0 | UTC times for the `Clock` | `siq-types` | P0.2 | Same version as teta-wot |
| schemars | 1.2.2 | MIT | JSON Schemas of the shared types | `siq-types` | P0.2 | 1.2.3 was under two weeks old; pinned back. `uuid1` feature for the ids |
| thiserror | 2.0.21 | MIT OR Apache-2.0 | Error enums | `siq-types` | P0.2 | Same version as teta-wot |
| uuid | 1.27.0 | Apache-2.0 OR MIT | UUID v7 dataset and calibration ids | `siq-types` | P0.2 | 1.28 and 1.29 were under two weeks old; pinned back |
| proptest | 1.11.0 | MIT OR Apache-2.0 | Property tests | `siq-types` (dev) | P0.2 | 1.12.0 was under two weeks old; pinned back. Its `zerocopy` pinned to 0.8.59 |
| insta | 1.49.0 | Apache-2.0 | Snapshot tests | `siq-types` (dev) | P0.2 | Published 2026-10-03 |
| tracing | 0.1.44 | MIT | Structured logging | `siq-telemetry`, then every crate | P0.3a | Same version as teta-wot |
| tracing-subscriber | 0.3.23 | MIT | Console and file output, env filter | `siq-telemetry` | P0.3a | Features `std`, `registry`, `fmt`, `ansi`, `env-filter`, as teta-wot |
| tracing-appender | 0.2.5 | MIT | Daily rolling log files, non-blocking writer | `siq-telemetry` | P0.3a | Brings `time` 0.3.55; its `deranged` pinned to 0.5.8 (0.5.9 was under two weeks old) |
