# M0 — Foundations

**Outcome:** a Cargo workspace with pinned tooling, the `xtask` quality gate, the documentation
templates, and the domain types, configuration and telemetry that every other crate builds on.

**Exit criteria:**

- `cargo xtask ci` is green on a clean checkout;
- `cargo xtask loc` and `cargo xtask layers` work;
- `config/sim.toml` validates;
- the ADR, impl-notes and README templates are in use.

---

### P0.1 — Workspace, toolchain, xtask and templates

**Budget** ~600 · **Depends on** — · **Requirements** REQ-NFR-001, REQ-NFR-006, REQ-NFR-007

**Goal.** Turn the single `SpectrumIQ` binary crate into the workspace described in
[conventions.md §3](../conventions.md#3-repository-layout), with the quality gate that every later
phase relies on.

**Scope**

- **Start from the working tree.** It has uncommitted changes to `.gitignore` and `src/main.rs`, and
  an untracked `Cargo.lock`. Show them to the user before replacing anything. Remove the hello-world
  `src/main.rs` and `[package]`.
- **Root `Cargo.toml`:**
  - `[workspace]` with resolver 3 and members `crates/*`, `apps/*`, `xtask`;
  - `[workspace.package]`: edition 2024, `rust-version = "1.98"`, license MIT;
  - `[workspace.lints]`: `missing_docs = "warn"`, `unsafe_code = "deny"`, clippy `all`;
  - `[workspace.dependencies]` for the shared crates: serde, serde_json, schemars, thiserror,
    anyhow, tracing, tokio, rayon, uuid, chrono, proptest, insta, criterion;
  - `teta-wot = { git = "https://github.com/lucaswewa/teta-wot", tag = "v0.1.0" }`, declared now
    and used from M2.
- **Toolchain and formatting:**
  - `rust-toolchain.toml`: channel `1.99.1`, components rustfmt and clippy, target
    `x86_64-pc-windows-msvc`;
  - `rustfmt.toml` as in teta-wot;
  - `.gitattributes` with LF line endings.
- **`xtask` crate**, run as `cargo xtask <cmd>`:
  - `ci`: fmt `--check`, clippy `--workspace --all-targets -D warnings`, `test --workspace`, then
    every example with `SIQ_SMOKE=1`, `cargo deny check licenses` if installed, and the web steps
    when `web/package.json` exists;
  - `loc --base <rev>`: the accounting rules of conventions §2 (path classes, non-blank,
    non-comment);
  - `layers`: checks the layering table of architecture §3.1 against `cargo metadata`.
- **Templates:** `docs/impl-notes/TEMPLATE.md` and `docs/templates/crate-README.md`. The ADR
  template already exists.
- **Placeholder files:** `docs/soup.md` (header and the first rows) and `docs/impl-notes/`.

**Interfaces.** The `cargo xtask ci`, `loc`, and `layers` commands and their exit codes.

**Tests**

- `loc` classification on a fixture tree: counted, test, example, generated and copied files.
- `layers` rejects an L3→L2 dependency in a fixture metadata JSON.

**Example.** `cargo xtask loc --base <initial commit>` prints the table.

**ADR.**

- A new ADR for the workspace layout, toolchain pin, and xtask as the quality gate.
- An amendment to ADR-0015 that records the exact `loc` rules.

**Acceptance.** `cargo xtask ci` passes on the empty workspace. `loc` and `layers` give the expected
results on the fixtures.

---

### P0.2 — Domain types (`siq-types`)

**Budget** ~800 · **Depends on** P0.1 · **Requirements** REQ-ACQ-004, REQ-CAL-003

**Goal.** Shared vocabulary: units, coordinate frames, transforms, image views, frame metadata, ids
and clocks.

**Scope**

- **Units:** `Micrometers`, `Millimeters`, `Nanometers`, `Millis`, `Hertz`, as `f64` newtypes with
  arithmetic, `Display`, `serde(transparent)`, and a `JsonSchema` description carrying the unit.
- **Geometry:**
  - `Vec2Um`, `PointPx`, `RectUm`, `RectPx`;
  - `StagePose { x, y, z_coarse, focus }`;
  - `Affine2`: 2×3 with `compose`, `invert`, `apply`, `from_params(scale_x, scale_y, rotation,
    shear, translation)` and `decompose()`.
- **Imaging:**
  - `PixelFormat` (Mono8/10/12/14/16 in u16 containers), `Roi`, `Binning`;
  - `ImageView<'a, T>` and `ImageViewMut<'a, T>`: width, height, stride, `row()`, `sub_view()`, and
    no allocation;
  - `OwnedImage<T>`: aligned storage, for tests and tools only.
- **Frames:** `FrameSeq`, `FrameMeta { seq, device_ts, host_ts_mid_exposure, exposure, gain, roi,
  binning, pixel_format, pose: Option<StagePose>, pose_quality, channel, objective, calibration,
  flags }`. `PoseQuality` is `Settled`, `Interpolated { uncertainty }` or `Unknown`.
- **Ids:** `DatasetId` (uuid v7), `ChannelId`, `ObjectiveId`, `CalibrationId`.
- **Time:** `Clock` trait (monotonic `Timestamp` plus UTC mapping), `SystemClock`, `ManualClock`.

**Interfaces.** Every type above is used by all later crates. Changing them after M1 needs an ADR
amendment.

**Tests**

- Proptest: `Affine2` invert∘apply ≈ identity, compose associativity, decompose→from_params
  round trip.
- `ImageView` bounds and stride.
- Serde round trips, and insta snapshots of the JSON schemas.

**Example.** `affine_demo`: builds a stage↔pixel calibration and maps a few points.

**ADR.** Coordinate systems, units and handedness: stage, camera, and mosaic frames; the +z
direction; pixel centre convention.

---

> **P0.3 was split into P0.3a and P0.3b during implementation.** The complete phase measured
> about 1060 counted lines against its estimate of 800. Telemetry and configuration are
> independent crates, so they became two phases (see `docs/impl-notes/P0.3a.md`). "P0.3"
> elsewhere means both.

### P0.3a — Telemetry (`siq-telemetry`)

**Budget** ~300 · **Depends on** P0.2 · **Requirements** — (it serves REQ-ACQ-005 and
REQ-NFR-003 later, through metrics)

**Goal.** Logging and metrics that every service component uses.

**Scope**

- **`init(LogConfig)`:** console plus daily rolling files (`tracing-appender`), with an env
  filter. `LogConfig` serializes, so the instrument configuration can embed it as `[log]`.
- **Metrics registry:** atomic `Counter`, `Gauge`, and fixed-bucket `Histogram`, named, with a
  JSON snapshot.

**Tests**

- Metrics under concurrent updates.
- Bucket edges.
- Name, kind and bounds conflicts.
- The snapshot's JSON.
- Logging to a daily file.

**Example.** `metrics_demo`.

**ADR.** Logging and metrics conventions.

---

### P0.3b — Instrument configuration (`siq-config`)

**Budget** ~800 · **Depends on** P0.3a · **Requirements** REQ-NFR-002, REQ-SIM-006

**Goal.** One validated instrument configuration.

**Scope**

- **`InstrumentConfig`,** loaded from TOML, with these sections:
  - `[server]`: bind, port, data_root, scratch;
  - `[memory]`: pool_slots, job_ram_gb, job_vram_gb, tile_cache_mb;
  - `[log]`: P0.3a's `LogConfig`;
  - `[camera]`, `[stage]`, `[focus]`: `kind = "sim" | "euresys" | "grbl" | "ascii"`, plus kind
    parameters (port, baud, poll_hz, dialect path);
  - `[[objectives]]`, `[[channels]]`, `[peripherals.*]`;
  - `[sim]`: world, seed, error injection, profile.
- **Validation** reports source spans. Defaults are applied, and `--print-config-schema` (JSON
  Schema) is supported.
- **Ship** `config/sim.toml` and `config/hardware.example.toml`.

**Tests**

- Valid and invalid configs, with insta snapshots of the error messages.
- Defaults.
- The schema snapshot.

**Example.** `validate_config <file>`.

**ADR.** Configuration format and layering: a TOML instrument config, while teta-wot settings
files hold runtime-tunable values.
