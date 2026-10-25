# web

SpectrumIQ's web app: one Vue 3 codebase for browsers and the Tauri desktop shell (REQ-VIS-005),
served by `spectrumiq-server` at `/` (ADR-0036). It started from `microscope@0640f99`'s web app and
keeps its look and feel, its components on Reka UI, and its rules (ADR-0008).

## Place in the architecture

- **Layer:** the frontend (architecture §11). It talks to the server only over HTTP: the Things
  under `/api/v1/` and SpectrumIQ's routes under `/api/siq/`.
- **Depends on:** Vue 3.5, Vue Router 5 (hash history), Pinia 4, Reka UI 2.10, and the Material
  Symbols icons. Vite 8, TypeScript 6, Vitest 5, ESLint 10 and Prettier 3 build and check it
  (`docs/soup.md`).
- **Used by:** `spectrumiq-server`, which embeds `dist/` in release builds, and the desktop shell
  (P3.5).
- **Phase:** P3.1; P3.2 added the observation sockets, the stream budget, the stores and the
  API types; P3.3 the Live page; P3.4 its stage, focus and peripheral controls. Every phase of M3
  adds to it.

## Layout

| Folder           | What                                                                                                                                                   |
| ---------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `src/theme/`     | The design tokens (two layers, light and dark), and the theme store                                                                                    |
| `src/host/`      | The `HostAdapter`: the only way to the platform (links, files, full screen)                                                                            |
| `src/api/wot/`   | The TD-driven client: properties, actions as invocations, observations over each Thing's WebSocket (SSE as the fallback), MJPEG over fetch, `ApiError` |
| `src/api/`       | `streams.ts`, the budget of 3 MJPEG streams; `generated/`, the server's OpenAPI document and its types                                                 |
| `src/stores/`    | Pinia stores: `connection`, and `camera`, `display`, `stage`, `focus`, `imaging`, `peripherals`, mirroring their Things                                |
| `src/app/`       | The shell: the navigation rail, keyboard shortcuts, and the placeholder of pages not built yet                                                         |
| `src/ui/`        | The components: native elements, and wrappers of Reka UI. Only this folder imports `reka-ui`                                                           |
| `src/router/`    | One hash route per destination of the rail, and the development galleries                                                                              |
| `src/live/`      | The Live page: the overview and the ROI streams, the ROI dragged on the overview, the display and camera controls, the histogram, the focus readout    |
| `src/control/`   | The stage, focus and peripherals on the Live page: the keys and the d-pad, the wheel, click-to-move, STOP, Home, the steps, saved positions (ADR-0039) |
| `src/system/`    | The About page                                                                                                                                         |
| `src/views/dev/` | The galleries, `/#/dev/components` and `/#/dev/tokens`, and `/#/dev/observe`: in development builds only                                               |

The rail lists architecture §11's destinations. Each shows once the instrument serves the Thing it
needs. Until its page is built, a destination says which milestone builds it.

## Running it

Against the simulated instrument:

```text
cargo run --release -p spectrumiq-server -- -c config/sim.toml
cd web
npm ci
npm run dev
```

1. Vite serves the app at <http://localhost:5173/> and forwards `/api` to the server.
   `SIQ_API_TARGET` points it elsewhere, such as `http://lab-pc:5000`.
2. Open `/#/dev/components` and `/#/dev/tokens`. Every component and token is shown in both
   themes, side by side.
3. Open `/#/dev/live`: the Live page against a fake camera drawn in the browser, which needs no
   server. With the server, `/#/live` is the real one: start live view, drag on the overview to
   choose the ROI, and move the histogram's markers.
4. Open `/#/dev/control`: the same page against a fake stage, focus and peripherals, with the
   commands the controls send listed beside it. Tap or hold the arrow keys and PgUp/PgDn, turn the
   wheel on the image, double-click the overview, and press Esc or STOP. With the server, `/#/live`
   drives the simulated stage.
5. Open `/#/dev/observe`: every store's values, live. Move the stage with its buttons, and watch
   its position follow. The browser's developer tools show one WebSocket per Thing, and no
   event stream.
6. Switch the theme with the system's setting, or with `localStorage['spectrumiq.theme']`: `light`,
   `dark` or `system`.

To see the app as the server serves it, build it (`npm run build`). Then either start the server
with `--webapp-dir web/dist`, or build a release server, which embeds `dist/`, and open
<http://127.0.0.1:5000/>.

## Checks

| Script                 | What                                                                                                                        |
| ---------------------- | --------------------------------------------------------------------------------------------------------------------------- |
| `npm run format:check` | Prettier: no semicolons, single quotes, 100 columns                                                                         |
| `npm run lint`         | ESLint, warnings denied, with the app's rules: hash routing, platform only through `src/host/`, `reka-ui` only in `src/ui/` |
| `npm run typecheck`    | `vue-tsc`, strict                                                                                                           |
| `npm run test:unit`    | Vitest, in happy-dom: `tests/unit/**/*.spec.ts`                                                                             |
| `npm run build`        | The production build in `dist/`, with `third-party-licences.txt`                                                            |
| `npm run api:openapi`  | Writes the server's OpenAPI document to `src/api/generated/openapi.json` (needs Cargo)                                      |
| `npm run api:types`    | Writes its types to `src/api/generated/schema.d.ts`; with `-- --check`, checks they are current                             |

`cargo xtask ci` runs the first five, and `api:types -- --check`. A server test fails when
`openapi.json` is older than the server: then run `api:openapi` and `api:types`. The end-to-end tests (`npm run test:e2e`, Playwright) start with M9.

## Notes

- **Copied code:** the files imported from `microscope` are listed in `docs/impl-notes/P3.1.md`.
  In their comments, "ADR-00NN" means `microscope`'s ADRs (github.com/lucaswewa/microscope,
  `docs/adr/`), unless the comment has been edited for SpectrumIQ.
- **Dependencies** are installed only at versions published at least two weeks earlier:
  `npm install --before=<date>`.
