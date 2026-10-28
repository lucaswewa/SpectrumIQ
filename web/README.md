# web

SpectrumIQ's web app: one Vue 3 codebase for browsers and the Tauri desktop shell (REQ-VIS-005),
served by `spectrumiq-server` at `/` (ADR-0036). It started from `microscope@0640f99`'s web app and
keeps its look and feel, its components on Reka UI, and its rules (ADR-0008).

## Place in the architecture

- **Layer:** the frontend (architecture §11). It talks to the server only over HTTP: the Things
  under `/api/v1/` and SpectrumIQ's routes under `/api/siq/`.
- **Depends on:** Vue 3.5, Vue Router 5 (hash history), Pinia 4, Reka UI 2.10, TanStack Table 9.3
  (headless, for `AppDataTable`), the Material Symbols icons, and `@tauri-apps/api` 2.12 in the
  desktop shell only. Vite 8, TypeScript 6, Vitest 5, ESLint 10 and Prettier 3 build and check it
  (`docs/soup.md`).
- **Used by:** `spectrumiq-server`, which embeds `dist/` in release builds and serves it to
  browsers and to the [desktop shell](../apps/spectrumiq-desktop/README.md) (ADR-0040).
- **Phase:** P3.1; P3.2 added the observation sockets, the stream budget, the stores and the
  API types; P3.3 the Live page; P3.4 its stage, focus and peripheral controls; P3.5 the desktop
  shell's host; P4.8 the Datasets and Viewer pages; P5.4 the Calibration page and its wizard;
  P6.6 the Scan page, and the saved positions on the server; P6.7 the run panel and sequences;
  P7.4 the Mosaic page, and the stage map over the live mosaic; P8.4 the Jobs page, Stitch on the
  Datasets page, and a stitched image's QA overlays in the viewer; P9.4 the login, the controls
  disabled by role and token with the reason, the control token bar, and the Admin page; P9.5 the
  end-to-end tests.

## Layout

| Folder             | What                                                                                                                                                                                                           |
| ------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `src/theme/`       | The design tokens (two layers, light and dark), and the theme store                                                                                                                                            |
| `src/host/`        | The `HostAdapter`: the only way to the platform (links, files, full screen). The browser's, or the desktop shell's (`tauri.ts`, ADR-0040)                                                                      |
| `src/auth/`        | The session (ADR-0068): the login, asked for only where the server enforces one; the desktop's login by the local secret; what each role may do, and why not                                                   |
| `src/api/wot/`     | The TD-driven client: properties, actions as invocations, observations over each Thing's WebSocket (SSE as the fallback), MJPEG over fetch, `ApiError`                                                         |
| `src/api/`         | `streams.ts`, the budget of 3 MJPEG streams; `generated/`, the server's OpenAPI document and its types                                                                                                         |
| `src/stores/`      | Pinia stores: `connection`, and `camera`, `display`, `stage`, `focus`, `imaging`, `peripherals`, `datasets`, `calibration`, `workflow`, `mosaic`, `jobs`, mirroring their Things                               |
| `src/app/`         | The shell: the navigation rail, keyboard shortcuts, and the placeholder of pages not built yet                                                                                                                 |
| `src/ui/`          | The components: native elements, and wrappers of Reka UI. Only this folder imports `reka-ui`. `AppDataTable`, on TanStack Table; `GatedFields`, controls disabled with the reason                              |
| `src/router/`      | One hash route per destination of the rail, and the development galleries                                                                                                                                      |
| `src/live/`        | The Live page: the overview and the ROI streams, the ROI dragged on the overview, the display and camera controls, the histogram, the focus readout                                                            |
| `src/control/`     | The stage, focus and peripherals on the Live page: the keys and the d-pad, the wheel, click-to-move, STOP, Home, the steps, saved positions (ADR-0039), kept by the server; the control token's bar (ADR-0068) |
| `src/datasets/`    | The Datasets page: cards with thumbnails, filtered by kind and name, a dataset's manifest and history beside them, its exports, Stitch… with its options, and deleting it                                      |
| `src/viewer2d/`    | The Viewer page: OpenSeadragon with an additive tiled image per channel, the channels' LUT store and controls, z and t, the scale bar, the probe                                                               |
| `src/overlay/`     | Overlay layers drawn over an image, from `microscope`, and the Layers menu: used by the viewer, filled by M8                                                                                                   |
| `src/calibration/` | The Calibration page, and the wizard from `microscope` (ADR-0051): its tasks per objective and channel, the first-run prompt, and the gating rules                                                             |
| `src/scan/`        | The Scan page (ADR-0057): the stage map and its geometry, the plan's form and checks, the server's estimate, and Start; the sequence form; the run panel and its thumbnails                                    |
| `src/mosaic/`      | The Mosaic page (ADR-0060): the live mosaic panned and zoomed, the camera's field, the minimap, double-click to move; the backdrop under it and the stage map; the viewport maths                              |
| `src/jobs/`        | The Jobs page: the jobs running, with their progress, and waiting, each with Cancel; auto-stitch                                                                                                               |
| `src/admin/`       | The Admin page: the users, added, given a role or a password, and removed; the audit trail, filtered by who and what, a page at a time                                                                         |
|  The About page    |
| `src/views/dev/`   | The galleries, `/#/dev/components` and `/#/dev/tokens`, and `/#/dev/observe`: in development builds only                                                                                                       |
| `e2e/`             | The end-to-end tests (ADR-0069), in Playwright against the served app on a fresh simulated instrument                                                                                                          |

The rail lists architecture §11's destinations. Each shows once the instrument serves the Thing it
needs. Until its page is built, a destination says which milestone builds it.

## Running it

Against the simulated instrument, which asks for a login (ADR-0065): add a user first, an admin
while there is none.

```text
cargo run --release -p spectrumiq-server -- user add ada -c config/sim.toml
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
6. Open `/#/dev/viewer2d`, with the server: the viewer on one of its datasets, with a demo overlay
   layer of the image's 512-pixel chunks under Layers. Make a synthetic dataset first, such as with
   `cargo run --release -p siq-jobs --example export_ometiff -- --out <the data root>`.
7. Open `/#/dev/calibration`: the Calibration page and its wizard against a fake instrument, which
   needs no server. The 10× in place lacks its calibrations, so the wizard opens for it; each
   calibration takes a second or two. With the server, `/#/calibration` is the real one.
8. Open `/#/dev/scan`: the Scan page against a fake instrument, the 20× in place. Draw a region with
   Rectangle or Polygon, pick a focus map's places with Places, and watch the estimate follow.
   With the server, `/#/scan` plans on the simulated stage, calibrated first on `/#/calibration`.
9. Open `/#/dev/run`: the same page, with runs that take a step every half second. Start a scan
   or a sequence, then pause, resume and cancel it; with On a failure set to "Wait, and ask", it
   waits at its fourth step for Retry or Skip.
10. Open `/#/dev/mosaic`: the Mosaic page against a fake instrument, which paints a field wherever
    its stage stands still. Double-click to move the stage, drag and zoom; the fields stay where
    they were painted. `/#/dev/scan`'s stage map then shows them too. With the server, `/#/mosaic`
    paints the simulated instrument's frames, once the stage–camera calibration is in force.
11. Open `/#/dev/admin`: the Admin page, the control token's bar and the Live page against fakes,
    which need no server. Choose who is logged in, an admin, an operator or a viewer, to see what
    each may do and why the rest is disabled; take control, and the Live page's controls come on.
    With the server, `/#/admin` is the real one, and a second browser logged in as another user
    sees who has control.
12. Switch the theme with the system's setting, or with `localStorage['spectrumiq.theme']`: `light`,
    `dark` or `system`.

To see the app as the server serves it, build it (`npm run build`). Then either start the server
with `--webapp-dir web/dist`, or build a release server, which embeds `dist/`, and open
<http://127.0.0.1:5000/>.

## Checks

| Script                 | What                                                                                                                                            |
| ---------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------- |
| `npm run format:check` | Prettier: no semicolons, single quotes, 100 columns                                                                                             |
| `npm run lint`         | ESLint, warnings denied, with the app's rules: hash routing, platform and `@tauri-apps/*` only through `src/host/`, `reka-ui` only in `src/ui/` |
| `npm run typecheck`    | `vue-tsc`, strict                                                                                                                               |
| `npm run test:unit`    | Vitest, in happy-dom: `tests/unit/**/*.spec.ts`                                                                                                 |
| `npm run test:e2e`     | Playwright, in the installed Edge, against a debug server of this checkout on a fresh simulated instrument: `e2e/**` (ADR-0069). Opt-in         |
| `npm run build`        | The production build in `dist/`, with `third-party-licences.txt`                                                                                |
| `npm run api:openapi`  | Writes the server's OpenAPI document to `src/api/generated/openapi.json` (needs Cargo)                                                          |
| `npm run api:types`    | Writes its types to `src/api/generated/schema.d.ts`; with `-- --check`, checks they are current                                                 |

`cargo xtask ci` runs the first five, and `api:types -- --check`. A server test fails when
`openapi.json` is older than the server: then run `api:openapi` and `api:types`.

`cargo xtask ci --e2e` runs the end-to-end tests too. They build the web app and a debug server,
and start it on port 5098 with a copy of `config/sim.toml` in the system's temporary folder: a
2048 × 1536 camera, rendered fast, and a 15 s token timeout. A setup adds the users (`ada`, an
admin, with the CLI; `olga` and `otto`, operators, and `vic`, a viewer, over HTTP), calibrates,
so that the wizard stays shut, and snaps a dataset to view. The tests run one at a time, each
with the token free and live view stopped:

- `login`: a wrong password refused, a viewer's controls disabled with the reason, a logout;
- `live`: an operator taking control, and both previews streaming;
- `handover`: two browsers, an admin taking over an operator's control, and a holder gone silent
  losing it after the timeout;
- `budget`: the connection budget (ADR-0010): one browser with the 3 MJPEG streams and the
  Things' sockets, while the viewer loads its tiles over the connections left.

They use the installed Edge (`channel: 'msedge'`), so no browser is downloaded.

## Notes

- **Copied code:** the files imported from `microscope` are listed in `docs/impl-notes/P3.1.md`.
  In their comments, "ADR-00NN" means `microscope`'s ADRs (github.com/lucaswewa/microscope,
  `docs/adr/`), unless the comment has been edited for SpectrumIQ.
- **Dependencies** are installed only at versions published at least two weeks earlier:
  `npm install --before=<date>`.
