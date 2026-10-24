# spectrumiq-server

The SpectrumIQ service: the instrument's Things and routes, served over HTTP, until Ctrl-C or the
system asks it to stop. The service itself is [`siq-server`](../../crates/siq-server/README.md).

```text
spectrumiq-server [run] -c config/sim.toml   serve the instrument; run is the default
spectrumiq-server --print-openapi            the Things' OpenAPI document, for client types
spectrumiq-server --print-config-schema      the configuration's JSON Schema
```

- **Where it listens:** `[server] bind` and `port`, 127.0.0.1:5000 by default.
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
- **Phase:** P2.3. The Windows service mode follows in P15.1.
