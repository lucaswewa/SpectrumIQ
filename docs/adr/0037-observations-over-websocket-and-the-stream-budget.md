# ADR-0037: Observations over WebSocket, and the stream budget

- **Status:** Accepted
- **Date:** 2026-10-24
- **Phase:** P3.2
- **Related:** ADR-0008, ADR-0010, REQ-NET-002, REQ-PRV-001; `microscope`'s ADR-0014, from which
  this deviates

## Context

- **Browsers open at most 6 HTTP/1.1 connections per server,** and teta-wot serves HTTP/1.1 only
  (architecture §7.3, ADR-0010). Each server-sent event stream and each MJPEG stream holds one
  connection for as long as it runs.
- **The copied client observes over fetch-SSE,** one stream per property or event (`microscope`'s
  ADR-0014). That suits `microscope`'s few Things. SpectrumIQ serves nine Things now, and
  following a handful of each would take every connection.
- **teta-wot serves a WebSocket per Thing,** `{prefix}/{thing}/ws`. Its TD gives it as a form,
  with `op` `observeproperty` or `subscribeevent`.
  - **The client sends** `{"messageType": "addPropertyObservation", "data": {name: …}}`, or
    `addEventSubscription`.
  - **The server sends** `propertyStatus` with `data: {name: value}`, and `event` with
    `data: {name: {data, timestamp}}`.
  - **A name it can't follow** gets a `response` with an error: 404 not found, 403 not
    observable.
  - **There is no unsubscribe,** and no current value is sent on subscribing.
- **Browsers don't count WebSockets against the 6 HTTP connections.**

## Decision

### Observations

- **`ThingSocket`** (`web/src/api/wot/ws.ts`) is one socket per Thing, which every observation of
  that Thing shares:
  - it opens with the first subscriber and closes with the last;
  - each (re)connection subscribes to every name with subscribers, and a name added while open is
    subscribed at once;
  - a socket that drops is reopened after 1 s, then 2, 4, … up to 30 s, as the copied streams are;
  - an error response goes to that name's subscribers, as an `ApiError` with the status;
  - an event's data is handed over without its timestamp, as the SSE stream gives it;
  - a name without subscribers is dropped on arrival, since the protocol can't unsubscribe it.
- **`ConsumedThing.observeProperty` and `subscribeEvent`** use the Thing's socket when its TD has
  a WebSocket form and the platform has `WebSocket`. Otherwise they use fetch-SSE, which stays as
  the fallback. `ClientOptions.webSocket` replaces the socket factory, and `null` forces SSE.
- **Requests carry `credentials: 'same-origin'`,** so the session cookies of M9 reach the server
  that served the page. A WebSocket sends them by itself.

### The stream budget

- **`StreamBudget`** (`web/src/api/streams.ts`) runs at most `MAX_STREAMS` = 3 MJPEG streams.
  - A stream wants to run while the page is visible (`visibilitychange`) and, if it has an
    element, while that is in view (`IntersectionObserver`).
  - The first three that want to run do, in the order they asked. The others wait, and start by
    themselves when a place frees up.
  - Streams that lose their place stop before others start, so no more than three run, even for
    a moment.
- **`useBudgetedStream(element, start)`** puts a component's stream in the budget while its
  element exists, and takes it out with the component. P3.3's views use it, through the app's
  single budget, `streamBudget`.

### The stores

- **`connection`** connects to the API of the server that served the page:
  - it reads the TDs and `/api/siq/system/health`, then checks the health every 5 s;
  - a failure makes it `lost`, and it tries again after 1 s, then 2, up to 10 s;
  - `isAvailable(thing)` feeds the rail, so a destination shows when its Thing is served.
- **`camera`, `display`, `stage`, `focus`, `imaging`, `peripherals`** mirror their Things through
  `useThingMirror` (`web/src/stores/thing.ts`):
  - each property is read once;
  - an observable property is then followed over the socket, and others are polled where a store
    asks (the camera's `stats` each second, the display's statistics every 500 ms);
  - a value equal to the last is ignored, since teta-wot resends unchanged status;
  - actions run as invocations to their end;
  - a failure, `ApiError` included, becomes an error toast with the server's error folded under
    it, and the action returns `undefined`.
- **The types come from the server.** `npm run api:openapi` writes
  `src/api/generated/openapi.json`, and `npm run api:types` writes `schema.d.ts`; both are
  committed.
  - The stores take each property's, input's and output's type from them, by name, so a store
    that names an affordance the server doesn't have fails to compile.
  - `cargo xtask ci` runs `api:types -- --check`.
  - A server test, `the_web_apps_openapi_document_is_current`, fails when the committed document
    is stale.

## Deviation from `microscope`

`microscope`'s ADR-0014 observes over fetch-SSE only. That was deliberate there: one transport,
and headers on every request. SpectrumIQ deviates, as ADR-0008 foresaw, because it has many more
Things. SSE remains its fallback.

## Alternatives considered

- **One socket for every Thing:** teta-wot's socket is per Thing, and multiplexing needs a server
  change (planned for frames in P14.2).
- **SSE with `observeallproperties`, one stream per Thing:** still one HTTP connection each, so
  nine Things would take every connection.
- **Polling everything:** no connection held, but late values and a constant load.
- **A budget that closes the oldest stream:** the view the user looks at longest would flicker;
  waiting keeps the views already open steady.

## Consequences

- With the simulator, the app holds 8 sockets (every Thing with something observable) and no HTTP
  stream. All 6 connections remain for requests and up to 3 MJPEG streams.
- A Thing's values keep arriving for names nobody follows any more, until the socket closes.
  teta-wot can't unsubscribe.
- The client's tests use fake sockets. The SSE tests copied from `microscope` now test the
  fallback.

## Verification

- `web/tests/unit/api/socket.spec.ts`:
  - subscribing;
  - delivery, with the event's timestamp removed;
  - refusals;
  - reconnecting with backoff, and resubscribing;
  - closing with the last subscriber;
  - the client's choice of socket or SSE.
- `web/tests/unit/api/streams.spec.ts`:
  - the limit and the order;
  - a hidden page;
  - an element out of view;
  - a component's stream.
- `web/tests/unit/stores/stores.spec.ts`:
  - the connection, lost and back;
  - mirroring, with repeated values ignored;
  - polling;
  - actions and their toasts;
  - writes.
- **By hand:** `/#/dev/observe` against the simulator, in Edge. There were 8 sockets and no SSE
  stream, and the stage's position followed a move.
