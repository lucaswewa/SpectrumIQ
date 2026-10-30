# siq-server

SpectrumIQ's service core. It makes the instrument's devices from the configuration, starts the
engines, and serves the instrument's Things on teta-wot, with SpectrumIQ's own routes in front.

- **Devices,** simulated or real, chosen per device (ADR-0034): the camera, the GRBL stage, the
  ASCII focus drive, and the fitted peripherals (simulated until P10.5).
- **Engines:** the acquisition (`siq-acquire`), the previews (`siq-preview`) and the jobs
  (`siq-jobs`), shared by the Things as the `Instrument` service, with the calibrations
  (`siq-calib`'s registry, in the settings folder).
- **Things,** under `/api/v1/` (ADR-0033):
  - `camera`, `display`, `stage` and `focus`;
  - the fitted peripherals: `illumination`, `filter_wheel`, `objective` and `modality`;
  - `imaging`, the channels (ADR-0035);
  - `jobs`, the background jobs (ADR-0043);
  - `datasets`, the datasets, a snap made into one, and their exports (ADR-0041, amended);
  - `calibration`, the stage–camera calibration of the objective in use (ADR-0049), and the dark
    and flat of the objective and channel in use (ADR-0050);
  - `workflow`, plans run by `siq-workflow`'s engine, and the saved positions (ADR-0056);
  - `mosaic_live`, the live mosaic painted as the stage moves, and the client's view of it
    streamed (ADR-0059, amended).

  The global lock is on. The `halt` actions, `jobs.cancel`, the `datasets` actions other than
  `snap_to_dataset`, the `workflow` actions other than its runs, and `mosaic_live` bypass it. A
  run holds it throughout.
- **Routes,** under `/api/siq/system/`: `health`, `metrics` and `memory`; under
  `/api/siq/auth/`, `login`, `logout`, `me` and `local` (ADR-0065); under `/api/siq/users`, an
  admin's management of the users (ADR-0068); under `/api/siq/data/`, a dataset's manifest and
  thumbnail, and its images' tiles. teta-wot's router is the fallback (ADR-0003,
  amended).
- **The session layer** (ADR-0065, ADR-0066), around every route, teta-wot's included:
  - a request other than `GET`, `HEAD` or `OPTIONS` whose `Origin` isn't the service's own, or
    one of `[auth] allowed_origins`, is refused with 403;
  - a request with a live session's cookie carries its `siq_auth::Session`, its role read again
    from the users, and is the control token's heartbeat if the session holds it;
  - where `[auth] enforce`, `authz` classifies the request and refuses what the caller may not
    do: 401 without a session, 403 for a role or the missing token.

  The service is served with each connection's address, which the login's limit and the local
  login use.
- **The audit trail** (ADR-0067), in `[server] audit`: the layer records every change, with its
  parameters and its answer, and every refusal, but the logins' own requests, the token's
  heartbeat, the live mosaic's viewport, and a read, though it be a `POST` (by its class); the logins and logouts, the token's changes, every
  job's transitions, every workflow run's start and end, and every dataset deleted are recorded
  where they happen. `GET /api/siq/audit` is an admin's query of it.
- **Who may do what** (`authz`, ADR-0066): `classify(method, path)` gives every route a class,
  public, read, stop, processing, control, hardware or admin, and `decide` the matrix of
  architecture §10. A change to a route without a class is refused.
- **The web app** at `/`, `/assets/…`, `/favicon.svg` and `/third-party-licences.txt` (ADR-0036):
  - `web/dist` is embedded in release builds, and read from disk in debug builds;
  - `Service::with_webapp(WebApp::Dir(folder))` serves another build;
  - without a build, a placeholder page.
- **The lifecycle:** start the Things, serve, and on shutdown stop live view and motion, and
  cancel the jobs and wait for them, before the Things.

This is the only crate that depends on teta-wot.

## Place in the architecture

- **Layer:** L5 Service.
- **Depends on:**
  - teta-wot v0.1.0 (git), and `tower` for `oneshot`;
  - `siq-acquire`, `siq-preview`, `siq-jobs`, `siq-workflow`, `siq-pyramid`, `siq-storage`,
    `siq-calib`, `siq-register`, `siq-stitch`, `siq-stitch-live`, `siq-gpu`;
  - `siq-devices`, `siq-grbl`, `siq-ascii`, `siq-serial`, `siq-sim`;
  - `siq-auth`, `siq-config`, `siq-telemetry`, `siq-buffer`, `siq-imgproc`, `siq-synth`,
    `siq-types`;
  - `tokio`, `serde`, `schemars`, `chrono`, `anyhow`, `tracing`, `jpeg-encoder`.
- **Used by:** `apps/spectrumiq-server`, the service binary.
- **Phase:** P2.3; P2.4 added the peripherals, the channels, and the metrics and memory routes;
  P3.1 the web app; P4.3 the `jobs` Thing; P4.5 the data routes, with the catalog and the tile
  cache; P4.7 the `datasets` Thing, a dataset's manifest and thumbnail; P4.8 an image's metadata;
  P5.2 the `calibration` Thing, and the frames' objective and calibration; P5.3 the darks and
  flats, and the previews' correction; P6.5 the `workflow` Thing; P7.3 the `mosaic_live` Thing,
  and the GPU of `[gpu]`; P8.4 stitching: `jobs.stitch`, auto-stitch, and the QA overlays; P9.1
  the sessions: the auth routes and the layer; P9.2 the authorization, the control token and its
  Thing, and the local console's login; P9.3 the audit trail and its query; P9.4 the users
  routes, the local console's user made at start, and where its secret is; P10.1 the Euresys
  camera, with `euresys`; P10.2 the pool checked against the RAM at start; P10.6 colour: the
  histogram per colour, `white_from_roi`, and a snap's image keeping its filter array; P10.7 a
  colour frame's luminance for the live mosaic.

## API

| Item | What |
|---|---|
| `Service::build(config)` | Opens the devices, starts the acquisition and previews, and makes the Things |
| `service.serve(listener, shutdown)` | Serves until `shutdown` completes, then shuts down |
| `service.router()` | Every route: SpectrumIQ's and the web app's first, teta-wot's after |
| `service.with_webapp(source)`, `webapp::WebApp` | Where the web app comes from: `Embedded` (the default) or `Dir(folder)` |
| `service.instrument()`, `service.into_parts()` | The `Instrument` (devices and engines), and the Things' server apart, for tests |
| `openapi()` | The Things' OpenAPI document, made without devices |
| `devices::open(config, clock)` | The devices alone, with the fitted `Peripherals` |
| `shutdown_signal()` | Ctrl-C, or the system asking the process to stop |
| `API_PREFIX` | `/api/v1` |
| `instrument.on_dataset_complete(hook)` | Tells a `DatasetHook` of every workflow's dataset from then on: `on_dataset_complete(&instrument, &report)`, however its run ended, before the run's action answers. M8 queues stitching so |
| `workflow::{Steps, Requested, PlanRequest, TileScanRequest, ZStackRequest, MultiPositionRequest, TimeLapseRequest, FocusMapRequest}` | The runs' inputs: each makes its plan and checks it as it is read |

### The Things

| Thing | Properties | Actions | Events and streams |
|---|---|---|---|
| `camera` | `exposure_ms`, `region`, `binning`, `pixel_format` (writable); `frame_rate`, `sensor`, `stats` | `start_live`, `stop_live`, `snap` → JPEG | `frames_lost`; MJPEG `overview` and `roi` |
| `display` | `lut` (writable), with a colour frame's `white_balance`, `auto` or `fixed`; `roi_region`, `flat_field` (writable): the previews corrected by the dark and flat in force; `histogram`, a colour frame's with its `colours`, red, green and blue, its `bins` and `window` being green's; `focus_measure`, of a colour frame's luminance | `auto_contrast_once`; `white_from_roi`: a colour frame's white balance fixed at the ROI's latest, red's and blue's 99.9th percentiles brought to green's (ADR-0071) | — |
| `stage` | `position`, `state`, `homed` (observable); `speed`; `limits` | `move_to`, `move_by`, `jog`, `home`, `halt` | `arrived`, `alarm` |
| `focus` | `position`, `state`, `homed` (observable); `limits` | `move_to`, `move_by`, `halt` | `arrived` |
| `illumination` | `levels` (observable); `sources` | `set_level(source, level)`, by the source's name | `changed` |
| `filter_wheel` | `position`, `moving` (observable); `filters` | `select(position)` | `changed` |
| `objective` | `position`, `moving` (observable); `objectives`, `current` | `select(position)`, which also sets the frames' objective | `changed` |
| `modality` | `modality`, `moving` (observable); `modes`, `patterns` | `set_modality(mode, pattern?)` | `changed` |
| `imaging` | `channel` (observable); `channels` | `select_channel(id)`: every light off, the filter, the exposure and colour, the light, then the frames' channel | `channel_changed` |
| `jobs` | `queue`, `running` (observable): each job's id, kind, priority, state, progress, estimate and times; `auto_stitch` (writable, on at start): a finished tile scan is stitched by itself into `stitched` | `cancel(id)`; `stitch(dataset, image?, options?)`: the dataset stitched into `image` (`stitched`) as a job, by the calibration it keeps and each channel's dark and flat in force: the job's id (ADR-0064) | — |
| `calibration` | `status`: each objective, with its calibration in force or `null`, and its dark and flat with each channel, and with none | `stage_camera`: the objective in use calibrated against the stage, with the stage's backlash and repeatability, kept as a new version, and stamped on its frames from then on; a snap's dataset keeps a copy, and its pixel size. `capture_dark(frames?)`: the objective and channel's dark, every light off, of the whole sensor. `capture_flat(defocus_um?, frames?)`: their flat, at nine places, out of focus or of a blank slide, less the dark. `estimate_shading(dataset, dark_level?)`: a dataset's tiles' flat, as a background job | — |
| `workflow` | `state` (`idle`, `running`, `paused`, `asking`), `progress`, `problem` (observable); `positions`; `field`, the frame's corners about where the stage is; `plan_schema` | `run(name, steps, on_error?)`, any plan; `tile_scan(name, region, overlap?, order?, channels?, settle_ms?, focus?, z_stack?, focus_map?)`; `z_stack(name, planes, steps?)`; `multi_position(name, positions?, autofocus?, steps?)`, the saved positions if none; `time_lapse(name, interval_s, count?, duration_s?, overrun?, steps?)`. Each holds the global lock until its run ends, answers `{name, dataset, outcome, error, steps, frames}`, and is cancelled by a `DELETE` of its invocation; a plan that couldn't run is a 422. `estimate_tile_scan`, with `tile_scan`'s input: its tiles, a frame's corners, frames, time, bytes, the disk's room, and warnings. Every run takes `on_error`; one whose policy asks waits on a failed step for `retry` or `skip`. `pause`, `resume`, `save_position(name, x?, y?, z?, focus?)`, `rename_position(name, to)`, `remove_position(name)`, without the lock | `tile_acquired`, `finished` |
| `mosaic_live` | `enabled`, `viewport` (`centre` in µm, `zoom` in view pixels a canvas pixel, `width` and `height` up to 1920 × 1080), `canvas_scale` (½), `max_uncertainty_px` (2) (writable); `bounds` in µm, `px_per_um`, `stats` (frames painted and passed over, the last paint's time, views, pages and their GPU memory against the budget) | `clear`; `save_snapshot(name?)`: what was painted, as the OME-Zarr image `mosaic` of a new dataset of kind `mosaic`, its pyramid submitted | MJPEG `view`, the viewport rendered |
| `datasets` | — | `summary(offset, limit)`: a page of the datasets, newest first; `snap_to_dataset(channels?, name?)`: a frame per channel, recorded raw, written as the image `snap` by a job, and its pyramid submitted; `export_ome_tiff(dataset, image)` and `export_dzi(dataset, image, c, z, t, format)`, as background jobs; `delete(dataset)`, refused while a job works on it | `created` |

The peripherals' Things are served only if their `[peripherals]` section is present.

**The objective in use** is the frames' context's, set by `objective.select`; else the turret's
position; else the only one configured. Every frame carries it and its calibration in force, if
any: at start, on `objective.select`, and after a calibration. The previews' dark and flat follow
the objective and the channel in the same way.

### The system routes

| Route | What |
|---|---|
| `GET /api/siq/system/health` | `status`, `version`, `uptime_s`, and the camera's model |
| `GET /api/siq/system/metrics` | Every metric by name, as `{"kind": "gauge", "value": …}`, `{"kind": "counter", …}` or `{"kind": "histogram", "count", "sum", "buckets"}`: the pool, the previews' latencies, and the acquisition's rate and losses |
| `GET /api/siq/system/ports` | This computer's serial ports, each `{ name, description, device }`, the device the configuration puts on it or `null`; the configured ports it hasn't, `missing`; and `error`, if they couldn't be listed (P10.3) |
| `GET /api/siq/system/memory` | The pool's slots by state and their size; the process's working set, its peak and its private bytes; the configured budgets, all in bytes; and the process's CPU time so far, `cpu_s`, in seconds |

### The session routes (ADR-0065)

The users are in `users.json` in `[server] settings`, managed with `spectrumiq-server user` or the
users routes. The service makes `[auth] local_user` at start if it is missing: an operator with a
random password no one is told, who logs in by the local secret only.

| Route | What |
|---|---|
| `POST /api/siq/auth/login` | With `{"name", "password"}`: `{"name", "role", "since", "expires"}` and the cookie `siq_session` (`HttpOnly`, `SameSite=Strict`, `Path=/`, `Max-Age` the session's lifetime); 401 if the name or the password is wrong; 429 with `Retry-After` past `max_failures` |
| `POST /api/siq/auth/logout` | 204: the session ended, and its cookie cleared |
| `GET /api/siq/auth/me` | `{"name", "role", "local", "since", "expires", "holds_token"}` of the cookie's session; 401 without a live one |
| `GET /api/siq/auth/local` | From this computer: `{"file"}`, where the local console's secret is, for the desktop shell to read; 403 from another computer, 404 without a local user |
| `POST /api/siq/auth/local` | With `{"secret"}`, the one in `local-login` in `[server] settings`, from this computer: a session of the local console as `[auth] local_user`, and its cookie; the next secret written. 403 from another computer, 404 without a local user, 401 for another secret |
| `GET /api/siq/control` | `{"holder": {"user", "since", "local"}}`, or `{"holder": null}` |
| `POST /api/siq/control/request` | The token for the session if it is free: who holds it now; 409 if another does, saying who |
| `POST /api/siq/control/release` | 204: the token given back; 409 if the session doesn't hold it |
| `POST /api/siq/control/heartbeat` | 204: the holder's sign of life, though any of its requests is one; 409 for another session |
| `POST /api/siq/control/take_over` | The token from whoever holds it, for an admin or the local console; 403 for others |
| `GET /api/siq/audit?from&to&user&kind&offset&limit` | An admin's query of the audit trail: `{"entries", "total"}`, newest first, 100 at most by default (ADR-0067) |
| `GET /api/siq/users` | An admin's: the users, `[{"name", "role", "created"}]`, without their hashes |
| `POST /api/siq/users` | An admin's: with `{"name", "password", "role"}`, 201 and the user; 400 for a bad name or a password under 8 characters, 409 if the name is taken |
| `PUT /api/siq/users/{name}` | An admin's: with `{"role"}`, `{"password"}` or both, the user. A new role applies to the user's sessions at once; a new password ends them. 404 for no such user, 409 for the last admin's role |
| `DELETE /api/siq/users/{name}` | An admin's: 204, the user's sessions ended; 404 for no such user, 409 for the last admin |

`Auth` is `Instrument::auth`: its `users`, `sessions`, `control` and `local`, and
`session(&headers)`, the live session of a request's cookie. The `control` Thing observes the
token: `holder` and `since`, and `changed` with the holder and the reason; it looks for a silent
holder's timeout every second.

### The data routes (ADR-0041, ADR-0045)

`{dataset}` is a dataset's id in the catalog of `[server] data_root`; `{image}` is an image of its
`zarr/` folder, without `.ome.zarr`.

| Route | What |
|---|---|
| `GET /api/siq/data/{dataset}/manifest` | The dataset's manifest, as JSON |
| `GET /api/siq/data/{dataset}/thumb.jpg` | Its first image's first plane, the channels composited in their colours, at the top level |
| `GET /api/siq/data/{dataset}/{image}/info` | The image's `name`, `sample` (`u16`, `f32`), `size` (`t`, `c`, `z`, `y`, `x`), `levels`, `pixel_size` and `origin` in µm (`z`, `y`, `x`), and `channels` (`label`, `colour` as `rrggbb`, `window`) |
| `GET /api/siq/data/{dataset}/{image}/{c}/{z}/{t}.dzi` | The plane's DZI descriptor |
| `GET /api/siq/data/{dataset}/{image}/{c}/{z}/{t}_files/{level}/{col}_{row}.{jpeg\|png}` | A tile, rendered on demand and cached within `tile_cache_mb`, with `min`, `max`, `gamma` and `cmap` (`grey`, `inverted`, `rrggbb`) in the query, each the channel's own if not given; an ETag, and 304 when it matches |
| `GET /api/siq/data/{dataset}/{image}/value?c&z&t&level&x&y` | `{"value", "level", "x", "y"}`: the true value of a pixel of a Zarr level |
| `GET /api/siq/data/{dataset}/{image}/overlays` | A stitched image's overlay layers, from `qa/<image>.json`: `stitch-tiles`, each tile's outline labelled with its position, and `stitch-residuals`, each pair's line, its `tone` its residual's (`success`, `warning`, `danger` rejected, `muted` untrusted); 404 without a report |

An unknown dataset, image, plane, level or tile answers 404, and a mapping that isn't valid 400,
each with `{"detail": …}`.

### Errors

- **A value that doesn't fit a property's or action's schema** answers 422, from teta-wot.
- **A camera setting the camera refuses** also answers 422, with the camera's reason, as a
  `value_error` at `body`. A region not aligned to the sensor's grid is one example (ADR-0033,
  amended).
- **Other failures** answer 500.
- **An action that fails** ends its invocation in `error`, with the reason.

Positions are in µm in the stage frame (ADR-0018). Each Thing's TD is at `/api/v1/{thing}/`, and
the interactive docs are at `/docs`.

```text
curl http://127.0.0.1:5000/api/siq/system/health
curl -X POST http://127.0.0.1:5000/api/v1/camera/start_live
curl -X POST http://127.0.0.1:5000/api/v1/stage/move_to -H "Content-Type: application/json" -d "{\"x\": -2000, \"y\": -1000, \"z\": 0}"
curl -X POST http://127.0.0.1:5000/api/v1/imaging/select_channel -H "Content-Type: application/json" -d "{\"id\": 2}"
curl http://127.0.0.1:5000/api/siq/system/memory
```

## Examples

| Example | What it shows | Run |
|---|---|---|
| `motion_repeatability` | The stage's repeatability and backlash, over `--repeats` arrivals (10) from `--reach` µm (50) each side, on the simulated instrument against its injected noise and backlash, or on the instrument of `-c` (P10.3). 20 repeats: 0.61 and 0.57 µm against 0.5, and 2.05 and 1.87 µm against 2 | `cargo run --release -p siq-server --example motion_repeatability [-- -c <config>] [--repeats N] [--reach µm]` |
| `calibrate_sim` | The simulated instrument's stage–camera calibration through the `calibration` Thing, on a 4096 × 3072 sensor binned 2 × 2: the scale, rotation and shear found against those injected, the residuals, the backlash, and where it was kept | `cargo run --release -p siq-server --example calibrate_sim [-- --settings <folder>]` |
| `live_mosaic_sim` | The live mosaic through the `mosaic_live` Thing: a 2048 × 1536 sensor stepped over 6 × 6 fields at 10 % overlap while the live view runs, each painted where its pose puts it; the whole area's view and a 1:1 view where four fields meet written as JPEGs, and the mosaic saved | `cargo run --release -p siq-server --example live_mosaic_sim [-- <out.jpg>]` |

Every example also runs in smoke mode (`SIQ_SMOKE=1`) in `cargo xtask ci`: `calibrate_sim` on
1024 × 768, where the smaller field calibrates less precisely; `live_mosaic_sim` on 512 × 384, 3 × 3
fields.

On the RTX 4060, `live_mosaic_sim` paints a frame in 1.8 ms, binned and its upload included: 36
fields in 15.7 s, the stage's moves and settling most of it, with no frame lost.

`apps/spectrumiq-server` runs the service. Scripts drive it over HTTP, with the standard library
only:

- `examples/python/live_and_move.py` moves the stage, and saves a frame before and after;
- `examples/python/channels.py` cycles through the channels, and saves a snap of each;
- `examples/python/live_soak.py` runs live view with both previews streaming, by default for
  10 minutes, and fails if a frame is lost;
- `examples/python/snap_and_fetch_tile.py` snaps into a dataset, waits for its pyramid, and saves
  a tile and the thumbnail, with the time each step took;
- `examples/python/tile_scan.py` runs a 2 × 2 tile scan, calibrating first if needed, and prints
  its progress;
- `examples/python/z_stack.py` takes 11 planes in two channels about the focus;
- `examples/python/timelapse.py` visits the saved positions every 10 s, four times, pausing once;
- `examples/python/stitch_dataset.py` stitches the newest tile scan again through `jobs.stitch`,
  follows its job, and prints its measures from the history and its worst pairs from the QA.

[`test.http`](../../test.http), at the repository's root, has a request for every route and
affordance, for VS Code's REST Client extension. It includes expected refusals. The requests named
with `# @name` feed the ones after them: the snap's state and JPEG, and cancelling a move.

## Features

- **`euresys`:** the Euresys camera, `[camera] kind = "euresys"` (`siq-euresys`, ADR-0070),
  through the installed eGrabber SDK. Its pool is locked in RAM. Without it, such a configuration
  is refused at start, saying so.

## Testing

`cargo test -p siq-server` runs its binaries, with `config/sim.toml` on a 256 × 192 sensor and
no calibrations, through teta-wot's in-process `TestClient`.

`server`:

- the Things and their affordances, with a snapshot;
- a stage move observed over server-sent events until it arrives;
- live view's MJPEG frames, and a snap as a JPEG;
- the global lock refusing a second hardware action, and `halt` passing it;
- SpectrumIQ's routes in front of teta-wot's;
- the OpenAPI document's paths, with a snapshot, none of them under `/api/siq/`;
- serving over TCP, and shutting down with live view stopped first.

`auth` (P9.1), through `Service::router()`:

- a login's cookie and its attributes, `me` with it and without, and a logout ending it;
- five wrong passwords, then the right one refused with 429 and `Retry-After`, while another name
  from the same address logs in, and an unknown name answers as a wrong password;
- a `PUT` on a Thing's property and a `POST` on a SpectrumIQ route refused from another site's
  origin, and passing from the service's own or without one;
- a role changed and a user removed by another `UserStore`, the CLI's, reaching its session.

`authz` (P9.2), with `[auth] enforce`, a token timeout of a second, and an admin, two operators, a
viewer and the local console's user:

- every route of the OpenAPI document and of SpectrumIQ's classified, with a snapshot of every
  change's class (`authz__route_classes`);
- the decision for every class and every caller against architecture §10's matrix;
- 401 without a session, the health public; a viewer reading but refused a change and a stop; an
  operator refused without the token, served with it; another refused the token, with its
  holder, but allowed to stop the stage and to process; deleting data an admin's; an admin's
  take-over, after which the first operator is refused; release by the holder only;
- a holder's requests half a second apart keeping the token, then 2.5 s of silence losing it, on
  the `control` Thing too;
- the local console's file named to this computer only; the console refused from another
  computer, logging in from this one as the operator the service made, refused the same secret
  twice, logging in with the next, and taking the token over from an operator;
- the users listed without their hashes to an admin and refused to a viewer; one added, refused
  a second time, given a role that applies to her session at once and a password that ends it,
  and removed; the last admin kept; no password in the trail.

`audit` (P9.3), with `[auth] enforce`: a failed login, three logins, the token requested, a change
with its parameters, a viewer's refusal, a logout ending the token, a job's end and a workflow's
run of one snap, all recorded by whom and from where; a read that is a `POST` not recorded; the
query refused to a viewer and given to an admin; no password in the trail; its chain intact.

`colour` (P10.6), the simulated sensor given a filter array: the histogram per colour, blue
below red below green under the warm lamp; `white_from_roi` raising red and blue; a snap's image
keeping its filter array, and a tile of it in colour.

`peripherals`:

- each peripheral's Thing over HTTP: its properties, its action, its event, and a refusal;
- a channel switch, with the true state sampled throughout: no light while the wheel turns, and
  the new light only after its filter;
- an instrument without peripherals: only `imaging` is added, and a channel still sets the camera;
- the metrics' names, with a snapshot, and the memory route.

`datasets`, end to end over HTTP on a 640 × 480 sensor:

- a snap in two channels made into a dataset, its event, and its pyramid built as a job;
- level 0 holding the raw frames' pixels, the channels named and coloured as theirs;
- its tiles, manifest (both jobs recorded) and thumbnail served;
- the summary, both exports as jobs, and an export of an image it hasn't refused;
- deleting it: refused while a job works on it, then done.

`calibration`, end to end over HTTP on a 4096 × 3072 sensor binned 2 × 2, in about 25 s, from
the corner of the stage's travel where it starts, and back:

- the camera's mount (0.4°, 0.2 % scale, a shear of 0.001) and the stage's scale error recovered
  within 0.1 % in scale and 0.05° in rotation, against the stage's noise and whole steps;
- the residuals under 1 µm RMS, and the backlash within 1 µm of the 2 injected;
- the calibration kept, in force, in `status`, on the next frame with its objective, and copied
  into a snap's dataset;
- the same on a colour sensor, its frames registered on their luminance (P10.7).

`shading`, end to end over HTTP on a blank slide, with the camera's cos⁴ vignetting:

- the dark at the sensor's 100 ADU, the lights back as they were;
- the flat 50 µm out of focus, the focus back where it was, within 0.07 % of the vignetting;
- both in force and in `status`; the previews' correction turned on;
- a `siq-datagen` tile scan's flat estimated as a job, and kept for its frames' objective and
  channel;
- a flat from the corner of the stage's travel, its places a field in, and the stage back;
- a colour sensor's flat, of its frames' luminance, within 2 % of the vignetting (P10.7).

`workflow`, over HTTP with the simulator's mount as the calibration in force:

- a 2 × 2 tile scan with three-plane stacks, estimated first, its frame the field of view's: its
  state and progress while it runs, a manual move refused as `GlobalLockBusy`, its 12 frames in
  33 steps within 20 % of the estimate's time, listed, and the hook told;
- a time-lapse paused, holding still, resumed, then cancelled by deleting its invocation, its
  frames kept; `pause` with no run refused;
- plans that couldn't run refused with 422, with their reasons, and one this instrument can't run
  failing at once with no dataset;
- a failed step that asks: `asking`, its problem, retried, asked again, skipped; `skip` with
  nothing waiting refused;
- positions saved, refused twice under one name, renamed and removed; a multi-position visiting
  them; kept for a server started on the same folder.

`stitch`, over HTTP with the simulator's mount as the calibration in force, over grains:

- a 2 × 2 tile scan at a quarter's overlap stitched by itself once it finishes: its record's
  outputs and results (4 tiles, 6 pairs, one component, under 0.5 px RMS), its mosaic's info and a
  tile, and its overlays, four outlines and six toned lines; stitched again on `jobs.stitch` into
  another image; overlays of an image without a report, 404;
- a dataset without a calibration refused, saying so; auto-stitch turned off.

`mosaic`, over HTTP with the simulator's mount as the calibration in force, each GPU test skipped
with its reason where there is no GPU:

- 3 × 3 fields visited and painted, their bounds within 3 µm of the grid's, `px_per_um` the
  calibration's at ½; the snapshot a `mosaic` dataset, correlating 0.87 with the specimen where the
  stage truly stopped, best at no offset of ±3 canvas pixels; cleared, no bounds;
- the largest view streamed at 1920 × 1080 while the stage moves and the frames paint, with no
  frame lost and the pages within the VRAM budget; disabled;
- a viewport or a canvas scale out of range refused with 422; a snapshot of nothing failing;
- without a GPU (`[gpu] enabled = false`), enabling refused with 422, saying why.

`webapp`, from a build in a temporary folder:

- the page and its assets, with their media types and caching;
- paths that would leave the build, refused;
- the placeholder without a build;
- the API untouched.

## Limitations

- **The Euresys camera** needs the `euresys` feature, and eGrabber's SDK to build it.
- **The peripherals are simulated only,** until P10.5.
- **The simulated camera images transmitted light only:** a fluorescence channel's snap is a
  brighter brightfield, tinted.
- **The lifecycle copies `ThingServer::serve_with` at v0.1.0:** check it on each teta-wot upgrade.
- **A run holds the global lock throughout:** for hours, a time-lapse leaves only `pause`,
  `resume`, the halts and what reads.
- **A workflow's dataset raises no `datasets.created`:** `workflow.finished` says so instead;
  nor does a mosaic's snapshot.
- **The live mosaic paints moving frames only below about 7 µm/s at 10×,** with 5 stage readings
  a second and the default `max_uncertainty_px`: faster, the frames wait for the stage to stop.
- **The live mosaic repaints the field the stage stands on at the frame rate,** its binning on the
  frame-path pool the previews share: disable it when it isn't looked at.
