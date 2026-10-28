# siq-auth

Who is calling (ADR-0065), and who operates (ADR-0066): local accounts with argon2id hashes and a
role each, browser sessions that end when idle or old, a limit on failed logins, the same-origin
check that mutating requests pass, the Hardware Control Token, and the local console's login.
Which role may do what, route by route, is `siq-server`'s (`authz`).

- **Accounts** (`UserStore`): a JSON file, `users.json` in `[server] settings`.
  - Each account has a name, a role (`viewer`, `operator` or `admin`), and an argon2id hash as a
    PHC string. The hash uses v19, m = 19 MiB, t = 2, p = 1, OWASP's first recommendation, and a
    16-byte random salt.
  - An unknown name is checked against a decoy hash, so a login takes as long either way.
  - The file is written whole and renamed into place. It is read again when its time stamp
    changes, or is under 2 s old, since two writes within Windows's 15 ms tick share one; so the
    CLI's changes reach a running service.
  - Names are 1 to 64 letters, digits, `.`, `_` or `-`; passwords have at least 8 characters; the
    last admin can't be removed or demoted.
- **Sessions** (`SessionStore`): in memory, each known by 32 random bytes from the operating
  system (`getrandom`). A session ends after its idle time (an hour by default) or its lifetime
  (12 hours) however busy. A session's `Debug` shows only its first digits.
- **The cookie** (`set_cookie`, `session_cookie`, `clear_cookie`): `siq_session`, `HttpOnly`,
  `SameSite=Strict`, `Path=/`, its `Max-Age` the session's lifetime. `<img>`, `fetch` streams and
  WebSockets send it without a script's help.
- **Failed logins** (`LoginLimiter`): a name may fail 5 times within 5 minutes, and an address
  four times as often. Past that, logins wait until the oldest failure leaves the window, even
  with the right password; a success forgets the name's failures.
- **Origins** (`same_origin`): a request with an `Origin` comes from the service's own pages if the
  origin's host and port are the request's `Host`, or the origin is allowed. One without an
  `Origin` isn't a browser's cross-site request. `null` is refused.
- **The control token** (`ControlManager`): at most one session holds it. It is requested
  (refused, with its holder, while another holds it), released, or taken over. A holder silent
  for the timeout (30 s by default) loses it, and a session's end lets it go; nothing it started
  is stopped. Its state, and why it last changed, is a `watch` channel.
- **The local console** (`LocalLogin`): a 256-bit secret in a file whose access list holds one
  entry, the service's account, with nothing inherited. Each secret works once: the exchange
  writes the next, into a file restricted before it holds anything.
- **The CLI** (`cli::run`): the `spectrumiq-server user` commands, apart from their parsing. The
  first user is an admin. Passwords are asked for twice through `read_password`, without echo at
  a console, or as lines from a pipe.

**Unsafe code** (ADR-0007):

- `console` calls `GetStdHandle`, `GetConsoleMode` and `SetConsoleMode`, to turn the console's
  echo off while a password is read, and back on;
- `local` calls `OpenProcessToken` and `GetTokenInformation` for the process's account,
  `SetEntriesInAclW` for an access list of it alone, and `SetNamedSecurityInfoW` to give the
  local secret's file that list, protected from inheritance.

## Place in the architecture

- **Layer:** L4 Engines.
- **Depends on:** `siq-types` (the `Clock`); `argon2`, `getrandom`, `serde`, `serde_json`,
  `schemars`, `chrono`, `thiserror`, `tokio` (`watch`), `windows-sys` (the console, the token's
  account and the access list); `tempfile` in the tests.
- **Used by:** `siq-server` (the routes and the layer); `spectrumiq-server` (the `user`
  commands).
- **Phase:** P9.1; P9.2 added the control token and the local console's login. P9.3 adds the
  audit.

## API

| Item | What |
|---|---|
| `UserStore::open(path)` | The users of `path`, or none until one is added |
| `store.add(name, password, role)`, `set_password`, `set_role`, `remove` | Change the users: `User` or a `UserError` (`Name`, `Password`, `Exists`, `Unknown`, `LastAdmin`, `Io`, `Format`, `Hash`) |
| `store.verify(name, password)` | The `User` if the password is theirs, `None` otherwise |
| `store.list()`, `get(name)`, `is_empty()`, `path()` | The users, by name |
| `User` | `name`, `role`, `created`, `changed` |
| `Role` | `Viewer` < `Operator` < `Admin`, as `viewer`, `operator`, `admin` |
| `SessionStore::new(clock, policy)` | No sessions; `SessionPolicy { idle, absolute }`, an hour and 12 hours by default |
| `sessions.create(&user, address)`, `create_local` | A new `Session`: `id`, `user`, `role`, `address`, `local`, `created`, `created_utc`, `seen` |
| `sessions.touch(&id)` | The live session, its idle time begun again, or `None` |
| `sessions.remove(&id)`, `end_user(name)`, `set_role(name, role)`, `list()`, `expires(&session)` | Logouts, a removed user, a new role, the live sessions, when one ends |
| `SessionId::random()`, `parse(text)`, `as_str()` | 64 hexadecimal digits |
| `LoginLimiter::new(clock, max_failures, window)` | `wait(name, address)`: how long to wait, if at all; `failed(name, address)`, `succeeded(name)` |
| `same_origin(origin, host, allowed)` | Whether a mutating request comes from the service's own pages |
| `set_cookie(&id, max_age)`, `session_cookie(header)`, `clear_cookie()`, `COOKIE` | The session cookie |
| `ControlManager::new(clock, timeout)` | The token, free |
| `control.request(&session)`, `release(&id)`, `take_over(&session)` | `Holder` (`user`, `since`, `local`, `session`), or a `ControlError` (`Held(holder)`, `NotHolder`) |
| `control.heartbeat(&id)`, `holder()`, `session_ended(&id)`, `expire()`, `watch()` | A sign of life, saying whether the session holds it; its holder; a logout; a timeout's check; its `ControlState` (`holder`, `reason`: `start`, `requested`, `released`, `taken_over`, `timed_out`, `ended`) as it changes |
| `LocalLogin::create(path)`, `exchange(secret)`, `path()` | A fresh secret in a private file; whether a secret is the current one, the next written if so |
| `cli::run(&store, &command, ask, out)` | Runs a `UserCommand` (`Add`, `Passwd`, `Role`, `List`, `Remove`); `CliError` |
| `read_password(prompt)` | A line from standard input, not echoed at a console |
| `USERS_FILE`, `MIN_PASSWORD` | `users.json`; 8 |

## Examples

| Example | What it shows | Run |
|---|---|---|
| `user_cli` | A scripted session of the user commands: the first admin, an operator and a viewer, a role and a password changed, a removal, the refusals of the last admin's removal and of a short password; then a login, its session, and failed logins past the limit | `cargo run -p siq-auth --example user_cli [-- --keep <folder>]` |

## Features

None.

## Testing

`cargo test -p siq-auth`:

- `users`: users added, verified, changed and removed; the file holding argon2id at
  `m=19456,t=2,p=1` with a 16-byte salt and a 32-byte hash, and no password; another process's
  changes read again; the last admin kept; bad names and short passwords refused;
- `control`, on a paused clock: the token requested, refused to another with its holder,
  released, taken over by an admin, and let go at a logout, each change watched; kept by a
  heartbeat every 25 s, lost after 30 s of silence; a local session's token marked local; the
  local secret used once, and its file's single entry, this account's, with no inherited ones,
  read by `icacls`;
- `sessions`, on a paused clock: a session ending after half an hour idle, and after two hours
  however busy; a user's sessions ended, and given its new role; five failures in two and a half
  minutes waiting for the first to leave the window, another name passing, an address allowed
  twenty; the same-origin rule's cases; the cookie's attributes, and its parsing.

`argon2` and `blake2` are optimised in dev builds, so a hash takes milliseconds in the tests too.

## Limitations

- **Sessions don't survive a restart:** everyone logs in again.
- **No `Secure` cookie:** the service speaks plain HTTP on the LAN; behind a TLS proxy the cookie
  still travels in the clear between the proxy and the service.
- **Two processes changing the users at once** could lose one change: the CLI and the service's
  admin routes (P9.4) are rarely used together.
