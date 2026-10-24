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
| serde_json | 1.0.151 | MIT OR Apache-2.0 | JSON | `xtask` | P0.1 | 1.0.152–1.0.154 were under two weeks old; pinned back |
| tempfile | 3.27.0 | MIT OR Apache-2.0 | Scratch folders in tests | `xtask` (dev) | P0.1 | Test-only |
