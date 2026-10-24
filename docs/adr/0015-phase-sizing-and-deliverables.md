# ADR-0015: Phase sizing and per-phase deliverables

- **Status:** Accepted
- **Date:** 2026-10-23
- **Phase:** planning
- **Related:** REQ-NFR-007

## Context

The goals require milestones made of phases, each with fewer than 1,000 lines of code, excluding
tests and documents. Every phase must have implementation notes, ADRs, a README, tests and
examples. The phases are implemented by Claude Code, one phase per session.

## Decision

- **Budget:** < 1,000 production lines per phase, **counting all languages together**. Tests,
  docs, generated files and configuration are excluded, and so is code copied unchanged from
  `microscope` in a separate import commit. Examples have a soft budget of about 300 lines. The
  exact rules are in [conventions §2](../plan/conventions.md#2-code-budget-and-accounting), and
  `cargo xtask loc` measures them.
- **Each phase delivers:**
  - code;
  - tests, tagged with requirement IDs;
  - at least one example that runs in smoke mode;
  - at least one ADR, new or amended;
  - a crate or app README;
  - `docs/impl-notes/P<m>.<n>.md`;
  - updated roadmap status and requirement traceability.
- **Self-contained briefs.** Each phase brief can be followed without the planning conversation,
  as the protocol in [conventions §1](../plan/conventions.md#1-phase-protocol) describes.

## Alternatives considered

- **`microscope`'s S/M/L budgets, which count tests and comments:** they don't match the goals'
  definition.

## Consequences

86 phases. Larger features are split across phases, with interfaces stated up front.

## Verification

`cargo xtask loc --base main` in each phase's definition of done.

## Amendment (P0.1, 2026-10-23): the exact counting rules

`cargo xtask loc` (`xtask/src/loc.rs`) implements these rules.

**What is compared.** The merge base of `--base` and `HEAD` is compared with the working tree:
commits, staged and unstaged changes, and untracked files that aren't ignored. A base branch that
moved on after the phase began therefore doesn't count. Lines are counted as *added*: a changed
line counts once, and removed lines don't count.

**Classes.** Paths are relative to the repository, and the first rule that matches wins:

1. **generated:**
   - a `generated/` folder;
   - `bindings.rs` or `openapi.json`;
   - a lockfile (`Cargo.lock`, `package-lock.json`, `pnpm-lock.yaml`, `yarn.lock`).
2. **tests:**
   - a `tests/` or `benches/` folder;
   - `tests.rs`, `*.spec.ts` or `*.test.ts`;
   - anything under `web/e2e/`.
3. **examples:**
   - an `examples/` folder;
   - anything under `web/src/views/dev/`, the dev galleries and demo pages copied from
     `microscope`. This corrects conventions' earlier `web/src/dev/`.
4. **docs:** `*.md`, or anything under `docs/`.
5. **counted:**
   - `*.wgsl` anywhere;
   - `crates/<c>/build.rs`;
   - code files under `crates/<c>/src/`, `crates/<c>/shim/` and `xtask/src/`;
   - code files under an `apps/**/src*/` folder, or an app's `build.rs`;
   - `.ts`, `.vue` and `.css` files under `web/src/`.

   "Code files" means `.rs .wgsl .c .cc .cpp .cxx .h .hpp .ts .tsx .js .mjs .vue .css`.
6. **config:**
   - `*.toml`, `*.json`, `*.yml`, `*.yaml`;
   - `.git*` files, `.prettierrc*`, `.nvmrc`.
7. **other:** everything else, such as `web/vite.config.ts` and `LICENSE`.

**Lines.** A line counts if it isn't blank and isn't only a comment.

- **C-like files** (Rust, WGSL, C/C++, TS/JS, Vue, CSS, HTML):
  - `//` line comments, including `///` and `//!`, don't count;
  - `/* … */` and `<!-- … -->` block comments don't count, and they carry over to later lines;
  - code after a comment's end on the same line counts;
  - a hunk that starts inside an older block comment is detected by its `*`, `* …` and `*/`
    lines.
- **`#` comments** apply to TOML, YAML, Python, shell and PowerShell.
- **Plain files** count every non-blank line.

**Import commits.** A commit whose subject matches `chore(P<m>.<n>): import …` copies files in
unchanged.

- The files it **adds** are compared from that commit, not from the merge base. Only later edits
  to them count.
- What the commit added is reported as `copied`: files and `git --numstat` lines.

**Budget.** The task fails, exit code 1, when the counted lines are not below `--budget` (default
1000).
