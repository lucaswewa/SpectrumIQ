# siq-server

SpectrumIQ's service core. It makes the instrument's devices from the configuration, starts the
engines, and serves the instrument's Things on teta-wot, with SpectrumIQ's own routes in front.

- **Devices,** simulated or real, chosen per device (ADR-0034): the camera, the GRBL stage, the
  ASCII focus drive, and the fitted peripherals (simulated until P10.5).
- **Engines:** the acquisition (`siq-acquire`) and the previews (`siq-preview`), shared by the
  Things as the `Instrument` service.
- **Things,** under `/api/v1/` (ADR-0033):
  - `camera`, `display`, `stage` and `focus`;
  - the fitted peripherals: `illumination`, `filter_wheel`, `objective` and `modality`;
  - `imaging`, the channels (ADR-0035).

  The global lock is on, and the `halt` actions bypass it.
- **Routes,** under `/api/siq/system/`: `health`, `metrics` and `memory`, with teta-wot's router
  as the fallback (ADR-0003, amended).
- **The lifecycle:** start the Things, serve, and on shutdown stop live view and motion before
  the Things.

This is the only crate that depends on teta-wot.

## Place in the architecture

- **Layer:** L5 Service.
- **Depends on:**
  - teta-wot v0.1.0 (git), and `tower` for `oneshot`;
  - `siq-acquire`, `siq-preview`;
  - `siq-devices`, `siq-grbl`, `siq-ascii`, `siq-serial`, `siq-sim`;
  - `siq-config`, `siq-telemetry`, `siq-buffer`, `siq-imgproc`, `siq-synth`, `siq-types`;
  - `tokio`, `serde`, `schemars`, `anyhow`, `tracing`.
- **Used by:** `apps/spectrumiq-server`, the service binary.
- **Phase:** P2.3; P2.4 added the peripherals, the channels, and the metrics and memory routes.

## API

| Item | What |
|---|---|
| `Service::build(config)` | Opens the devices, starts the acquisition and previews, and makes the Things |
| `service.serve(listener, shutdown)` | Serves until `shutdown` completes, then shuts down |
| `service.router()` | Every route: SpectrumIQ's first, teta-wot's after |
| `service.instrument()`, `service.into_parts()` | The `Instrument` (devices and engines), and the Things' server apart, for tests |
| `openapi()` | The Things' OpenAPI document, made without devices |
| `devices::open(config, clock)` | The devices alone, with the fitted `Peripherals` |
| `shutdown_signal()` | Ctrl-C, or the system asking the process to stop |
| `API_PREFIX` | `/api/v1` |

### The Things

| Thing | Properties | Actions | Events and streams |
|---|---|---|---|
| `camera` | `exposure_ms`, `region`, `binning`, `pixel_format` (writable); `frame_rate`, `sensor`, `stats` | `start_live`, `stop_live`, `snap` → JPEG | `frames_lost`; MJPEG `overview` and `roi` |
| `display` | `lut`, `roi_region` (writable); `histogram`, `focus_measure` | `auto_contrast_once` | — |
| `stage` | `position`, `state`, `homed` (observable); `speed`; `limits` | `move_to`, `move_by`, `jog`, `home`, `halt` | `arrived`, `alarm` |
| `focus` | `position`, `state`, `homed` (observable); `limits` | `move_to`, `move_by`, `halt` | `arrived` |
| `illumination` | `levels` (observable); `sources` | `set_level(source, level)`, by the source's name | `changed` |
| `filter_wheel` | `position`, `moving` (observable); `filters` | `select(position)` | `changed` |
| `objective` | `position`, `moving` (observable); `objectives`, `current` | `select(position)`, which also sets the frames' objective | `changed` |
| `modality` | `modality`, `moving` (observable); `modes`, `patterns` | `set_modality(mode, pattern?)` | `changed` |
| `imaging` | `channel` (observable); `channels` | `select_channel(id)`: every light off, the filter, the exposure and colour, the light, then the frames' channel | `channel_changed` |

The peripherals' Things are served only if their `[peripherals]` section is present.

### The system routes

| Route | What |
|---|---|
| `GET /api/siq/system/health` | `status`, `version`, `uptime_s`, and the camera's model |
| `GET /api/siq/system/metrics` | Every metric by name, as `{"kind": "gauge", "value": …}`, `{"kind": "counter", …}` or `{"kind": "histogram", "count", "sum", "buckets"}`: the pool, the previews' latencies, and the acquisition's rate and losses |
| `GET /api/siq/system/memory` | The pool's slots by state and their size; the process's working set, its peak and its private bytes; the configured budgets, all in bytes |

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

`apps/spectrumiq-server` runs the service. Two scripts drive it over HTTP, with the standard
library only:

- `examples/python/live_and_move.py` moves the stage, and saves a frame before and after;
- `examples/python/channels.py` cycles through the channels, and saves a snap of each.

## Features

None.

## Testing

`cargo test -p siq-server` runs two binaries, with `config/sim.toml` on a 256 × 192 sensor,
through teta-wot's in-process `TestClient`.

`server`:

- the Things and their affordances, with a snapshot;
- a stage move observed over server-sent events until it arrives;
- live view's MJPEG frames, and a snap as a JPEG;
- the global lock refusing a second hardware action, and `halt` passing it;
- SpectrumIQ's routes in front of teta-wot's;
- the OpenAPI document's paths, with a snapshot, none of them under `/api/siq/`;
- serving over TCP, and shutting down with live view stopped first.

`peripherals`:

- each peripheral's Thing over HTTP: its properties, its action, its event, and a refusal;
- a channel switch, with the true state sampled throughout: no light while the wheel turns, and
  the new light only after its filter;
- an instrument without peripherals: only `imaging` is added, and a channel still sets the camera;
- the metrics' names, with a snapshot, and the memory route.

## Limitations

- **The Euresys camera** is refused until P10.1.
- **The peripherals are simulated only,** until P10.5.
- **The simulated camera images transmitted light only:** a fluorescence channel's snap is a
  brighter brightfield, tinted.
- **No authentication yet:** P9.1. Bind to `127.0.0.1` until then.
- **The lifecycle copies `ThingServer::serve_with` at v0.1.0:** check it on each teta-wot upgrade.
