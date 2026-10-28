# ADR-0065: Authentication and sessions

- **Status:** Accepted
- **Date:** 2026-10-27
- **Phase:** P9.1
- **Related:** ADR-0011, ADR-0007, ADR-0003, REQ-SEC-001

## Context

M9 lets many clients on the LAN use the instrument at once (ADR-0011). Before the service can
decide what a caller may do (P9.2), it has to know who is calling.

- The browser shows live view as `<img>` MJPEG streams, reads properties with `fetch`, and
  observes Things over WebSockets. None of these lets a script add an `Authorization` header to
  every request; a cookie rides along with all of them.
- The service runs on one lab workstation, without a directory service. Its users are a few
  people, managed by whoever looks after the instrument.
- The CLI that adds users runs as another process, possibly while the service runs.

## Decision

- **Accounts** (`siq_auth::UserStore`) live in a JSON file, `users.json` in `[server] settings`:
  - each has a name, a role (Viewer, Operator or Admin), and an argon2id hash as a PHC string;
  - the hash uses argon2id v19 with m = 19 MiB, t = 2 and p = 1, OWASP's first recommendation,
    and a 16-byte random salt;
  - an unknown name is checked against a decoy hash, so a login takes as long whether the name
    exists or not;
  - names are 1 to 64 letters, digits, `.`, `_` or `-`; passwords have at least 8 characters;
  - the file is written whole to a temporary file and renamed over the old one, and read again
    whenever its time stamp changes or is under 2 s old, since Windows keeps file times to the
    system's tick of about 15 ms;
  - the last admin can't be removed or demoted.
- **Sessions** (`SessionStore`) are in memory:
  - each is known by 32 random bytes from the operating system, as 64 hexadecimal digits;
  - a session ends after `idle_minutes` without a request (60) or `session_hours` (12) however
    busy;
  - a restart ends them all.
- **The cookie** is `siq_session=<id>; HttpOnly; SameSite=Strict; Path=/; Max-Age=<12 h>`:
  - `HttpOnly`: no script can read it;
  - `SameSite=Strict`: another site's page can't make the browser send it;
  - no `Secure`: the service speaks plain HTTP on the LAN; TLS is a reverse proxy's (ADR-0011).
- **Routes:**
  - `POST /api/siq/auth/login` with `{ name, password }` answers 200 with who the session is and
    sets the cookie, or 401 for a wrong name or password, the same either way;
  - `POST /api/siq/auth/logout` ends the session and clears the cookie;
  - `GET /api/siq/auth/me` answers who the session is and when it ends, or 401.
- **Failed logins** (`LoginLimiter`):
  - a name may fail `max_failures` (5) times within `failure_minutes` (5);
  - an address may fail four times as often, since one lab computer serves many people;
  - past either limit, a login is refused with 429 and `Retry-After`, even with the right password,
    until the oldest failure leaves the window;
  - a success forgets the name's failures.
- **One layer around every route**, teta-wot's included:
  - it refuses a request other than `GET`, `HEAD` or `OPTIONS` with 403 when its `Origin`
    differs from its `Host`, unless the origin is in `allowed_origins`;
  - a request without an `Origin` passes: it isn't a browser's cross-site request;
  - a request with a live session's cookie carries its `Session` in its extensions;
  - each request reads the user's role again, so the CLI's changes take effect at once, and a
    removed user's sessions end.
- **The CLI:** `spectrumiq-server user add|passwd|role|list|remove`. The first user added is an
  admin. A password is asked for twice, without echo at a console, or read as two lines from a
  pipe. The no-echo console is a small Windows-API module (ADR-0007).
- **No enforcement yet:** P9.1 tells who is calling. Which role may do what, and the control token,
  are P9.2's.

## Alternatives considered

- **Bearer tokens in `Authorization`:** `<img>`, `EventSource` and WebSockets can't send them.
  Putting the token in the URL instead leaks it into logs and the history.
- **Signed cookies holding the session's state:** a logout or a removed user couldn't take effect
  before the cookie expired, and nothing needs sessions to outlive a restart.
- **Windows accounts or Active Directory:** this depends on a domain, and is still possible later
  (ADR-0011).
- **bcrypt or PBKDF2:** argon2id resists GPU guessing better, and RustCrypto's crate is pure Rust.
- **The `cookie` crate:** one name and one value need a few lines of parsing.

## Consequences

- **Every browser request can be attributed,** to a user and an address: P9.2 authorizes by it, and
  P9.3 audits it.
- **A login costs about 13 ms of a processor** (measured on the workstation, a wrong password
  included, as it hashes in full). It is checked on a blocking thread, not the runtime's. `argon2` and `blake2` are optimised in dev builds too, so the tests hash at the
  service's cost.
- **A browser page served from elsewhere,** such as a development server that doesn't proxy, needs
  its origin in `allowed_origins`. Vite's proxy keeps the `Host`, so it needs none.
- **A passive viewer** watching only an MJPEG stream for an hour, with no other request, is logged
  out; the web app's own requests (P9.4) keep a session alive.

## Verification

- `crates/siq-auth/tests/users.rs`: users added, verified, changed and removed; the hash's
  parameters, salt and length; another process's changes read again; the last admin kept; names
  and short passwords refused.
- `crates/siq-auth/tests/sessions.rs`: idle and absolute expiry on a paused clock; a user's
  sessions ended and given its new role; failures past the limit waiting for the window, an
  address allowed four times as many; the same-origin rule; the cookie.
- `crates/siq-server/tests/auth.rs`: a login's cookie, `me` and a logout; wrong passwords, then the
  limit with `Retry-After`; another site's changes refused on SpectrumIQ's and teta-wot's routes;
  the CLI's changes reaching a session at once.
- `siq-auth --example user_cli`.

## Amendment (P9.4, 2026-10-28): the first admin, and the web app's login

- **While there is no admin, a user added is one.** The service makes the local console's user
  itself (ADR-0066, amended), so the first user added was no longer the first user. The service
  warns at start while there is no admin.
- **`GET /api/siq/auth/me`'s 401 says `"enforced"`:** the web app asks for a login only where the
  service enforces one (ADR-0068).
- **The users over HTTP, an admin's:** `GET` and `POST /api/siq/users`, and `PUT` and `DELETE
  /api/siq/users/{name}`. A new password and a removal end the user's sessions; a new role
  applies to them at once. The last admin stays, as with the CLI.
