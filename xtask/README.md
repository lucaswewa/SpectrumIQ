# xtask

SpectrumIQ's development tasks, run as `cargo xtask <task>`:

- `ci` is the quality gate every phase passes;
- `loc` measures the code budget;
- `layers` checks the crate layering;
- `regen-bindings` regenerates the Euresys shim's bindings.

## Place in the architecture

- **Layer:** Tooling, outside the architecture. It may depend on anything, and nothing depends on
  it.
- **Depends on:** `anyhow`, `clap`, `serde`, `serde_json`; `tempfile` in tests. It runs `cargo`,
  `git` and, for the web checks, `npm`; `regen-bindings` runs `bindgen` and `rustfmt`.
- **Used by:** every phase's definition of done ([conventions §1.3](../docs/plan/conventions.md#13-definition-of-done-every-phase)).
- **Phase:** P0.1; P2.5 added `regen-bindings`, and the check for Euresys SDK files.

The `cargo xtask` alias in [`.cargo/config.toml`](../.cargo/config.toml) builds xtask into
`target/xtask`. The running `xtask.exe` is therefore never overwritten by the workspace builds it
starts; Windows can't replace a running executable.

## API

The binary is a thin command line over the library:

| Task | Module | What it does |
|---|---|---|
| `cargo xtask ci [--skip-web] [--e2e]` | `ci` | Runs fmt, clippy (warnings denied), the tests, rustdoc (warnings denied), `layers`, a check that no tracked file is a Euresys SDK file (REQ-NFR-005), every example in smoke mode, `cargo deny check licenses` when cargo-deny is installed, and the web checks once `web/package.json` exists. It stops at the first failure. |
| `cargo xtask loc [--base main] [--budget 1000] [--files]` | `loc` | Counts the lines added since the merge base of `--base` and `HEAD`, including uncommitted and untracked files, by class. It fails when the counted lines reach the budget. |
| `cargo xtask layers` | `layers` | Checks every workspace crate's normal and build dependencies against the layer table of architecture §3.1. |
| `cargo xtask regen-bindings` | `regen` | Regenerates `crates/siq-euresys-sys/src/bindings.rs` from the shim's header with `bindgen` (ADR-0004), and formats it. It needs `cargo install bindgen-cli`, and LLVM with `LIBCLANG_PATH` set; it says so when they are missing. |

### `loc`

The counting rules are those of
[conventions §2](../docs/plan/conventions.md#2-code-budget-and-accounting):

- **Classes:** `loc::classify` sorts paths into counted, tests, examples, generated, docs, config
  and other. The first matching rule wins, in that order of precedence: generated, tests,
  examples, docs, counted, config, other.
- **Lines:** `loc::LineCounter` counts non-blank lines that aren't only a comment. It handles
  `//`, `/* */` and `<!-- -->` in C-like files and `#` in TOML, YAML, Python and shell. Plain
  files count every non-blank line.
- **Import commits:** a commit whose subject is `chore(P<m>.<n>): import …` copies files in
  unchanged. Those files count only from that commit on, and what it copied is reported as
  `copied`.
- **Rules:** [ADR-0015's amendment](../docs/adr/0015-phase-sizing-and-deliverables.md) records the
  exact rules.

### `layers`

`layers::TABLE` gives every planned crate its layer.

- A crate may depend on its own layer or below.
- Processing never depends on Devices.
- Only `siq-server` uses teta-wot.
- Apps and tooling may use anything.
- Dev-dependencies are exempt.
- A workspace crate missing from the table is a violation, so each new crate is placed explicitly.

## Examples

| Example | What it shows | Run |
|---|---|---|
| `classify` | How `loc` classifies paths: given paths, or a self-checking sample | `cargo run -p xtask --example classify -- <path>…` |

`cargo xtask loc --base <rev>` is the task's own demonstration:

```text
class       files   lines
counted         7     810
tests           7     435
…
budget: 810 counted lines, below 1000: OK
```

## Features

None.

## Testing

- **Unit tests** (`src/*/tests.rs`): line counting, comment handling, diff parsing, import-commit
  subjects, the layer rules, and which paths are Euresys SDK files.
- **Integration tests** (`tests/`):
  - path classes from `tests/fixtures/loc_paths.txt`;
  - a measurement in a scratch git repository, with a phase branch, an import commit, a base
    branch that moved on, and uncommitted and untracked files;
  - layering on fixture metadata;
  - layering on this workspace.

Run them with `cargo test -p xtask`. They need `git` on the `PATH`.

## Limitations

- **Comment detection is per line**, with a heuristic for diff hunks that start inside a block
  comment: a line of the form `* …` is treated as a comment. Arithmetic continuation lines that
  start with `* ` are rare, and count as comments.
- **Lines are counted as added**, not net. Replacing a line counts once.
- **`regen-bindings` hasn't run yet:** libclang isn't installed. P2.5 wrote the first bindings
  by hand, in bindgen's form.
- **The licence check is skipped** until cargo-deny is installed. `deny.toml` holds the policy,
  and hasn't yet been run.
