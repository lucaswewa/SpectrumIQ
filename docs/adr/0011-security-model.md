# ADR-0011: Security: local accounts, roles, Hardware Control Token, audit; LAN only

- **Status:** Accepted
- **Date:** 2026-10-23
- **Phase:** planning
- **Related:** REQ-SEC-001–004, REQ-NET-004, REQ-NFR-006

## Context

Remote browsers connect from the lab LAN. The goals ask for view-only access by default, and for an
"active operator" lock so that clients don't make conflicting stage moves. IVD work later will need
per-user accountability. Browsers can't add headers to `<img>`, `EventSource` or WebSocket
requests.

## Decision

- **Accounts:**
  - local accounts with argon2id hashes;
  - roles Viewer, Operator and Admin;
  - `HttpOnly`, `SameSite=Strict` session cookies, plus an `Origin` check on mutations.
- **Hardware Control Token:**
  - at most one session holds it;
  - request, release, and take-over (Admin or local console);
  - released after 30 s without a heartbeat;
  - hardware mutations require it;
  - **stop and abort are always allowed** for Operators and the local console.
- **Local console:** logs in through a secret in an ACL-restricted file, without typing a
  password.
- **Enforcement:** an axum layer classifies every route by HTTP method and route class (teta-wot's
  WebSocket is observe-only). The matrix is in architecture §10.
- **Audit trail:** hash-chained JSON Lines of logins, token changes, mutating requests, and
  workflow and job lifecycles, with a `verify` command.
- **Deployment:** LAN only. TLS is optional, through a reverse proxy.

## Alternatives considered

- **Shared role tokens:** no per-user accountability.
- **Windows / Active Directory:** domain-dependent and heavier, and still possible later.

## Consequences

- Every new route must be classified. A test enforces it.
- There is user management to build and maintain (P9.1, P9.4).

## Verification

P9.2: the matrix tests over every route and role. P9.3: chain verification and tamper tests. P9.5:
e2e handover.
