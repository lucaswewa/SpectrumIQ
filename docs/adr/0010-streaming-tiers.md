# ADR-0010: Streaming tiers, browser connection budget, and staged local fast paths

- **Status:** Accepted
- **Date:** 2026-10-23
- **Phase:** planning
- **Related:** REQ-PRV-001–005, REQ-NET-005, REQ-NFR-004

## Context

- **Raw data cannot be streamed to clients.** It is 1.8 GB/s.
- **The LAN is plain HTTP, and teta-wot serves HTTP/1.1 only.** Browsers therefore allow
  **6 connections per origin**, and each MJPEG or SSE stream holds one connection. `microscope` hit
  this limit with only a few Things (its ADR-0024).
- **teta-wot already provides `MjpegStream`.**
- **A 1080p overview of a 100 MP frame is about 6–9× smaller per axis**, too coarse for focusing.

## Decision

- **Live (v1):**
  - an MJPEG overview at ≤ 1080p;
  - an MJPEG ROI at 1:1 or 2:1;
  - an MJPEG viewport of the live mosaic.

  The LUT is applied on the server.
- **Connection budget:**
  1. Observations go over WebSocket, one per Thing (not counted against the 6-connection HTTP
     pool).
  2. At most 3 MJPEG streams are open, and hidden views close theirs.
  3. Tiles and API calls use the rest.

  P3.2 verifies this, and P9.5 re-tests it under load.
- **Later:**
  - a multiplexed binary WS for all live frames, with per-client flow control (P14.2);
  - H.264 through the Media Foundation hardware encoder (NVENC on the RTX 3060), played with
    WebCodecs (P14.3).
- **Local fast paths** for the desktop, each kept only if a benchmark shows a gain over loopback:
  - v1: loopback, like a browser;
  - v2: direct DZI tile reads through a Tauri custom protocol (P14.4);
  - v3: named shared memory to WebView2 `SharedBuffer`, for true zero-copy (P14.5).
- **Optional TLS** through a reverse proxy enables HTTP/2 and removes the limit.

## Alternatives considered

- **H.264 or WebRTC from the start:** more complex, and not needed on a LAN at 2–4 Mbit/s per
  stream.
- **SSE per Thing (as in `microscope`):** exceeds the connection budget with SpectrumIQ's Things.

## Consequences

- The copied client gains a WS transport.
- The UI must manage stream lifetimes deliberately.

## Verification

- P3.2: transport and budget tests.
- P9.5: an e2e connection-budget scenario and a soak test.
- P14.4 and P14.5: benchmark gates.
