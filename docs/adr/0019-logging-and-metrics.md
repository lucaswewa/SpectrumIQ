# ADR-0019: Logging and metrics conventions

- **Status:** Accepted
- **Date:** 2026-10-23
- **Phase:** P0.3a
- **Related:** ADR-0001, ADR-0005, REQ-ACQ-005, REQ-NFR-003, REQ-NFR-004

## Context

The service runs for hours, unattended, as a Windows service. Its logs must survive restarts and
be useful after the fact. Its health needs numbers, not only log lines:

- frames acquired and lost;
- pool occupancy;
- preview latency;
- job queue depth.

Some of those numbers change on the frame path, at 9 fps and faster, where nothing may block or
allocate (ADR-0005).

## Decision

### Logging

- **`tracing` everywhere,** with structured fields (`slots = 12`), not formatted strings. Spans name
  workflows, jobs and requests.
- **`siq_telemetry::init(&LogConfig)`** sets up the process once:
  - **console:** standard error, with colours;
  - **files:** daily rolling `spectrumiq.YYYY-MM-DD.log` in `LogConfig.directory`, without
    colours, through a non-blocking writer, keeping `keep_days` files (default 30).
- **The filter** is `LogConfig.filter`, in `RUST_LOG` syntax (default `info`). The `RUST_LOG`
  environment variable overrides it, so a developer can raise verbosity without editing the
  configuration.
- **The log folder is created by `init`.** `tracing-appender` would otherwise try to prune it
  before creating it, and print an error.
- **The service holds the `LogGuard`** returned by `init` until it exits, so the last lines are
  flushed.
- **Levels:**
  - `error`: an operation failed and needs a person;
  - `warn`: degraded but continuing, such as a lost frame or a retry;
  - `info`: lifecycle events (start, stop, scan begun or finished);
  - `debug`: per-operation detail;
  - `trace`: protocol bytes (serial TX/RX).

### Metrics

- **One `Arc<Registry>` per service.** Components fetch or create their metrics by name once, then
  update them through their `Arc`:
  - `Counter::add` and `Gauge::set` are one atomic operation;
  - `Histogram::record` is one atomic increment plus a compare-and-swap on the sum.

  No lock and no allocation, so the frame path can use them.
- **Names** are lowercase, dot-separated paths: `<component>.<quantity>[_<unit>]`, such as
  `camera.frames_lost` and `preview.latency_ms`. There are no labels. A second instance gets its
  own name part, such as `camera.0.frames_lost`.
- **Histograms** have fixed, strictly ascending bounds, chosen by their owner. Bucket `i` counts
  values up to `bounds[i]`, inclusive. A last, unbounded bucket (`le: null` in JSON) counts the
  rest. NaNs are ignored.
- **Conflicts are errors.** Asking for an existing name as another kind, or a histogram with other
  bounds, is an error, never a silent second metric.
- **`Registry::snapshot()`** is a JSON-serializable map of name to `{kind, …}`. The service serves
  it at `/api/siq/system/metrics` (P2.4).

## Alternatives considered

- **The `metrics` crate's facade with a Prometheus exporter:** global recorders and label maps
  allocate per call. Prometheus isn't in the architecture. A JSON endpoint is enough, and an
  exporter can be added later over the snapshot.
- **Logging with `log` and `env_logger`:** no structured fields or spans, which teta-wot already
  uses through `tracing`.

## Consequences

- Every crate logs through `tracing`. Only the binaries call `init`.
- Metric names are part of the API seen by the UI and tests, so renaming one is a deliberate
  change.

## Verification

- `siq-telemetry` tests:
  - concurrent updates are counted exactly across 8 threads;
  - bucket edges;
  - name, kind and bounds conflicts;
  - the snapshot's JSON, pinned by insta;
  - logging to a daily file without colour codes, and a second `init` refused.
- The `metrics_demo` example.
