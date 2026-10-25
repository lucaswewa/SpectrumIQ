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

## Amendment (P3.1, 2026-10-24): Reka UI stays; the files imported

### PrimeVue is not used: its licence changed

- **PrimeVue 5 is no longer MIT.** Its `LICENSE.md` is the "PrimeUI License", and it depends on
  `@primeui/license-manager`:
  - a free Community License, only for organizations under limits of revenue, developers,
    employees and funding;
  - otherwise a paid Commercial License;
  - a licence key in the build;
  - restrictions on redistribution.

  PrimeVue 5.0.2, the newest version old enough for the two-week rule, came out on 29 September
  2026. The policy (conventions §7) allows only permissive licences.
- **The last MIT release is PrimeVue 4.5.5,** from 8 April 2026. The 4.x line has had no release
  since, so it would be a frozen dependency, and moving to 5 later would need the commercial
  licence.
- **The owner chose to keep `reka-ui`** (MIT, maintained, `microscope`'s own stack).
  - The copied wrappers in `src/ui/` stay as they are: `AppSelect`, `AppAccordion` with
    `AccordionSection`, `AppDialog`, `AppMenu`, `AppPagination`, `AppSlider`, `AppTooltip`,
    `MultiSelect` and `OverlayProvider`.
  - Their props, emits and slots are unchanged, and so are their unit tests.
- **The rule becomes "only `src/ui/` imports `reka-ui`",** enforced by ESLint
  (`app/tauri-ready` and `app/ui-components`). It replaces "only `src/ui/` imports `primevue`".
- **Widgets Reka UI lacks** come as new wrappers in `src/ui/`, styled with the tokens:
  - `AppDataTable` (M9) on TanStack Table, which is MIT and headless;
  - `AppTree` on Reka UI's own `Tree`.

  Each is checked under the dependency policy when its phase adds it.

### What was imported (commit A, `6f83351`)

97 files from `microscope@0640f99`, unchanged, and not counted in P3.1's budget. P3.1's notes list
each one:

| Folder | Files |
|---|---|
| `web/` (configs) | 15: `.gitignore`, `.nvmrc`, `.prettierignore`, `.prettierrc.json`, `env.d.ts`, `eslint.config.js`, `index.html`, `package.json`, `playwright.config.ts`, the four `tsconfig*.json`, `vite.config.ts` and `vitest.config.ts` |
| `src/` | `main.ts`, `App.vue` |
| `src/theme/`, `src/host/`, `src/api/wot/`, `src/app/`, `src/router/` | 4, 3, 6, 7 and 1 |
| `src/ui/` | 31 |
| `src/views/dev/` | 2, the galleries |
| `tests/unit/` | 26: those of the modules above, and the app's mount helper. `api/things.spec.ts` is left out: it tests `microscope`'s Thing wrappers |

### What commit B adapted

- **Branding:** the name, the title, the theme's storage key (`spectrumiq.theme`), the favicon
  (Material Symbols `center_focus_strong` in the accent colour), and an About page.
- **The rail:** architecture §11's twelve destinations, each with the Thing it needs. A
  destination whose page isn't built shows a placeholder naming its milestone.
- **Code that wasn't copied:** the connection, calibration, server-UI and page imports came out.
  The connection store and its indicator come back in P3.2.
- **The configs:**
  - the API proxy's variable is `SIQ_API_TARGET`;
  - Playwright starts `spectrumiq-server` on `config/sim.toml` for M9's end-to-end tests;
  - `tsconfig` no longer includes `microscope`'s test folders;
  - the lint messages cite this ADR.
- **The dependencies,** installed from `microscope`'s lockfile, so at the versions its tests ran
  on:
  - `reka-ui`, `vue`, `vue-router`, `pinia` and the icons, bundled;
  - the build and test tools, not bundled.

  `openseadragon` waits for M4, and `openapi-typescript` for P3.2.
- **The tests** that named `microscope`'s destinations (the rail, tab cycling, the app's pages,
  the host's name) now name SpectrumIQ's, and check the same things.
- **Comments in the copied files** that cite "ADR-00NN" mean `microscope`'s ADRs, unless they
  were edited. `web/README.md` says so.
