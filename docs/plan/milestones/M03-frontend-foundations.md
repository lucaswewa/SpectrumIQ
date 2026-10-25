# M3 — Frontend foundations

**Outcome:** the SpectrumIQ web app, built on `microscope@0640f99`'s look and feel (ADR-0008), is
served by the service at `/`:

- live overview and ROI streams;
- display controls;
- stage, focus and peripheral control.

It runs in a browser and inside the Tauri desktop shell.

**Exit criteria:**

- the app loads from `spectrumiq-server -c config/sim.toml` in Edge/Chrome and in the Tauri shell;
- both themes pass the copied token-contrast tests;
- the live view stays responsive with both streams open;
- `npm run lint && npm run typecheck && npm run test:unit && npm run build` and
  `npm run api:types -- --check` pass in `cargo xtask ci`.

---

### P3.1 — Import the microscope web app and the PrimeVue skin

**Budget** ~800 counted, plus the uncounted import commit · **Depends on** P2.3 · **Requirements** REQ-VIS-005, REQ-NET-002

**Goal.** Start the frontend from `microscope`'s web app, and replace its `reka-ui` widgets with
PrimeVue (unstyled + pass-through) behind the same component API.

> **Amended in P3.1 (ADR-0008):** PrimeVue 5 is no longer MIT, so the owner chose to keep
> `reka-ui` and the copied wrappers as they are. The PrimeVue items below don't apply.

**Scope**

- **Commit A, the import (not counted):** copy from `c:\repos\microscope` at `0640f99`:
  - `web/` configs: `vite.config`, `tsconfig*`, `eslint.config.js`, `.prettierrc.json`,
    `vitest`, `playwright.config.ts`, `.nvmrc`, and `package.json` scripts;
  - `src/theme/`, `src/host/`, `src/api/wot/`, `src/app/`, `src/ui/`, `src/router/`,
    `src/views/dev/`, `src/main.ts`, `App.vue`, and the matching unit tests.

  List every file in the impl notes. Add a `NOTICE` entry. Both projects are MIT and have the same
  author.
- **Commit B, the adaptation (counted):**
  - branding: name, favicon, and an About page placeholder;
  - the NavRail destinations of architecture §11, with icons from `@material-symbols/svg-400`;
  - PrimeVue (unstyled) installed under the two-week rule. The 10 `reka-ui` wrappers are
    reimplemented on PrimeVue with identical props, emits and slots:
    - `AppSelect`, `AppAccordion` + `AccordionSection`, `AppDialog`, `AppMenu`, `AppPagination`,
      `AppSlider`, `AppTooltip`, and `MultiSelect`;
    - `OverlayProvider` becomes PrimeVue `appendTo: 'body'` plus the theme scoping.
  - the PT preset in `src/ui/pt/`, styled only with the tokens;
  - `reka-ui` removed;
  - the ESLint rule `app/ui-components` retargeted to PrimeVue.
- **Server:** serves `web/dist` at `/` and `/assets/…`: `rust-embed` in release builds, disk in
  debug builds, `--webapp-dir`, and a placeholder page when no build exists. This follows
  `microscope`'s ADR-0016 as a reference, implemented in `siq-server`.

**Tests**

- The copied unit tests pass, including token contrast in both themes.
- Component tests for each re-implemented wrapper: same public contract, `v-model`, keyboard
  behaviour.
- An HTTP test that `/` serves the app and that `/api/v1/` is untouched.

**Example.** `/#/dev/components` and `/#/dev/tokens` show every component in both themes.

**ADR.** An amendment to ADR-0008 with the exact file list, the PrimeVue PT approach, and the
wrapper contracts. A new ADR: serving and embedding the web app.

---

### P3.2 — API client: WebSocket observations, connection budget and stores

**Budget** ~800 · **Depends on** P3.1 · **Requirements** REQ-NET-002, REQ-PRV-001

**Goal.** Make the copied TD-driven client fit SpectrumIQ's many Things within the browser's
connection limits (architecture §7.3).

**Scope**

- **Copied client:** API root `/api/v1/`, and `credentials: 'same-origin'` so session cookies flow
  (M9).
- **WebSocket observation transport,** one socket per Thing:
  - property observations and event subscriptions multiplexed over it;
  - the same reconnect backoff as the copied SSE code;
  - fetch-SSE kept as a fallback.
- **`StreamBudget`:** at most 3 concurrent MJPEG (fetch) streams. Streams pause when their view is
  hidden (visibility and IntersectionObserver) and resume automatically.
- **OpenAPI types:** `npm run api:openapi` (`spectrumiq-server --print-openapi`) and
  `npm run api:types`. The generated files are committed.
- **Pinia stores:** `connection`, `camera`, `display`, `stage`, `focus`, `imaging`, and
  `peripherals`. They mirror observed properties. Actions go through invocations, with `ApiError`
  mapped to toasts.

**Tests**

- The WS transport against a mock socket: subscribe, reconnect, resubscribe.
- StreamBudget scheduling.
- Stores with a mocked client.

**Example.** `/#/dev/observe` shows live property values from the simulated server.

**ADR.** Observations over WebSocket and the connection budget. This amends ADR-0010 and records
the deliberate deviation from `microscope`'s ADR-0014.

---

### P3.3 — Live view

**Budget** ~850 · **Depends on** P3.2 · **Requirements** REQ-PRV-001, REQ-PRV-002, REQ-PRV-003

**Goal.** The Live page: overview and native-resolution ROI side by side, with display and camera
controls.

**Scope**

- **Streams:** adapt `microscope`'s `LiveImage` and `MiniStream`. The overview fills the main view.
  The ROI view sits in a split or picture-in-picture layout, user-selectable.
- **ROI:** drag a rectangle on the overview to set `display.roi_region`, with a 1:1 / 2:1 toggle.
- **Display controls**, in a `ControlPane` accordion:
  - auto/manual mode;
  - min/max/gamma sliders;
  - colormap.
- **Histogram:** canvas, linear or log scale, with draggable min/max markers.
- **Camera controls:** exposure, mode presets (full, ROI, binning), start/stop live, and snap.
- **Readouts:** focus measure with a sparkline; lost-frames indicator.

**Tests**

- Histogram ↔ LUT mapping.
- ROI geometry between overview and sensor pixels.
- Component tests with a mocked stream.

**Example.** `/#/dev/live`, with mocked streams and stores.

**ADR.** Live view layout, and the rules for when streams open and close.

---

### P3.4 — Stage, focus and peripheral control

**Budget** ~800 · **Depends on** P3.3 · **Requirements** REQ-STG-001, REQ-STG-002, REQ-SEC-004, REQ-DEV-001

**Goal.** Drive the instrument from the Live page.

**Scope**

- **Adapt `microscope`'s control/ modules:** `DirectionPad`, `jog`, `clickToMove`, `wheel`,
  `PositionSection`, `NavigationSection`.
- **XY:** jog with step presets, plus keyboard shortcuts.
- **Click-to-move:** double-click centres the image on that point, through the calibration affine
  (nominal until M5).
- **Z and focus:** the mouse wheel moves the focus; separate coarse-Z buttons.
- **Readouts and safety:** XYZ + focus readouts; Home; a STOP that is always visible and not
  disabled by busy states.
- **Saved positions:** list, go-to, rename (client-side for now; moved to the server in P6.5).
- **Peripherals panel:**
  - channel selector;
  - illumination intensity;
  - filter;
  - objective, with a confirmation and a parfocal note;
  - modality.

**Tests**

- Jog and wheel mapping.
- Click-to-move maths.
- STOP is reachable in every state.

**Example.** `/#/dev/control`.

**ADR.** Input mapping and safety rules for motion control.

---

### P3.5 — Tauri desktop shell (`apps/spectrumiq-desktop`)

**Budget** ~600 · **Depends on** P3.2 · **Requirements** REQ-VIS-005, REQ-NET-001, REQ-NET-005

**Goal.** A native window around the same web app, with the Tauri `HostAdapter`.

**Scope**

- **Startup:** Tauri 2 shows a bundled splash/connect page, then probes
  `http://127.0.0.1:5000/api/siq/system/health`.
  - In dev builds, it offers to start `spectrumiq-server -c config/sim.toml` as a child process.
  - In production, it expects the Windows service (P15.1).
  - It then navigates the main webview to the server URL.
- **Capabilities:** remote origin `http://127.0.0.1:*` only, for the host commands (app info, save
  dialog, open link, fullscreen, service control).
- **`web/src/host/tauri.ts`:** the `HostAdapter` implementation, chosen in `main.ts` when running
  under Tauri.
- **Window:** single instance; window-state persistence.

**Tests**

- Rust unit tests: health probe and URL logic.
- Web unit tests: host adapter with a mocked `@tauri-apps/api`.

**Example.** A walkthrough in the app's README: `npm run tauri dev` against the simulated server.

**ADR.** The desktop shell loads the UI from the local service, using remote-URL capabilities.
This amends ADR-0002, with `microscope`'s ADR-0033 as a reference.
