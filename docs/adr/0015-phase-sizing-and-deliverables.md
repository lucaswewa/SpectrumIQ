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
