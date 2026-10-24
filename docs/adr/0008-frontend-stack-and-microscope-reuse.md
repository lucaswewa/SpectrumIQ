# ADR-0008: Frontend: Vue + PrimeVue (unstyled) skinned with `microscope`'s tokens, copied from `microscope@0640f99`

- **Status:** Accepted
- **Date:** 2026-10-23
- **Phase:** planning
- **Related:** REQ-VIS-005; goals: "Use `microscope` UI and theme (look and feel) as reference
  frontend design"

## Context

`c:\repos\microscope` (MIT, same author, `github.com/lucaswewa/microscope`, at `0640f99`) is a
Rust + teta-wot microscope app. Its Vue 3 web app has a mature look and feel and a set of
conventions.

- **Stack:** Vue 3.5, Vite 8, TypeScript 6, Pinia 4, Vue Router 5 with hash history, OpenSeadragon
  6, Material Symbols icons, Vitest and Playwright.
- **Design tokens** (its ADR-0011): two layers, a magenta accent at hue 326°, Segoe UI system fonts,
  a 4 px grid, compact 40/30 px controls, an 84 px nav rail, 180/400 px control panes, and light
  and dark themes with tested contrast.
- **Components:** its own `src/ui/` components on `reka-ui` (its ADR-0013).
- **Platform isolation:** a `HostAdapter` for Tauri readiness (its ADR-0010).
- **Client:** a TD-driven fetch client (its ADR-0014).

The owner chose **PrimeVue** for its widget coverage (data tables, trees, overlays), styled with
`microscope`'s tokens. Backend code is **not** reused; `microscope`'s ADRs are read as references
only.

## Decision

- **Import and adapt (P3.1).**
  - Commit A copies these from `microscope@0640f99`, unchanged and uncounted in the LOC budget:
    - `src/theme/`, `src/host/`, `src/api/wot/`, `src/app/`, `src/ui/`, `src/router/`,
      `src/views/dev/`;
    - the configs, unit tests, and `DeepZoom`, `LiveImage`, the overlay modules and the
      calibration wizard (when their phases need them).
  - Commit B adapts them.
  - A `NOTICE` entry is added. From then on, SpectrumIQ diverges freely.
- **Components:**
  - PrimeVue in **unstyled mode with pass-through presets** in `src/ui/pt/`, styled only with the
    tokens;
  - the 10 `reka-ui`-based wrappers (Select, Accordion, Dialog, Menu, Pagination, Slider, Tooltip,
    MultiSelect, OverlayProvider) are re-implemented on PrimeVue with **the same props, emits and
    slots**;
  - native-element components stay as copied;
  - `reka-ui` is removed;
  - **only `src/ui/` imports PrimeVue** (ESLint).
- **`microscope`'s web rules are kept:**
  - hash routing;
  - platform APIs only through `src/host/`;
  - tokens only for styles;
  - all assets bundled;
  - Prettier settings;
  - the two-week dependency rule;
  - both themes checked for every UI change.
- **Deviations, each recorded in its phase ADR:**
  - observations over WebSocket instead of fetch-SSE (ADR-0010, P3.2);
  - PrimeVue in place of `reka-ui`.
- **Information architecture.** The nav rail destinations are:
  - top: Live, Mosaic, Scan, Datasets, Viewer, 3D, Jobs;
  - bottom: Calibration, Settings, Admin, Logging, About.

## Alternatives considered

- **Adopt `microscope`'s `reka-ui` stack unchanged:** the closest match to the look, but every
  complex widget would have to be built.
- **A shared npm package used by both apps:** no drift, but every change is coordinated across two
  repositories.
- **Reference only, no copy:** reimplements about 4,000 lines that already work.

## Consequences

- The look and feel match `microscope` from day one.
- PrimeVue widgets need PT styling work, which is concentrated in P3.1 and in each new wrapper.
- The copy will drift from `microscope`. A future shared package stays possible, because the
  wrappers isolate the library.

## Verification

- P3.1: the copied token-contrast tests pass, and the wrapper contract tests pass.
- Galleries at `/#/dev/components` and `/#/dev/tokens`.
- Visual review in both themes.
