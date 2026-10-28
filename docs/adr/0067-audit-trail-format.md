# ADR-0067: The audit trail format, kept IVD-ready

- **Status:** Accepted
- **Date:** 2026-10-27
- **Phase:** P9.3
- **Related:** ADR-0011, ADR-0065, ADR-0066, ADR-0001, REQ-SEC-003, REQ-NFR-006

## Context

With many clients on the LAN (ADR-0066), the lab needs to know who did what to the instrument and
its data, and to trust that the record wasn't changed afterwards. IVD work later will need the
same (ADR-0001). The service's log is for diagnosis: it rotates, is filtered by level, and anyone
with the file can edit it.

## Decision

- **JSON Lines, a file a month,** `YYYY-MM.jsonl` in `[server] audit` (`audit` beside the
  configuration by default; `%ProgramData%\SpectrumIQ\audit` when installed). Each line is one
  entry: `seq`, `time` (UTC), `kind`, `user`, `session` (its first 8 digits: a full identifier is
  a credential), `address`, `detail`, `prev_hash`, `hash`.
- **The chain:** `hash` is the SHA-256 of the line's own text without `,"hash":"…"`, and that text
  holds the line before's `hash` as its `prev_hash`; the first entry's is 64 zeros. Verifying
  needs no re-serialisation, only the line. A month's first entry chains to the last month's last.
- **`spectrumiq-server audit verify`** checks every file in order: each line hashes to its `hash`,
  holds the previous `hash`, and is numbered one after it. It names the first file and line that
  fail, and exits with 1. It prints the last hash, to keep elsewhere.
- **What is recorded:**
  - logins, by password or the local console's secret, and failed and refused ones;
  - logouts;
  - every change of the control token's hands, with the holder before and after and why;
  - every request that would change something: its method, path, query, class, parameters and
    status; its parameters are its JSON body with strings cut to 120 characters, or its size and
    keys past 4 KB;
  - every such request refused: by its origin, its caller's role or a missing token;
  - every job's transitions: queued, started, and done, failed or cancelled;
  - every workflow run's start and end, with its dataset, frames and error;
  - every dataset deleted.
- **What isn't:** reads; the logins' own requests, whose bodies hold passwords; the token's
  heartbeat; and the live mosaic's viewport, which changes by the second and keeps nothing.
- **Written by a thread of its own:** recording doesn't wait. The thread numbers, chains and writes
  each entry, synced to disk before the next. Each entry is timed when it is recorded, not when it
  is written.
- **Every transition, none coalesced:** the token's keeper records its own changes, and the
  scheduler tells listeners of each job's transitions (`Scheduler::on_transition`). A `watch`
  channel keeps only the latest value, and missed quick changes when the trail was fed by one.
- **`GET /api/siq/audit?from&to&user&kind&offset&limit`:** the entries a filter takes, newest
  first, and their total; an admin's.

## Alternatives considered

- **The service's log, with a field for the audit:** it rotates by size and level, and isn't
  chained.
- **A database, such as SQLite:** a native library and a schema to migrate, for a record that is
  only appended and read in order.
- **Signing each entry,** or the head, with a key: it needs a key the service guards. A key on the
  same computer adds little against someone who can edit its files, and the chain already shows
  edits. Signing can come with IVD, against an external timestamp.
- **Hashing re-serialised JSON:** a parser's choices, such as a float's formatting or the keys'
  order, would change the bytes. Hashing the text as written can't drift.

## Consequences

- **An edit, a deletion or a reordering shows,** at its line. Truncating the newest entries
  doesn't, unless the last hash was kept elsewhere: `verify` prints it for that.
- **A crash mid-line** leaves a line without its hash; `verify` reports it.
- **The trail grows** with the changes made: a scan's run is a few entries, a busy day of
  operation a few thousand, about 370 bytes each.
- **Disk space:** the trail isn't rotated away; keeping or archiving old months is the lab's.

## Verification

- `crates/siq-auth/tests/audit.rs`:
  - entries chained and verified, and queried by user, kind and time, newest first, in pages;
  - an edit, a removal, a reordering, and an edit with its hash recomputed, each found at its
    line;
  - a month's file chaining to the last month's, across a restart too, and a missing month found.
- `crates/siq-server/tests/audit.rs`: logins, a failed one, the token's request and end, a change
  with its parameters, a refusal, a logout, a job's end and a workflow's run all recorded; the
  query an admin's; no password anywhere; the chain intact.
- `crates/siq-jobs/tests/scheduler.rs`: `every_transition_is_told_as_it_happens`.
- `siq-auth --example audit_verify`.

## Amendment (P9.4, 2026-10-28): a read by its class, and the users' passwords

- **A read is told by its class, not its method:** a `POST` that reads, such as a dataset's
  summary or a scan's estimate (`Class::Read`, ADR-0066), isn't recorded. The Datasets page asks
  for a summary each time it opens, by anyone.
- **The users routes are recorded** (ADR-0065, amended) with any `password` or `secret` in their
  parameters cut to "…".
- `crates/siq-server/tests/audit.rs`: a viewer's summary not recorded; `tests/authz.rs`: the users'
  changes recorded, and not their passwords.
