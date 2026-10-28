# spectrumiq-desktop

The SpectrumIQ desktop shell: a Tauri 2 window around the same web app that browsers load,
served by the local service (ADR-0040). The service runs on its own (ADR-0002), so closing the
window stops nothing on the instrument.

```text
spectrumiq-desktop [--server http://127.0.0.1:<port>]   the service to load; 5000 by default
SPECTRUMIQ_SERVER=http://127.0.0.1:<port>               the same, when --server isn't given
```

- **It opens on a bundled connect page** (`connect/`). The page probes the service's health
  (`/api/siq/system/health`), then loads the web app from the service.
  - Until the service answers, it says why and tries again every 2 s.
  - **Development builds** offer to start the simulated instrument. That is
    `spectrumiq-server -c config/sim.toml` on the same port, as a child process, killed when the
    window closes.
  - **Release builds** expect the Windows service (P15.1).
- **The service must be on this computer:** `http://127.0.0.1:<port>`, with `localhost` taken as
  127.0.0.1. Only that origin's pages get the shell's commands.
- **The web app's host** (`web/src/host/tauri.ts`):
  - links open in the system's browser;
  - files save through a save dialog;
  - full screen is the window's;
  - in development builds, the service can be started, stopped and restarted;
  - the local console's secret (`localSecret`, ADR-0066, ADR-0068), which logs the desktop in by
    itself: the service names the file to this computer only (`GET /api/siq/auth/local`), and the
    shell reads it if it is called `local-login`, giving back only its 64 hexadecimal digits. A
    page can't make the shell read any other file.
- **One window:** a second launch shows the first window instead. The window's size and place
  are kept between runs.

## Place in the architecture

- **Layer:** App.
- **Depends on:**
  - Tauri 2.12 (WebView2), with the single-instance, window-state, dialog and opener plugins;
  - `tokio`, `url`, `serde_json`, `thiserror` and `percent-encoding`.

  No SpectrumIQ crate: the shell talks to the service over HTTP only.

- **Its parts:**

  | Path                      | What                                                                                      |
  | ------------------------- | ----------------------------------------------------------------------------------------- |
  | `src-tauri/`              | The Rust crate, a workspace member by name, since the `apps/*` glob stops above it        |
  | `src-tauri/src/server.rs` | The service's address, its health probe, and the local secret: HTTP/1.1 over loopback     |
  | `src-tauri/src/child.rs`  | The development builds' simulated instrument                                              |
  | `src-tauri/capabilities/` | `connect`, the bundled page's commands; `remote`, the web app's, for `http://127.0.0.1:*` |
  | `connect/`                | The connect page: HTML, CSS and a script, with no build step                              |
  | `package.json`            | Tauri's command line, `@tauri-apps/cli`                                                   |

- **Phase:** P3.5; P9.4 added the local secret. The fast local paths follow in P14.4 and P14.5,
  and the installer in P15.1.

## Running it

From the repository, with the web app and the server built (the shell starts the release build of
the server if there is one, else the debug build):

```text
cd web && npm ci && npm run build && cd ..
cargo build --release -p spectrumiq-server
cd apps/spectrumiq-desktop
npm ci
npm run tauri dev
```

1. **The window opens on the connect page:** "SpectrumIQ isn't answering at
   http://127.0.0.1:5000/".
2. **Choose "Start the simulated instrument".** The server starts and the page loads the web app
   from it, on the Live page.
3. **Open About:** it says "Running in: The desktop app".
4. **Start a second copy** (`target/debug/spectrumiq-desktop.exe`): it exits, and the first window
   comes forward.
5. **Close the window:** the simulated instrument stops with it. With a server you started
   yourself, closing the window leaves it running.

`--server` points it at a server on another port: `npm run tauri dev -- -- --server
http://127.0.0.1:5078`. To look inside the window, set `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS` to
`--remote-debugging-port=9333`, and open `http://127.0.0.1:9333` in Edge.

## Testing

- `cargo test -p spectrumiq-desktop`:
  - the address's rules;
  - the health probe, against a fake service: healthy, an error, another program, silence and
    nothing;
  - the two capabilities: which pages they cover, and what they grant.
- `npm run test:unit` in `web/`: the host, with `@tauri-apps/api` mocked.
