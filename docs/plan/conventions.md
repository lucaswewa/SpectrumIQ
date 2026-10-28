# Conventions and phase protocol

Every phase is implemented by a Claude Code session. A phase brief (in a milestone file), together with
this document, [architecture.md](architecture.md), and the ADRs it references, must be enough to
implement the phase without access to the planning conversation.

## 1. Phase protocol

### 1.1 Before starting a phase

1. Read, in order:
   - [README.md](README.md): decisions summary and roadmap status;
   - this file;
   - the architecture sections the brief references;
   - the phase brief;
   - every ADR the brief references;
   - the implementation notes (`docs/impl-notes/`) of the phases it depends on.
2. Check that every dependency phase is marked **Done** in the roadmap table.
3. Create a branch: `phase/P<m>.<n>-<slug>` (e.g. `phase/P1.1-frame-pool`).

### 1.2 While implementing

- Stay inside the brief's **Scope**. Anything else goes to "Follow-ups" in the implementation notes.
- If the brief is wrong, impossible, or would push the phase over budget, stop and describe the
  problem in the implementation notes ("Deviations"). Ask the user before changing any interface that
  later phases depend on (they are listed under **Interfaces** in the brief).
- Write the tests for the core logic first where practical. Where a brief gives a numeric acceptance
  target (accuracy, throughput, latency), add a test or benchmark that measures it.
- Keep the build green after each commit.

### 1.3 Definition of done (every phase)

- [ ] `cargo xtask ci` passes. It runs, in order:
      - fmt;
      - clippy (`-D warnings`);
      - tests;
      - rustdoc (`-D warnings`);
      - `layers`;
      - example smoke runs;
      - the licence check (when cargo-deny is installed);
      - the web checks (once `web/package.json` exists).

      The exact steps are in ADR-0017.
- [ ] `cargo xtask loc --base main` reports **< 1000** production lines (see §2).
- [ ] The tests listed in the brief exist and pass. Tests that verify a requirement are tagged with
      its ID (§5.3).
- [ ] At least one **example** exists, is documented in the crate/app README, and exits 0 in smoke
      mode (§6).
- [ ] At least one **ADR** is written or amended in `docs/adr/` for the phase's decisions (§4.1).
- [ ] A **README.md** exists and is up to date for every crate/app the phase creates or touches. The
      top-level README is updated if the change is user-visible.
- [ ] **Implementation notes** are written: `docs/impl-notes/P<m>.<n>.md`, from the template in §4.2.
- [ ] The roadmap status is updated in [README.md](README.md), and the "Verified by" column is
      updated in [../requirements.md](../requirements.md).
- [ ] New third-party dependencies are added to [../soup.md](../soup.md).
- [ ] Commits on the phase branch use Conventional Commits with the phase ID (`feat(P1.1): …`).
      The user reviews and merges.

## 2. Code budget and accounting

- **Budget:** < 1000 production lines per phase, **counting all languages together** (Rust, TS,
  Vue, WGSL, C/C++ shim, build scripts). Briefs carry an estimate. A phase whose estimate exceeds
  ~900 lines is split before starting.
- **Counted:** non-blank, non-comment lines added in `crates/*/src/**`, `crates/*/build.rs`,
  `crates/*/shim/**`, `apps/**/src/**`, `apps/**/src-tauri/src/**`, `web/src/**` (`.ts`, `.vue`,
  `.css`), `**/*.wgsl`, and `xtask/src/**`.
- **Not counted:**
  - tests: `tests/`, `**/tests.rs`, `**/tests/**`, `*.spec.ts`, `*.test.ts`, `web/e2e/**`;
  - `benches/`, `examples/`, and `web/src/views/dev/**` (the dev galleries and demo pages, which
    serve as frontend examples);
  - documentation (`*.md`), and configuration (`Cargo.toml`, `package.json`, `tauri.conf.json`,
    `*.toml` configs);
  - generated files (`**/generated/**`, `bindings.rs`, `openapi.json`, lockfiles).
- **Examples** have their own soft budget of about 300 lines per phase.
- **Copied code** from `microscope` at the pinned commit (ADR-0008) is not counted in the phase that
  imports it, if it is imported unchanged in a separate commit (`chore(P3.1): import … from
  microscope@0640f99`) and the files are listed in the impl notes. Later edits to those files
  count normally.
- **Unit tests** live in separate files (`#[cfg(test)] mod tests;` → `src/<module>/tests.rs`), so
  that counting by path is exact.
- `cargo xtask loc --base <rev>` implements these rules and prints counted, excluded, and example
  totals. The exact path and comment rules are in the ADR-0015 amendment.

## 3. Repository layout

```
Cargo.toml                 workspace
rust-toolchain.toml        pinned toolchain (1.99.1, x86_64-pc-windows-msvc)
.cargo/config.toml         the `cargo xtask` alias (own target dir: target/xtask)
deny.toml                  licence policy for cargo-deny
crates/siq-*/              libraries (see architecture.md §3)
apps/spectrumiq-server/    the service binary
apps/spectrumiq-desktop/   Tauri 2 desktop shell (src-tauri/ inside)
apps/siq-datagen/          synthetic dataset generator CLI
apps/spectrumiq-viewer3d/  native winit+wgpu viewer (late, optional)
web/                       Vue 3 + Vite + TS frontend (one codebase for browser and desktop)
examples/python/           scripting examples against the HTTP API
xtask/                     cargo xtask: ci, loc, gen-api, regen-bindings, trace, soup
config/                    example instrument configs (sim.toml, hardware.example.toml)
docs/                      goals, plan, adr, impl-notes, requirements, soup, guide (later)
```

## 4. Documentation produced by every phase

### 4.1 ADRs

- Location: `docs/adr/NNNN-kebab-title.md`. Take the next free number when writing the ADR; the
  phase briefs name ADR topics, not numbers.
- Template: [../adr/0000-template.md](../adr/0000-template.md) (Status, Date, Phase, Context,
  Decision, Alternatives, Consequences, Verification).
- A phase that only refines an earlier decision amends that ADR: add a dated "Amendment (P<m>.<n>)"
  section. It does not create a near-duplicate.
- Index every ADR in [../adr/README.md](../adr/README.md).

### 4.2 Implementation notes: `docs/impl-notes/P<m>.<n>.md`

```markdown
# P<m>.<n> — <title> — implementation notes
- Branch / commits:
- Date:
## Summary            what was built, in 3–6 bullets
## Interfaces          public types/functions/HTTP routes later phases rely on (with paths)
## Deviations          from the brief, and why
## Tests               list, with requirement IDs; how to run hardware/perf tests if any
## Measurements        LOC (counted / tests / examples), performance numbers vs targets
## Follow-ups          work discovered but out of scope (candidate phase or issue)
## Open questions      for the user
```

### 4.3 Crate / app README

Each README has these sections: purpose (2–3 lines), position in the architecture (layer, depends on,
used by), public API overview, examples (with commands), features/flags, testing notes (including
hardware/ignored tests), and limitations.

## 5. Coding standards

### 5.1 Rust

- Edition 2024, toolchain pinned in `rust-toolchain.toml`. Workspace lints:
  - `missing_docs = "warn"`;
  - `unsafe_code = "deny"`;
  - clippy `all = "warn"`, with CI running `-D warnings`.
- **Unsafe** is allowed only in the crates and modules listed in ADR-0007 (Euresys FFI, frame pool
  allocator, shared memory, Windows API calls). They use `#![allow(unsafe_code)]` at module scope, put
  a `// SAFETY:` comment on every block, and keep a safe public API.
- **Errors:** `thiserror` enums in libraries, `anyhow` only in binaries and xtask. Panics are allowed
  only for broken invariants, via `expect("why this cannot fail")`.
- **Async and threads:**
  - tokio for I/O and orchestration;
  - CPU-heavy work goes to rayon pools or dedicated threads, never onto tokio worker threads;
  - the frame path never allocates in steady state (ADR on the frame pool);
  - layer-3 algorithm crates are synchronous and do not depend on tokio (architecture §3).
- **Units:** physical quantities use newtypes from `siq-types` (`Micrometers`, `Millis`, …). Raw
  `f64` is allowed only inside numeric kernels.
- **Time:** code that waits or timestamps takes a `Clock` (from `siq-types`), so tests are
  deterministic (`ManualClock`, `tokio::time::pause`).
- **Logging:** `tracing`, with structured fields, and spans per workflow, job, and request.
- **Public types** that cross the HTTP API derive `Serialize`, `Deserialize` and `JsonSchema`.

### 5.2 Other languages

- **WGSL:** one kernel per file under `crates/<crate>/src/shaders/`, validated by naga in unit
  tests. Every kernel has a CPU reference implementation and an equivalence test (§5.3).
- **C++ shim (Euresys only):** C++17, `extern "C"` API, no exceptions across the boundary (catch
  everything and return error codes), no STL types in the C header.
- **TypeScript / Vue** follow the copied `microscope` rules (ADR-0008), with ESLint enforcing them
  where it can:
  - TS `strict`, `<script setup lang="ts">`, and Prettier (no semicolons, single quotes, 100
    columns);
  - hash routing (`createWebHashHistory`);
  - no platform APIs outside `src/host/` (`HostAdapter`: browser and Tauri implementations);
  - styles use only the design tokens in `src/theme/tokens.css`;
  - **only `src/ui/` imports `reka-ui`**, and pages use the `src/ui/` wrappers (ADR-0008, amended:
    PrimeVue 5 is no longer MIT);
  - components never call `fetch`; they go through `src/api/` and the Pinia stores;
  - every asset is bundled (no CDN), so the app works offline in Tauri;
  - every UI change is checked in both themes, and the dev galleries (`/#/dev/components`,
    `/#/dev/tokens`) are kept current.

### 5.3 Tests

| Kind | Where | Notes |
|---|---|---|
| Unit | `src/<module>/tests.rs` | Pure logic. Fast, deterministic, seeded. |
| Property | unit or `tests/`, `proptest` | Geometry, transforms, codecs, parsers. |
| Integration | `crates/<c>/tests/*.rs` | Across modules. Simulated devices; tokio paused time. |
| Snapshot | `insta` | JSON schemas, manifests, TD fragments, OpenAPI. |
| GPU equivalence | `tests/gpu_*.rs` | WGSL vs CPU reference within tolerance. Skipped with a logged reason if no adapter is present. |
| Ground truth | `tests/gt_*.rs` | Algorithms vs `siq-synth` ground truth, with numeric targets from the brief. |
| Hardware | `#[ignore = "hardware"]`, feature-gated | Run manually with `cargo xtask hw-test`. |
| Performance | `benches/` (criterion) or `examples/*_bench.rs` | Not in CI. Results recorded in impl notes. |
| Frontend unit | `*.spec.ts` (Vitest + Vue Test Utils) | Stores, components, API layer with mocked fetch. |
| E2E | `web/e2e/` (Playwright, from M9) | Against a debug `spectrumiq-server` on a fresh copy of `config/sim.toml`, in the installed Edge (ADR-0069). Opt-in: `cargo xtask ci --e2e`. |

**Requirement tagging.** Put `// REQ-XXX-NNN` in the doc comment or first line of every test that
verifies a requirement. `cargo xtask trace` builds the traceability matrix from these tags.

## 6. Examples

- **Rust:** `crates/<crate>/examples/<name>.rs`, run as `cargo run -p <crate> --example <name>`.
  Every example supports smoke mode (env `SIQ_SMOKE=1`: short duration, small sizes) and exits
  non-zero on failure. `cargo xtask ci` runs every example in smoke mode.
- **Frontend:** a dev-only demo page under `web/src/views/dev/` (mocked stores, works without a server),
  plus a written walkthrough in `web/README.md` for running it against the simulated server.
- **Scripting:** `examples/python/*.py` (httpx), run with `uv run`. Each script is documented with
  the server command it expects.

## 7. Dependency policy

- **Licenses** must be permissive: MIT, Apache-2.0, BSD-2/3-Clause, ISC, Zlib, Unicode, BSL-1.0.
  Anything else needs an ADR. ADR-0028 accepts MPL-2.0 for unmodified dependencies (`serialport`)
  and IJG with its attribution (`jpeg-encoder`). `cargo deny check licenses` runs in `xtask ci`
  when installed, and the npm side uses `license-checker`.
- **Native code** is limited to the allow-list in ADR-0007 (Euresys shim, OS APIs via
  `windows`/`windows-sys`, and the specific C libraries an ADR accepts).
- **Euresys SDK** files are never committed (EULA). The shim compiles against the installed SDK.
  Bindings are generated from our own shim header only.
- **Version age:** install only versions published at least two weeks earlier
  (`npm install --before=<date 14 days ago>`; on the Cargo side, check the release date before
  adding a crate). This is the same rule `microscope` uses.
- Every new dependency goes into [../soup.md](../soup.md): name, version, license, purpose, and
  which crate uses it.

## 8. Naming

- Crates: `siq-<area>`. Binaries and apps: `spectrumiq-<name>`. Thing names: `snake_case` nouns
  (`camera`, `stage`, `mosaic_live`).
- HTTP:
  - the UI is at `/` and `/assets/…`, with hash routes (`/#/live`);
  - teta-wot Things are under `/api/v1/…` (`api_prefix`, the same layout as `microscope`, so the
    copied client works unchanged);
  - SpectrumIQ's own routes (auth, data, tiles, system) are under `/api/siq/…`;
  - a startup test asserts that the two route sets never overlap.
- **Ports for manual checks:** the owner uses 5000 (server) and 5173 (Vite). Claude sessions use
  5290–5297 for the server and 5390–5399 for Vite (`--strictPort`), and stop only processes they
  started, by PID.
- Branches: `phase/P<m>.<n>-<slug>`. Commits: Conventional Commits with the phase ID in the scope.
