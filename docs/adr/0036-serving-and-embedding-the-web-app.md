# ADR-0036: Serving and embedding the web app

- **Status:** Accepted
- **Date:** 2026-10-24
- **Phase:** P3.1
- **Related:** ADR-0003, ADR-0008, REQ-VIS-005, REQ-NET-001; `microscope`'s ADR-0016, as a
  reference

## Context

- **The web app** (ADR-0008) builds with Vite into `web/dist`:
  - an `index.html`;
  - content-hashed files under `assets/`;
  - a favicon;
  - `third-party-licences.txt`.

  It routes in the URL's hash, so the server needs `/` and never a route per page.
- **One program should be enough** to use the instrument: the server, serving the API and the app.
  The Tauri shell (P3.5) loads the app from that server.
- **Development keeps Vite's own server,** which forwards `/api` to a running server.
- **SpectrumIQ's routes come before teta-wot's** (ADR-0003, amended), with no change to teta-wot.
- **`microscope` solved the same problem** (its ADR-0016). Its backend is a reference only
  (ADR-0008), so this is written anew, for `siq-server`.

## Decision

- **`siq_server::webapp` serves the app,** merged with SpectrumIQ's routes in front of teta-wot's:

  | Route | What | `Cache-Control` |
  |---|---|---|
  | `/`, `/index.html` | The app's page, or a placeholder when there is no build | `no-store` |
  | `/assets/{*path}` | The hashed scripts, styles and fonts | `public, max-age=31536000, immutable` |
  | `/favicon.svg`, `/third-party-licences.txt` | The icon, and the bundled npm packages' licences | `no-cache` |

  - A missing file answers 404 `{"detail": "Not Found"}`, as teta-wot does.
  - Any other path is still teta-wot's: `/api/v1/…`, `/docs`, `/openapi.json` and
    `/.well-known/wot`.
- **The files come from `web/dist`, through `rust-embed`:**
  - **release builds embed it,** so the binary is self-contained;
  - **debug builds read it from disk,** so `cargo run` serves the latest web build;
  - **`spectrumiq-server --webapp-dir <folder>`** serves another build (`WebApp::Dir`).
  - `Service::with_webapp` chooses the source; `WebApp::Embedded` is the default.
- **Without a build,** `/` shows a placeholder page. It says how to build the app, and links to
  the API and `/docs`. The server compiles and runs either way, which is what Rust CI needs.
- **Build order:** the web app first (`npm run build`), then a release server.
  - `siq-server`'s build script tells Cargo to rebuild it when `web/dist` changes.
  - `cargo xtask ci` builds the web app after the Rust checks; its tests don't depend on a build.
- **Paths that could leave the build's folder are refused.** Only plain components are accepted:
  no `..`, no root, no drive.
- **`spectrumiq-server --port`** overrides the configuration's port, for the end-to-end tests
  (M9) and for running beside another server.
- **The dependency:** `rust-embed` 8.13.0 (MIT, 7 October 2026). The crates it brings are each
  MIT, or offered under MIT, Apache-2.0 or the Unlicense. `sha2` is pinned to 0.11.0, because
  0.11.1 is too new for the two-week rule (`docs/soup.md`).

## Alternatives considered

- **Shipping `web/dist` beside the binary:** two things to install and keep in step.
  `--webapp-dir` still allows it.
- **`include_dir`, or `include_bytes!` from a build script:** no reading from disk in debug builds.
- **`tower-http`'s `ServeDir`:** reads from disk only, so it can't serve an embedded build.
- **Serving from a sub-path such as `/app/`:** the root is what people type, and the API already
  has its own prefix.

## Consequences

- One binary serves everything, and the app it serves is the one it was built with.
- A release server built before the web app shows the placeholder, which says so.
- While `web/dist` is missing, the build script runs on every build, cheaply.
- The release binary grows by the app's size: about 330 KB today.

## Verification

`siq-server` `webapp` tests, from a build in a temporary folder:

- `the_app_is_served_at_the_root_with_its_assets`, with each route's media type and caching;
- `a_path_that_would_leave_the_build_is_refused`;
- `without_a_build_the_root_says_how_to_build_one`;
- `the_api_is_untouched`: `/api/v1/things/`, `/api/siq/system/health`, and a 404 from teta-wot.

By hand: the release server with the embedded build, in Edge, in both themes.
