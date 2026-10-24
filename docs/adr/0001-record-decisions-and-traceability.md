# ADR-0001: Record decisions and keep traceability (IVD-ready)

- **Status:** Accepted
- **Date:** 2026-10-23
- **Phase:** planning
- **Related:** REQ-NFR-006, REQ-NFR-007

## Context

SpectrumIQ is research-use today, but may become a medical/IVD product later (IEC 62304,
ISO 13485, ISO 14971). Retrofitting traceability is expensive. Phases are implemented by separate
Claude Code sessions, which need durable written context.

## Decision

- **ADRs** in `docs/adr/` (this format) record every significant decision. Phases add ADRs or
  amend existing ones.
- **Requirement IDs** live in [../requirements.md](../requirements.md). Tests carry
  `// REQ-XXX-NNN` tags. `cargo xtask trace` builds the REQ ↔ tests ↔ phases matrix.
- **Third-party components** are listed in [../soup.md](../soup.md), and checked by
  `cargo xtask soup` from P15.3.
- **Per-phase implementation notes** go in `docs/impl-notes/`. Changes are tracked in git, with
  one branch per phase.
- **Risk management** starts in P15.3, as a hazard register template.

## Alternatives considered

- **Formal IEC 62304 process now:** too heavy for a research tool. The artefacts above keep that
  door open.
- **Decisions only in commit messages:** not discoverable by later sessions.

## Consequences

Every phase spends a little time on documentation. The traceability matrix can be produced on
demand. This is not a compliance claim: a formal process would add design reviews, a verification
plan, and software safety classification.

## Verification

- `cargo xtask trace` reports no untagged requirement at milestone exits (from M2).
- `cargo xtask soup` reports no unregistered dependency.
