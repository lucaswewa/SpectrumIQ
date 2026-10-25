# ADR-0040: The desktop shell loads the web app from the local service

- **Status:** Accepted
- **Date:** 2026-10-24
- **Phase:** P3.5
- **Related:** ADR-0002 (amended), ADR-0008, ADR-0036, REQ-VIS-005, REQ-NET-001, REQ-NET-005;
  `microscope`'s ADR-0033

## Context

- **ADR-0002 makes the desktop app a shell around the web app** that the service serves. The
  service owns the instrument, so closing the window must stop nothing.
- **The service already serves the web app at `/`** (ADR-0036), and every server URL comes from
  the connection (ADR-0008).
- **Tauri 2 can load a remote page, and give it IPC.** The page gets IPC only through a capability
  whose `remote.urls` match its origin. Bundled pages get their own capabilities.
- **The host services** the web app needs from a desktop are behind `HostAdapter` (ADR-0008):
  - app information;
  - saving a file;
  - opening a link;
  - full screen;
  - control of a local service.
- **`microscope`'s ADR-0033** started its server as a supervised child, with a launcher that the
  Tauri app was to replace. SpectrumIQ's production service is a Windows service instead (P15.1).

## Decision

- **The shell bundles only a connect page** (`apps/spectrumiq-desktop/connect/`).
  - The page probes `http://127.0.0.1:<port>/api/siq/system/health`, and once the service answers,
    navigates the window to `http://127.0.0.1:<port>/`.
  - Until then it says why, and tries again every 2 s.
  - The port is 5000, or `--server` or `SPECTRUMIQ_SERVER`.
  - The probe is one HTTP/1.1 request over loopback, which needs no HTTP client.
- **The service must be on this computer:** plain HTTP to 127.0.0.1 on a given port, with
  `localhost` taken as 127.0.0.1. A remote instrument is reached from a browser instead.
- **Two capabilities, both for the main window:**
  - **`connect`,** for the bundled page: `app_info`, `probe` and `service_start`, and Tauri's
    defaults;
  - **`remote`,** for `http://127.0.0.1:*` only (Tauri fills an empty path with `*`):
    - the host commands: `app_info`, `open_link`, `save_file`, and the `service_*` commands;
    - the window's full-screen commands;
    - listening to events.

    Nothing else: not the probe, and not Tauri's core set. The web app can't close, move or
    navigate the window by IPC.
  - **The commands are the app's own,** each a permission through `tauri-build`'s app manifest.
- **The web app chooses its host at start** (`createHost`):
  - inside the shell (`__TAURI_INTERNALS__`), it is `web/src/host/tauri.ts`, loaded in a chunk of
    its own;
  - in a browser, it is the browser host.
  - If the shell refuses its commands, the app runs as in a browser.
- **The host commands:**
  - **links** open in the system's browser: web and mail links only;
  - **a file is saved** through a save dialog. Its bytes go as the request's raw body, not as
    JSON, with its name URI-encoded in a header;
  - **full screen** is the window's, followed through its resize events;
  - **the service:** see below.
- **The service in development builds:** the shell can start the simulated instrument,
  `spectrumiq-server -c config/sim.toml` on the shell's port.
  - It runs from the repository's `target/`: the release build if there is one, else the debug
    build.
  - It runs as a child process, and is killed when the window closes.
  - The web app's `host.service` controls it: state, start, stop, restart.
- **The service in release builds:** the shell manages nothing, and expects the Windows service
  (P15.1).
- **One window:** a second launch shows the first (`tauri-plugin-single-instance`).
- **The window's size and place are kept** (`tauri-plugin-window-state`).
- **Image data never crosses the IPC** (ADR-0002): the web app reads streams and tiles from the
  service over HTTP, as in a browser.

## Alternatives considered

- **Bundling the web app in the shell, and pointing it at the service:**
  - the shell and the service could drift apart in version;
  - the app would need a connection profile before its first page;
  - the custom protocol's origin would need CORS on the service.

  Loading the service's own app keeps one version per instrument.
- **`microscope`'s supervised sidecar in production:** a child of the window would stop with it.
  ADR-0002 rules that out.
- **Tauri's core permissions for the remote page** (`core:default`): they give the page the
  window's controls, which it doesn't need.
- **An HTTP client crate for the probe:** a single request on loopback doesn't need one.
- **A remote capability for every origin:** any page the window was led to would get the host
  commands.

## Consequences

- **The desktop and browsers run the same build of the web app:** the one the service serves.
- **A service on another computer gets no host commands,** and the shell won't load it. Browsers
  remain the remote client.
- **A development server outlives a shell that is killed:** it is killed only when the window
  closes.
- **The shell needs WebView2**, which Windows 11 has. The installer is P15.1's.
- **The fast local paths** (ADR-0010; P14.4, P14.5) will be further host commands, each in the
  `remote` capability.

## Verification

- `cargo test -p spectrumiq-desktop`:
  - the address's rules, and where it comes from;
  - the health probe, against a fake service: healthy, an error status, another program, silence
    and nothing listening;
  - `tests/capabilities.rs`: the remote pattern matches the local service's pages, and nothing
    else; the web app's grants and the connect page's.
- `web/tests/unit/host/tauri.spec.ts`, with `@tauri-apps/api` mocked:
  - the app's info, links, saving, full screen;
  - the service, only where the shell manages one;
  - the host chosen inside the shell, and the browser's when it refuses.
- **By hand,** on the desktop, with the development build, over WebView2's DevTools protocol:
  - the connect page, then the simulated instrument started from it;
  - the web app from `http://127.0.0.1:5000/`;
  - `app_info` and `service_state` allowed from its origin, and `probe` and `window.close` refused;
  - About saying "The desktop app";
  - a second launch exiting.
