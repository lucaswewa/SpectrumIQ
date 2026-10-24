# ADR-0028: Licence exceptions: MPL-2.0 for unmodified dependencies, and IJG with attribution

- **Status:** Accepted
- **Date:** 2026-10-24
- **Phase:** P1.8, decided by the owner
- **Related:** ADR-0027, conventions §7, REQ-NFR-006

## Context

- **The licence policy** (conventions §7) allows MIT, Apache-2.0, BSD-2/3-Clause, ISC, Zlib,
  Unicode and BSL-1.0. Anything else needs an ADR.
- **The serial transport** (P1.8) uses `tokio-serial` and `mio-serial` (MIT), which depend on
  `serialport` 4.10.1. `serialport` is MPL-2.0.
- **The JPEG previews** (P2.2) are planned on `jpeg-encoder` 0.7.1. It is
  "(MIT OR Apache-2.0) AND IJG", because it derives from the Independent JPEG Group's code.

## Decision

- **MPL-2.0 is allowed for dependencies used unmodified.**
  - MPL-2.0 is copyleft per file. Our code stays MIT, and only changes to the MPL files
    themselves would have to be shared.
  - We don't vendor or patch them. Patching one needs a new ADR.
- **IJG is allowed,** with the attribution its licence requires: "This software is based in part
  on the work of the Independent JPEG Group". It goes into the documentation and the application's
  About page when `jpeg-encoder` arrives (P2.2).
- **`deny.toml` gains `MPL-2.0` and `IJG`,** and conventions §7 refers to this ADR.
- **The SOUP register** records `serialport` (MPL-2.0) now, and `jpeg-encoder` (IJG) when P2.2
  adds it.

## Alternatives considered

- **A serial port of our own on `windows-sys`:**
  - no new licence;
  - but about 200 lines of `unsafe` overlapped I/O, and a port to maintain.
- **Deferring the real port to P10.3:** P1.9's manual `--port COM5` path would wait for it.
- **A pure MIT JPEG encoder:** none as mature was found. NVENC (M14) is the later route for
  video.

## Consequences

- **Each MPL-2.0 or IJG component** is listed with its licence in `docs/soup.md`.
- **The IJG attribution** becomes a release-checklist item (P15.3).

## Verification

- `cargo deny check licenses` passes with the two additions, when cargo-deny is installed.
- `docs/soup.md` lists `serialport` with MPL-2.0.
