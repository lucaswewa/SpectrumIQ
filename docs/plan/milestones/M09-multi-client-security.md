# M9 — Multi-client, accounts, control token and audit

**Outcome:** many concurrent clients on the LAN:

- local user accounts and roles;
- one active operator holding the Hardware Control Token;
- read-only viewers;
- a tamper-evident audit trail.

The desktop logs in locally without a password prompt.

**Exit criteria:**

- every route in the OpenAPI document and every SpectrumIQ route is classified, and the
  authorization matrix tests pass for every role;
- two browsers plus the desktop operate concurrently, with the token handed over;
- a 30-minute soak with 8 viewers and 1 operator shows no lost camera frames and preview latency
  within target.

---

### P9.1 — Accounts and sessions (`siq-auth`)

**Budget** ~850 · **Depends on** P2.3 · **Requirements** REQ-SEC-001

**Goal.** Who is calling, with browser-friendly sessions.

**Scope**

- **`UserStore`:** a JSON file in `%ProgramData%\SpectrumIQ\config`, argon2id hashes, and roles
  Viewer, Operator and Admin.
- **`SessionStore`:** in memory, with idle and absolute expiry.
- **Routes:** `/api/siq/auth/{login,logout,me}`. The session cookie is `HttpOnly`,
  `SameSite=Strict`, `Path=/`.
- **Protections:** an `Origin` check on mutating requests, and rate limiting on login attempts.
- **CLI:** `spectrumiq-server user add|passwd|role|list|remove`, and bootstrapping the first admin.

**Tests**

- Login and logout.
- Expiry under paused time.
- Rate limiting.
- Origin check.
- Hashing parameters.

**Example.** `user_cli`, a scripted session.

**ADR.** Authentication and sessions: cookies, because `<img>`, fetch streams and WebSockets carry
them.

---

### P9.2 — Authorization layer and Hardware Control Token

**Budget** ~850 · **Depends on** P9.1, P6.5 · **Requirements** REQ-SEC-002, REQ-SEC-004

**Goal.** Enforce the matrix of architecture §10, and keep a single active operator.

**Scope**

- **axum middleware around the whole router.** A route classification table (read, hardware,
  stop, processing, admin) covers teta-wot routes by Thing and method, and our own routes.
  teta-wot's WebSocket is observe-only, so a WS upgrade is a read.
- **`ControlManager`:**
  - request, release, and take-over (Admin or local console);
  - holder heartbeat over the client's WS;
  - auto-release after 30 s of silence;
  - a running workflow keeps running.
- **`control` Thing:** `holder` and `since` properties, observed; `changed` event.
- **Local console login (`/api/siq/auth/local`):**
  - the service writes a one-time secret to `%ProgramData%\SpectrumIQ\local-login`;
  - the file's ACL allows only the interactive user and the service (`windows-sys`
    `SetNamedSecurityInfoW`);
  - the Tauri host reads the secret and exchanges it for a session as the configured local user.

**Tests**

- Every route × every role. The test walks the OpenAPI document and fails on an unclassified
  route.
- Token handover and timeout.
- STOP is allowed without the token.
- Local-login secret rotation.

**Example.** `examples/python/two_clients.py`: a viewer is refused, the operator succeeds, then the
token is handed over.

**ADR.** The authorization matrix and the Hardware Control Token.

---

### P9.3 — Audit trail

**Budget** ~600 · **Depends on** P9.2 · **Requirements** REQ-SEC-003, REQ-NFR-006

**Goal.** A tamper-evident record of who did what to the instrument and its data.

**Scope**

- **`AuditLog`:** JSON Lines with `prev_hash` and `hash` (SHA-256, `sha2`), rotated monthly.
- **Events:**
  - logins and logouts;
  - token changes;
  - every mutating request (user, session, IP, route, parameter summary, outcome);
  - workflow and job lifecycles;
  - dataset deletions.
- **`spectrumiq-server audit verify`.**
- **Admin query endpoint,** with paging and filters.

**Tests**

- Chain verification.
- Tamper detection: edit, delete, and reorder.
- Rotation continuity.

**Example.** `audit_verify`.

**ADR.** The audit trail format, kept IVD-ready.

---

### P9.4 — Frontend: login, roles, control token and admin

**Budget** ~900 · **Depends on** P9.2, P9.3, P3.5 · **Requirements** REQ-SEC-001, REQ-SEC-002, REQ-VIS-005

**Goal.** Multi-client UX that stays out of the way.

**Scope**

- **Login page** and session store, with 401 handling and redirect.
- **Role-aware controls:** disabled with an explanation, rather than hidden, for viewers.
- **Control token bar:** the holder; request, release, and take-over; banners when someone else is
  operating.
- **Admin pages:** user CRUD, and an audit viewer built on a new `AppDataTable` wrapper (TanStack
  Table, headless, styled with the tokens: ADR-0008, amended in P3.1).
- **Tauri local auto-login** through the `HostAdapter`.

**Tests**

- Store and role gating.
- Token bar states.
- `AppDataTable` contract.

**Example.** `/#/dev/admin`.

**ADR.** Multi-client UX rules.

---

### P9.5 — End-to-end tests and the soak tool

**Budget** ~500 (mostly tests) · **Depends on** P9.4 · **Requirements** REQ-NFR-003, REQ-NFR-004, REQ-SEC-002

**Goal.** Prove concurrency and the connection-budget design under load.

**Scope**

- **Playwright e2e** (following `microscope`'s setup) against the simulated server:
  - login;
  - live view;
  - two browser contexts handing over control;
  - token timeout;
  - the connection-budget scenario: 3 MJPEG streams, WS observations, and deep-zoom tiles loading
    at the same time.
- **Soak tool (`xtask soak`):** N viewer clients (MJPEG, WS and tiles) plus an operator running
  scans for T minutes. It records preview latency, frame skips, lost camera frames, and server
  CPU and RAM.

**Tests.** The e2e suite (opt-in, `cargo xtask ci --e2e`).

**Example.** `cargo xtask soak --viewers 8 --minutes 30` (smoke: 2 viewers for 20 s).

**ADR.** The e2e and soak strategy, with its targets.
