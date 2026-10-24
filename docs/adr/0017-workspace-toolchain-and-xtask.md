# ADR-0017: Workspace layout, toolchain pin, and `cargo xtask` as the quality gate

- **Status:** Accepted
- **Date:** 2026-10-23
- **Phase:** P0.1
- **Related:** ADR-0015, REQ-NFR-001, REQ-NFR-006, REQ-NFR-007

## Context

The repository started as one `SpectrumIQ` binary crate. The plan needs about 35 crates and 4
apps, in layers with rules between them (architecture §3), and a gate that every phase passes
before review (conventions §1.3). CI is local only for now, so the gate must run with one command
on the development machine. That machine runs Windows, where a running executable can't be
overwritten.

## Decision

- **A virtual Cargo workspace** with resolver 3 and members `crates/*`, `apps/*` and `xtask`.
  - Cargo ignores glob matches that aren't folders, so `crates/` and `apps/` exist from the start
    with only a README, which indexes their contents.
  - `[workspace.package]`: edition 2024, `rust-version = "1.98"` (teta-wot's minimum), MIT,
    `publish = false`.
  - `[workspace.dependencies]`: the shared versions. They match teta-wot's requirements where it
    uses the same crates, so the two resolve to one copy.
  - `[workspace.lints]`: `missing_docs = "warn"`, `unsafe_code = "deny"`, clippy `all`. Every
    crate opts in with `[lints] workspace = true`.
- **Toolchain:**
  - `rust-toolchain.toml` pins `1.99.1` (with rustfmt, clippy, and target
    `x86_64-pc-windows-msvc`);
  - `rustfmt.toml` sets `style_edition = "2024"`, as teta-wot does;
  - `.gitattributes` stores text with LF.
- **`cargo xtask`** is a workspace crate: a library with a thin `clap` command line. Its tasks are
  `ci`, `loc` and `layers`.
  - The alias in `.cargo/config.toml` is `run --quiet --package xtask --target-dir target/xtask --`.
    The separate target directory keeps the running `xtask.exe` out of the workspace builds that
    `ci` starts.
- **`ci` runs these checks in order** and stops at the first failure:
  1. `cargo fmt --check`;
  2. clippy `--all-targets -D warnings`;
  3. `cargo test`;
  4. `cargo doc` with `RUSTDOCFLAGS=-D warnings`. This step is added beyond conventions §1.3,
     because broken intra-doc links are cheap to catch.
  5. `layers`;
  6. every example with `SIQ_SMOKE=1`, skipping those with `required-features`;
  7. `cargo deny check licenses`, with `deny.toml`, if cargo-deny is installed; otherwise it is
     reported as skipped;
  8. the web scripts `format:check`, `lint`, `typecheck`, `test:unit`, `build` and
     `api:types -- --check`, if `web/package.json` declares them (`npm ci` first, when
     `node_modules` is missing). `--e2e` adds `test:e2e`, and `--skip-web` skips the web.

  Every Cargo step runs with `--locked`.
- **`layers` enforces architecture §3.1:**
  - a crate depends only on its own layer or lower ones;
  - L3 Processing never depends on L2 Devices;
  - only `siq-server` depends on `teta-wot` or `teta-wot-*`;
  - apps and tooling may depend on anything, and nothing depends on them;
  - every workspace crate must appear in `xtask/src/layers.rs`.

  **Dev-dependencies are exempt,** so tests may use simulators to make data.
- **`loc`** implements conventions §2. The exact rules are in the ADR-0015 amendment.
- **Dependency age:** after any lockfile change, newly resolved versions published less than two
  weeks earlier are downgraded with `cargo update -p <crate> --precise <version>`. P0.1 did that
  for `equivalent`, `libc`, `serde_json`, `syn` and `unicode-ident`.

## Alternatives considered

- **`just`, `make` or PowerShell scripts:** another tool to install, and logic like `loc`'s diff
  accounting is easier to test in Rust.
- **One fixed list of workspace members:** every phase would edit it. The globs pick up new crates
  by themselves.
- **Pinning 1.98.1, like teta-wot and microscope:** either pin needs a download, since only
  `stable` was installed. 1.99.1 is the current stable.
- **The `cargo_metadata` crate:** pulls in more dependencies than the 40 lines of serde types that
  the tasks need.

## Consequences

- The first build on a machine downloads the pinned toolchain.
- xtask is built twice, once in `target/xtask` and once in `target/` for clippy and the tests. It
  is small.
- **The Tauri crate** (`apps/spectrumiq-desktop/src-tauri`) sits one level deeper than the
  `apps/*` glob reaches. P3.5 adds it to `members` explicitly.
- **A new crate fails `layers`** until it is added to the table. That is deliberate.
- **The licence check is unverified** until cargo-deny is installed.

## Verification

- `cargo xtask ci` passes on the workspace.
- Unit and integration tests in `xtask/`, tagged REQ-NFR-007:
  - `loc`: path classes from a fixture, and a measurement in a scratch git repository;
  - `layers`: violations and a clean pass on fixture metadata, and this workspace.
