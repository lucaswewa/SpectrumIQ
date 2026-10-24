# ADR-0003: teta-wot v0.1.0 as the service framework, extended inside SpectrumIQ

- **Status:** Accepted
- **Date:** 2026-10-23
- **Phase:** planning
- **Related:** REQ-NET-002, REQ-SEC-001, REQ-SEC-002

## Context

The goals ask to embed `teta-wot` (`c:\repos\teta-wot`, `github.com/lucaswewa/teta-wot`). v0.1.0
(`135f9ff`) provides:

- Things, properties, actions and events;
- invocations with cancellation;
- MJPEG streams and Blobs;
- slots and interfaces, services, and device actors;
- an optional global lock;
- TD and OpenAPI generation;
- `api_prefix`;
- a Windows service mode;
- `ThingServer::router()`, which returns an axum `Router`.

It lacks four things SpectrumIQ needs:

- **Credentials:** it supports one shared Basic or bearer credential, with no roles.
- **Binary WebSocket:** its WebSocket is observe-only (`addPropertyObservation`,
  `addActionObservation`, `addEventSubscription`) and rejects binary frames.
- **Per-client lock:** its global lock is per invocation, not per client.
- **HTTP/2:** it serves HTTP/1.1 only.

## Decision

- **Dependency:** `teta-wot = { git = "https://github.com/lucaswewa/teta-wot", tag = "v0.1.0" }`.
  Upgrades are deliberate.
- **Use teta-wot unmodified.** SpectrumIQ adds what is missing in `siq-server`:
  - an axum auth and authorization layer around the whole router (method and route class →
    role);
  - a `control` Thing for the Hardware Control Token;
  - SpectrumIQ routes under `/api/siq/…`, merged with the router (data and tiles, auth, system,
    and the later binary streams).
- **Route layout:**
  - teta-wot Things under `/api/v1/` (`api_prefix`), the same as `microscope`, so the copied
    client works unchanged (ADR-0008);
  - the web app at `/`;
  - a test asserts that the route sets don't overlap.
- **The global lock is enabled.** Hardware actions take it, other affordances opt out, and
  `stop` bypasses it.
- **Generic improvements may be upstreamed later,** once they are proven here.
- **`siq-server` is the only crate that depends on teta-wot** (architecture §3).

## Alternatives considered

- **Extend teta-wot first:** each phase would touch two repositories.
- **Fork it into the workspace:** loses upstream fixes.

## Consequences

Authorization relies on teta-wot keeping its WebSocket observe-only. A test walks the OpenAPI
document so that every route is classified (P9.2).

## Verification

P2.3 integration tests through `TestClient`. P9.2's route-classification test.

## Amendment (P2.3, 2026-10-24): the route layout, and the merge order

- **teta-wot's Things are under `/api/v1/`** (`api_prefix`), with its directory at
  `/api/v1/things/` and its OpenAPI document and docs pages at the root (`/openapi.json`, `/docs`,
  `/redoc`), as in FastAPI.
- **SpectrumIQ's routes are under `/api/siq/`,** from `system/health` now.
- **The merge order:** SpectrumIQ's router comes first, and teta-wot's router is its fallback
  (`Router::fallback_service`). So a SpectrumIQ route is matched before teta-wot sees the request,
  and every other request goes to teta-wot unchanged.
- **The lifecycle is SpectrumIQ's,** because `ThingServer::serve_with` serves only teta-wot's
  router. `siq_server::Service::serve` follows `serve_with` at v0.1.0, with one change: live view
  stops, then motion, before the Things do. The steps are:
  1. start the Things;
  2. serve until the shutdown signal;
  3. stop accepting connections;
  4. stop live view and the stage and focus drive;
  5. end observation streams, and cancel invocations;
  6. wait up to the grace period;
  7. stop the Things in reverse order.

  Compare it with `serve_with` again whenever teta-wot is upgraded.
- **The overlap test:** `siq-server`'s `the_openapi_document_has_every_affordance` checks that no
  teta-wot path is under `/api/siq/`, and `siq_routes_come_first_and_the_things_after` checks that
  both are served.
- **`tower`'s `oneshot`** reads the OpenAPI document from the router without serving
  (`spectrumiq-server --print-openapi`).
