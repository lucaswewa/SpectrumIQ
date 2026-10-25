# ADR-0033: Thing decomposition and naming

- **Status:** Accepted
- **Date:** 2026-10-24
- **Phase:** P2.3
- **Related:** ADR-0003, ADR-0023, ADR-0031, ADR-0032, REQ-NET-002, REQ-SEC-004

## Context

- **teta-wot serves Things,** each with properties, actions, events and MJPEG streams, under
  `/api/v1/{thing}/` (ADR-0003). Its global lock serializes hardware access.
- **Architecture §6.1 lists the Things** the UI and scripts use. This phase brings the first four:
  `camera`, `display`, `stage` and `focus`.
- **Things should be thin.** They adapt the engines and drivers, and hold no algorithms.
- **teta-wot's macros name an action after its method,** and the `Thing` trait has lifecycle
  methods `start` and `stop`.

## Decision

### One Thing per role the UI controls

| Thing | Properties | Actions | Events, streams |
|---|---|---|---|
| `camera` | `exposure_ms`, `region`, `binning`, `pixel_format` (writable); `frame_rate`, `sensor`, `stats` | `start_live`, `stop_live`, `snap` | event `frames_lost`; streams `overview`, `roi` |
| `display` | `lut`, `roi_region` (writable, without the lock); `histogram`, `focus_measure` | `auto_contrast_once` (without the lock) | — |
| `stage` | `position`, `state`, `homed` (observable); `speed` (a setting); `limits` | `move_to`, `move_by`, `jog`, `home`, `halt` | `arrived`, `alarm` |
| `focus` | `position`, `state`, `homed` (observable); `limits` | `move_to`, `move_by`, `halt` | `arrived` |

### How each Thing works

- **The `Instrument` service** holds the devices and the engines: the acquisition, the previews,
  the latest statistics, the stage, the focus drive and the metrics. Each Thing takes it from the
  server when it starts.
- **Camera settings are functional properties.** Writing one reconfigures the acquisition
  (ADR-0031), which restarts the camera if it must.
- **The stage's and focus's status is mirrored into data properties** by a task that follows the
  driver's watch. So `position` and `state` are observable, over server-sent events and
  WebSocket.
- **The display's properties are the preview settings themselves** (`DisplaySettings` and
  `RoiSettings`, ADR-0032), serialized as they are. For example
  `{"contrast": {"kind": "auto", "low_pct": 0.1, "high_pct": 99.9}, "colormap": "grey"}`.
- **`snap`** waits for the stage and focus to be still, triggers the camera (ADR-0031), and returns
  the frame's preview as a JPEG `Blob`.
- **A move's invocation, when cancelled (`DELETE`), stops the device.** `arrived` follows each
  completed move.
- **`frames_lost`** is emitted, at most once a second, when the camera's lost-frame totals rise.

### The global lock

- **The server enables it.** Hardware actions and writes take it.
- **The display's affordances opt out:** they touch no hardware.
- **`stage.halt` and `focus.halt` opt out** (REQ-SEC-004), so they stop a move that holds the
  lock.

### Naming

- **Things are lowercase nouns,** and affordances are snake_case, as in teta-wot and `microscope`.
- **Units are in the name** where the type doesn't carry them (`exposure_ms`), or are the stage
  frame's µm (ADR-0018). The TD's `unit` says so.
- **Two names differ from architecture §6.1:**
  - the camera's sensor ROI is **`region`**, because `/camera/roi` is the ROI preview stream;
  - the stop actions are **`halt`**, because an action named `stop` collides with the `Thing`
    trait's `stop` lifecycle method under teta-wot's macros.

## Alternatives considered

- **One `microscope` Thing with everything:** a large TD, and one lock scope for the camera and the
  stage. The UI would also lose the per-device grouping that `microscope`'s client expects.
- **Engines inside each Thing's struct:** teta-wot builds Things with their `Default`, and the
  engines need the configuration. The shared service keeps one instance of each engine.
- **Polling the drivers from functional properties:** they would not be observable.
- **A `stop` action written with `ThingDefinition` by hand:** possible, but it would give up the
  macros for the whole Thing, and their checks.

## Consequences

- **Every affordance is in the TDs and the OpenAPI document,** which clients generate their types
  from (`spectrumiq-server --print-openapi`).
- **The display mapping is shared by every client:** one viewer's change shows in another's
  stream.
- **The peripherals and the `imaging` Thing follow in P2.4,** in the same way (ADR-0035).

## Verification

`siq-server` `server` tests:

- `the_things_are_listed_and_described`, with a snapshot of each Thing's affordances;
- `a_stage_move_over_http_is_observed_until_it_arrives`;
- `live_view_streams_mjpeg_and_a_snap_comes_back_as_a_jpeg`;
- `the_global_lock_refuses_a_second_hardware_action_but_not_halt`;
- `the_openapi_document_has_every_affordance`, with a snapshot of the paths.

## Amendment (after P2.5, 2026-10-24): a refused setting is the client's error

- **A camera setting the camera refuses answers HTTP 422, not 500.** The camera's setters turn
  `DeviceError::InvalidArgument` into teta-wot's `PropertyError::Invalid`: a pydantic-style
  `value_error` at `body`, with the camera's reason and the value sent. A region not aligned to the
  sensor's grid is one example.
- **Other failures stay `PropertyError::Failed`, HTTP 500:** a device that doesn't answer, or a
  restart that fails.
- **Verification:** the `siq-server` `server` test
  `a_value_the_camera_refuses_is_a_422_with_its_reason`.
