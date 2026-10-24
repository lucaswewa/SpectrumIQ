# M15 — Production readiness

**Outcome:** an installable, recoverable, documented instrument application, with traceability
artifacts that a future IVD effort can build on.

---

### P15.1 — Windows service and installer

**Budget** ~600 · **Depends on** P3.5, P9.2 · **Requirements** REQ-NET-001, REQ-NFR-001

**Scope**

- **Service management:** `spectrumiq-server service install|uninstall|start|stop`, using
  teta-wot's `windows-service` feature.
- **Service account and folders:** `%ProgramData%\SpectrumIQ\{config,settings,logs,audit}` with
  ACLs; the data root and scratch are configurable.
- **Tauri bundle (NSIS):** includes the server, installs and starts the service, adds the firewall
  rule for the LAN (with consent), and upgrades while preserving config and data.
- **Uninstall** keeps the data unless the user explicitly asks to remove it.

**Tests**

- Scripted install and uninstall on a clean VM (manual, with a checklist).
- Service start and stop.
- Upgrade.

**Example.** `docs/guide/install.md` walkthrough.

**ADR.** Packaging and service deployment (`microscope`'s ADR-0035 as a reference for third-party
notices).

---

### P15.2 — Fault tolerance and safe states

**Budget** ~800 · **Depends on** P10.3, P6.5 · **Requirements** REQ-SEC-004, REQ-NFR-003

**Scope**

- **Device fault detection and recovery:**
  - camera disconnect: reopen, and restore the settings;
  - serial reconnect: re-sync the position, and require homing after a GRBL reset;
  - focus-drive timeouts.
- **Shutdown order:** cancel workflows → stop live → park the stage (configurable) → flush the
  writers → close the devices.
- **Crash-safe manifests:** a journal plus a recovery scan at startup.
- **Watchdogs:** an acquisition-thread heartbeat, and stalled-job detection.
- **Safe state on control-token timeout during manual jogging:** stop the jog.

**Tests**

- Fault injection through the emulators: disconnects, garbage, and timeouts.
- Recovery assertions.

**Example.** `fault_injection_demo`.

**ADR.** Fault handling and safe states.

---

### P15.3 — Documentation and traceability tooling

**Budget** ~500 · **Depends on** all functional milestones · **Requirements** REQ-NFR-006

**Scope**

- **User guide** (mdBook, `docs/guide/`): operator and admin.
- **`cargo xtask trace`:** builds `docs/traceability.md`, the REQ ↔ tests ↔ phases matrix, from
  the test tags and `requirements.md`.
- **`cargo xtask soup`:** builds the SOUP register from `cargo metadata` and `npm ls`, with
  licenses, and diffs it against `docs/soup.md`.
- **Risk register template** (ISO 14971-style hazard table) in `docs/risk/`.
- **Release checklist.**

**Tests**

- `trace` and `soup` on fixtures.
- A doc-link checker.

**Example.** `cargo xtask trace`.

**ADR.** An amendment to ADR-0001 with the traceability process.
