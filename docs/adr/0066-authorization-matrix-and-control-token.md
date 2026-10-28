# ADR-0066: The authorization matrix and the Hardware Control Token

- **Status:** Accepted
- **Date:** 2026-10-27
- **Phase:** P9.2
- **Related:** ADR-0011, ADR-0065, ADR-0003, ADR-0007, REQ-SEC-002, REQ-SEC-004

## Context

ADR-0065 tells who is calling. M9 also needs to decide what each caller may do (architecture
§10), and to keep one operator at a time on the instrument, so that two clients don't make
conflicting stage moves. Three facts shape this:

- Most of the API is teta-wot's: its Things' properties, actions and events, under `/api/v1/`.
  Its actions run inside the Thing, with no knowledge of the HTTP request or its caller.
- teta-wot's WebSocket is observe-only, and its connection is teta-wot's: a layer in front of it
  sees the upgrade request, but not the messages that follow.
- The desktop runs on the instrument's own computer, and should log in without a password
  prompt (ADR-0011).

## Decision

- **Every route has a class** (`siq_server::authz::classify`), from its method and path:

  | Class | What | Who |
  |---|---|---|
  | Public | the web app, `/api/siq/auth/…`, the health, `OPTIONS` | anyone |
  | Read | any other `GET` or `HEAD`: properties, events, streams, tiles, descriptions, WebSocket upgrades | any user |
  | Stop | `stage.halt`, `focus.halt`, `DELETE` of an invocation, a run's included | operators, admins |
  | Processing | `jobs`, the exports, the shading's estimate, the live mosaic's snapshot | operators, admins |
  | Control | `/api/siq/control/…` | operators, admins; taking over, admins and the local console |
  | Hardware | any other change to a hardware Thing: the camera, stage, focus, peripherals, imaging, display, live mosaic, workflow and calibration; `datasets.snap_to_dataset` | operators and admins holding the token |
  | Admin | `datasets.delete`; the users and the configuration later | admins |

  Two `POST`s that only compute, `workflow.estimate_tile_scan` and `datasets.summary`, are reads.
  A change to a route without a class is refused.
- **One layer decides,** around the whole router, teta-wot's included (ADR-0065's):
  - without a session, a non-public request gets 401;
  - a role or a token short of the class gets 403, with the reason;
  - this applies only where `[auth] enforce` (on by default).
- **The display and the live mosaic's view are the instrument's:** they are shared by every
  client, so changing them takes the token. A viewer sees what the operator sees.
- **The Hardware Control Token** (`siq_auth::ControlManager`):
  - at most one session holds it;
  - `POST /api/siq/control/request` gives it to an operator or admin if it is free, and answers
    409 with its holder if not;
  - `release` gives it back;
  - `take_over` takes it from its holder, for an admin or the local console;
  - a logout lets it go.
- **The heartbeat is any request of the holder's session,** and `POST /api/siq/control/heartbeat`
  for a client that has nothing else to ask. A holder silent for `control_timeout_s` (30) loses
  the token. The `control` Thing looks every second, so its watchers hear of a timeout as it
  happens. Losing the token stops nothing: a run started keeps running.
- **The `control` Thing** observes the token: `holder` and `since`, and `changed` with the holder
  and the reason (requested, released, taken over, timed out, ended). The token's routes are
  SpectrumIQ's, not the Thing's actions, because they need the caller.
- **The local console's login** (`siq_auth::LocalLogin`), where `[auth] local_user` names a user:
  - the service writes a 256-bit secret to `local-login` in `[server] settings`;
  - the file's access list has a single entry, the service's own account, with full access and
    nothing inherited, set with `SetNamedSecurityInfoW` before the secret is written;
  - `POST /api/siq/auth/local` with the secret, from a loopback address only, opens a session as
    that user, marked local;
  - each secret works once: the exchange writes the next;
  - a local session may take the token over whatever its role.

## Alternatives considered

- **The heartbeat over teta-wot's WebSocket,** as first planned: the layer doesn't see the socket's
  messages, and a socket open in a forgotten tab would keep the token forever. A request is a
  sign of life the service already sees.
- **The token's operations as the `control` Thing's actions:** an action doesn't know its caller,
  so it couldn't tell whose token it is.
- **Hiding refused routes,** answering 404: a client couldn't tell a missing route from a refusal,
  and P9.4's UI explains refusals.
- **Viewers changing their own display:** the display is the service's, one for every client; a
  per-client display is a frontend matter (M14).
- **An ACL for the interactive user too:** while the desktop starts the service, they are one
  account. A service run as another account (M15) adds the interactive user's entry then.

## Consequences

- **Every new route must be classified:** `tests/authz.rs` walks the OpenAPI document and the
  SpectrumIQ routes, fails on an unclassified change, and snapshots every change's class.
- **Until the web app and the desktop log in (P9.4),** `config/sim.toml` sets `enforce = false`.
  The tests and `two_clients.py` enforce.
- **Scripts** log in, and keep their cookie: `examples/python/two_clients.py` shows how.
- **An admin needs the token too** to change the instrument: being an admin lets one take it
  over, not bypass it.

## Verification

- `crates/siq-server/tests/authz.rs`:
  - every route classified, with a snapshot of the changes' classes;
  - the decision for every class and every caller, against architecture §10;
  - over HTTP, a viewer refused, an operator refused without the token and served with it,
    another refused the token but allowed to stop, an admin's take-over;
  - a silent holder losing the token, seen on the `control` Thing;
  - the local console logging in once per secret, from this computer only, and taking the token
    over.
- `crates/siq-auth/tests/control.rs`: the token's handover, timeout, heartbeat and logout on a
  paused clock; the local secret's rotation, and its file's single access entry, read by
  `icacls`.
- `examples/python/two_clients.py`.

## Amendment (P9.4, 2026-10-28): enforced on the simulator, and the desktop's secret

- **`config/sim.toml` enforces,** now that the web app and the desktop log in (ADR-0068). The
  examples log in by `SIQ_USER` and `SIQ_PASSWORD`.
- **The local console's user is the service's own:** made at start if it is missing, as an
  operator, with a random password that no one is told. It logs in only by the secret.
- **The desktop shell reads the secret:**
  - `GET /api/siq/auth/local` names the file, to this computer only: `{"file"}`.
  - The shell reads it only if it is called `local-login`, and gives the page back only 64
    hexadecimal digits.
  - The shell runs as the interactive user, as the service does in development. The service run
    as another account (M15) adds that user's entry to the file's access list.
- `crates/siq-server/tests/authz.rs`: the file named to this computer only, and the local user an
  operator; `apps/spectrumiq-desktop`'s `server` tests: no other file read.
