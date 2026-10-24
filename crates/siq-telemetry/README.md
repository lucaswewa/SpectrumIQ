# siq-telemetry

SpectrumIQ's logging setup and metrics registry:

- `init` sends `tracing` output to the console and to daily log files;
- `Registry` holds named counters, gauges and histograms, cheap enough for the frame path, with a
  JSON snapshot.

The conventions are in [ADR-0019](../../docs/adr/0019-logging-and-metrics.md).

## Place in the architecture

- **Layer:** L0 Foundation.
- **Depends on:** `tracing`, `tracing-subscriber` (`fmt`, `env-filter`), `tracing-appender`,
  `serde`, `schemars`, `thiserror`.
- **Used by:**
  - `siq-config` embeds `LogConfig` as the `[log]` section;
  - the service and tools call `init`;
  - every component uses `Registry`.
- **Phase:** P0.3a.

## API

| Item | What |
|---|---|
| `LogConfig` | `filter` (`RUST_LOG` syntax, default `info`; the `RUST_LOG` variable overrides it), `console`, `directory` (daily files `spectrumiq.YYYY-MM-DD.log`), `keep_days` (default 30) |
| `init(&LogConfig) -> Result<LogGuard, TelemetryError>` | Sets up the process's subscriber, once. Hold the `LogGuard` until exit: dropping it flushes the files |
| `Registry` | `counter(name)`, `gauge(name)`, `histogram(name, bounds)` fetch or make a metric. `snapshot()` gives every value |
| `Counter`, `Gauge`, `Histogram` | `add` / `increment`, `set` / `get`, and `record`. Each is one atomic update, with no lock and no allocation |
| `MetricValue` | A snapshot entry, serialized as `{"kind": "counter" \| "gauge" \| "histogram", …}` |

```rust
let registry = Arc::new(Registry::new());
let lost = registry.counter("camera.frames_lost")?;          // once, at start-up
let latency = registry.histogram("preview.latency_ms", &[10.0, 20.0, 40.0, 80.0])?;
lost.increment();                                             // on the frame path
latency.record(23.5);
let json = serde_json::to_string(&registry.snapshot())?;
```

## Examples

| Example | What it shows | Run |
|---|---|---|
| `metrics_demo` | Logs to the console and to a daily file in the temp folder. A simulated camera and preview update metrics from two threads, and the JSON snapshot is printed. It exits with an error if a count is wrong | `cargo run -p siq-telemetry --example metrics_demo` |

## Features

None.

## Testing

`cargo test -p siq-telemetry` covers:

- **Unit tests** in `src/metrics/tests.rs`:
  - 8 threads × 10 000 updates, counted exactly;
  - histogram bucket edges and NaN;
  - name, kind and bounds conflicts;
  - the snapshot's JSON (insta).
- **`tests/logging.rs`:**
  - an invalid filter is rejected;
  - logs land in one daily file, with fields and without colour codes;
  - a second `init` is refused.

  It runs in its own test binary, because a process sets its global subscriber only once.

## Limitations

- **Metrics have no labels.** Instances get their own name part.
- **The log filter is fixed at start-up.** Changing it while the service runs would need a
  `reload` layer; add one if it's ever needed.
- **`RUST_LOG` overrides the configured filter.** The logging test skips its invalid-filter check
  when `RUST_LOG` is set.
