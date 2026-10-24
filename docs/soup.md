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
