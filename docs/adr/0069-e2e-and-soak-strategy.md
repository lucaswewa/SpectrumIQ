# ADR-0069: The end-to-end and soak strategy, with its targets

- **Status:** Accepted
- **Date:** 2026-10-28
- **Phase:** P9.5
- **Related:** ADR-0010, ADR-0032, ADR-0066, ADR-0068, REQ-NFR-003, REQ-NFR-004, REQ-SEC-002

## Context

The unit and integration tests check the pieces, in process. M9 shares one instrument among many
clients, and two claims need the real thing:

- **The connection budget (ADR-0010).** A browser over HTTP/1.1 allows 6 connections per origin.
  The web app keeps to 3 MJPEG streams, with observations over WebSocket, so tiles and calls have
  the rest. That has to hold in a browser, with every stream open at once.
- **M9's exit criterion.** A 30-minute soak with 8 viewers and an operator must show no lost
  camera frames and the preview latency within target. REQ-NFR-004 sets that at 250 ms from
  exposure end to display.

## Decision

- **End-to-end tests: Playwright, in `web/e2e/`, opt-in** (`cargo xtask ci --e2e`, or
  `npm run test:e2e`). They run against the app as the server serves it: a debug build of this
  checkout, serving the fresh web build.
  - **A fresh instrument for each run:** `config/sim.toml`, copied to the system's temporary
    folder, with a 2048 × 1536 camera rendered fast and a 15 s token timeout. Its data, settings,
    audit and logs are beside it, and the folder is emptied first.
  - **The installed Edge** (`channel: 'msedge'`): no browser is downloaded.
  - **One worker:** the tests share the one instrument. Each starts with the token free and live
    view stopped: an admin takes the token over, stops live view, and lets the token go.
  - **A setup project first:**
    - the first admin with the CLI, and the other users over HTTP;
    - the stage–camera calibration and every channel's dark and flat, so that the wizard stays
      shut;
    - a snap into a dataset, with its pyramid, for the viewer.
  - **The scenarios:**
    - the login;
    - live view;
    - two browsers handing the token over;
    - a silent holder losing it;
    - the connection budget: in one browser, the Live and Mosaic pages hold the 3 MJPEG streams
      (the overview, the ROI and the live mosaic's view) and the Things' sockets, while the
      Viewer page loads its tiles. Every tile must arrive, and the streams and observations must
      go on meanwhile.
- **The soak: `cargo xtask soak --viewers 8 --minutes 30`;** `--smoke` is 2 viewers for 20 s.
  - **Its own service:** a release build on port 5097, on a fresh copy of `config/sim.toml` at
    full size in `target/soak/`, with an admin, who operates, and a viewer added through the CLI.
  - **The operator:**
    - takes the token, turns auto-stitch off, calibrates the stage against the camera, starts live
      view, and snaps an image for the viewers;
    - then runs 2 × 2 tile scans until the time is up;
    - deletes each scan's dataset, so that the disk doesn't fill;
    - looks at live view for 5 s between scans.
  - **Each viewer:** follows the overview and ROI streams, observes the camera and the stage over
    their sockets, and pans about the snap's tiles: 4 × 3 tiles of a level, every second.
  - **Every 5 s,** the service is sampled:
    - its working set, private bytes and CPU time (`cpu_s`, new in `/api/siq/system/memory`);
    - its metrics: the frames published and lost, and the previews' latency and work.
  - **The report** is printed and written to `target/soak/report.json`. The soak fails if a
    target is missed.
  - **Blocking clients, a thread each:**
    - HTTP/1.0, whose bodies end when the connection closes, so no chunked decoding is needed;
    - `tungstenite`'s client for the sockets, already in the lockfile through axum.

    xtask gets no async runtime.
- **The targets:**

  | Measure | Target | Why |
  |---|---|---|
  | Camera frames lost | 0 | REQ-NFR-003 |
  | The previews' pipeline work (`preview.<stream>.work_ms`), p95 | ≤ 100 ms | A camera's readout at 9 fps (≤ 111 ms), this, the LAN's few ms for a JPEG of about 300 KB, and the browser's decoding add up to 250 ms (REQ-NFR-004) |
  | The frames each viewer receives | ≥ 90 % of those made | A stream is latest-only, and skips only what its client can't keep up with (ADR-0032) |
  | Tiles | none failed, p95 ≤ 1 s | A viewer pans without waiting |
  | Observations | every viewer receives some | The sockets keep working under load |

  The latency from mid-exposure (`latency_ms`) is reported, but isn't a target on the
  simulator. There, it includes the simulator's rendering of each 100 MP frame on the CPU, about
  a quarter of a second, which a camera's readout doesn't take. On the instrument it is the
  measure: P10.2 runs the soak there. CPU and RAM are reported, without targets.

## Alternatives considered

- **A browser for each viewer in the soak:** 8 Edge processes measure the browsers more than the
  service. The end-to-end tests cover the browser.
- **The soak in Python, as `live_soak.py`:** the standard library has no WebSocket client.
- **An async client in xtask (tokio and hyper):** a runtime and a client stack for a few dozen
  connections, where a thread each suffices.
- **Playwright's own Chromium:** downloading it changes the computer, and Edge comes with Windows.
- **Timing each frame from exposure to the client:** teta-wot's MJPEG parts carry no timestamp,
  and teta-wot stays unmodified.

## Consequences

- **The end-to-end tests** need Edge and port 5098. They take about a minute once the builds
  are done.
- **The soak** needs a release build, port 5097, and about 3 GB of RAM for the pool. It replaces
  `target/soak/` each run.
- **On the simulator,** the camera's rate is the simulator's: it renders 100 MP frames on the
  CPU, well under 9 fps. The soak's load is still the service's own: the streams, sockets,
  tiles, scans and their datasets.

## Verification

- `web/e2e/`: `login`, `live`, `handover` (the hand-over and the timeout) and `budget`, after
  `setup`: six passed, twice running, in 1.1 min each.
- `xtask` `soak::tests`: an MJPEG stream's parts counted once however it is split; a histogram's
  quantile since the first sample; a percentile.
- `cargo xtask soak --smoke`: every target met.
- `cargo xtask soak --viewers 8 --minutes 30`, on this workstation (16 logical cores) with the
  simulator at full size:
  - **met:**
    - no camera frame lost, of 8,537;
    - every viewer received every frame of both streams;
    - 136,844 tiles, none failed, p95 1 ms, at most 1 s;
    - at least 19,102 observations a viewer;
    - 210 scans, 3.1 s each;
    - the ROI's work p95 ≤ 100 ms;
  - **missed:** the overview's work, 42 ms on average, but p95 between 100 and 200 ms;
  - **reported:** from mid-exposure, 304 ms on average for the overview and 287 for the ROI. The
    service used 65 % of the cores on average, 84 % at most. Its working set reached 1.93 GB, and
    its private bytes ended at 2.97 GB, from 2.94.
- **What the overview's tail comes from,** found with shorter runs:
  - The operator alone, without viewers, for 3 minutes: the overview's work 27 ms on average, p95
    ≤ 60 ms, at the same 63 % of the cores. So the viewers' load stretches it: their streams,
    sockets and tile renders.
  - With the operator alone, 63 % of the cores are busy, while the previews do about 50 ms of
    work a frame, at about 5 frames a second. The rest is the simulator rendering 100 MP frames,
    which the instrument doesn't do, and the scans storing theirs. P10.2 measures the tail on the
    instrument. Tuning the frame path there, such as its threads' priority, is P10.2's.
