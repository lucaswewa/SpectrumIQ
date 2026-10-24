# ADR-0002: The core runs as a separate service; desktop and browsers are clients

- **Status:** Accepted
- **Date:** 2026-10-23
- **Phase:** planning
- **Related:** REQ-NET-001, REQ-NET-005, REQ-VIS-005

## Context

The goals ask for a Tauri desktop app and a web app, a decoupled client–server core, and local
"zero-copy" access. Scans can run for hours, so a UI crash or a closed window must not stop
acquisition.

## Decision

- **`spectrumiq-server`** is a headless process: a Windows service in production (teta-wot's
  `windows-service` feature), a console process in development. It owns all hardware, data and
  processing.
- **The desktop app** (Tauri 2) is a shell around the same web app the server serves. It connects
  to `http://127.0.0.1:<port>` like any browser. It adds only host services through the
  `HostAdapter`: dialogs, local login, launching the native viewer, and the later fast paths.
- **Image data never crosses Tauri IPC.**
- **Local fast paths are staged** (ADR-0010): loopback first, then direct tile reads, then
  WebView2 shared buffers, each gated by a benchmark.

## Alternatives considered

- **Tauri embeds the core:** simplest to install, but closing the UI stops acquisition for every
  client.
- **Both embedded and standalone:** doubles the build and test matrix for little gain.

## Consequences

- One code path for local and remote clients.
- Installation must deliver a service (P15.1).
- In development, the desktop app can start a server child process (P3.5).

## Verification

P3.5: the desktop shell works against a running server. P15.1: the service survives the UI
closing.
