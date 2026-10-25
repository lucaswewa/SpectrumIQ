# spectrumiq-server

The SpectrumIQ service: the instrument's Things and routes, served over HTTP, until Ctrl-C or the
system asks it to stop. The service itself is [`siq-server`](../../crates/siq-server/README.md).

```text
spectrumiq-server [run] -c config/sim.toml   serve the instrument; run is the default
    [--port P]                               on port P instead of the configuration's
    [--webapp-dir web/dist]                  the web app from a folder, not the build compiled in
spectrumiq-server --print-openapi            the Things' OpenAPI document, for client types
spectrumiq-server --print-config-schema      the configuration's JSON Schema
```

- **Where it listens:** `[server] bind` and `port`, 127.0.0.1:5000 by default, or `--port`.
- **The web app** is at `/` (ADR-0036). A release build embeds `web/dist`, so build the web app
  first (`npm run build` in `web/`). Without a build, `/` says how to make one.
- **Logging:** `[log]`, to the console and to daily files.
- **Shutting down** stops live view, then motion, then the Things.
- **Exit codes:** 0 on a clean stop, 1 on an error, such as a configuration or a device that won't
  open. The message says which.

With the simulated instrument, from the repository:

```text
cargo run --release -p spectrumiq-server -- -c config/sim.toml
python examples/python/live_and_move.py
```

The script moves the stage, and saves an overview frame before and after.

## Place in the architecture

- **Layer:** App.
- **Depends on:** `siq-server`, `siq-config`, `siq-telemetry`, `clap`, `tokio`, `anyhow`.
- **Phase:** P2.3; P3.1 added the web app, `--port` and `--webapp-dir`. The Windows service mode
  follows in P15.1.
