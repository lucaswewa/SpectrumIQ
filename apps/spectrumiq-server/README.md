# spectrumiq-server

The SpectrumIQ service: the instrument's Things and routes, served over HTTP, until Ctrl-C or the
system asks it to stop. The service itself is [`siq-server`](../../crates/siq-server/README.md).

```text
spectrumiq-server [run] -c config/sim.toml   serve the instrument; run is the default
    [--port P]                               on port P instead of the configuration's
    [--webapp-dir web/dist]                  the web app from a folder, not the build compiled in
spectrumiq-server --print-openapi            the Things' OpenAPI document, for client types
spectrumiq-server --print-config-schema      the configuration's JSON Schema
spectrumiq-server user add <name> [--role R] -c config/sim.toml
                                             a user; an admin while there is none
spectrumiq-server user passwd <name>         a new password
spectrumiq-server user role <name> <role>    viewer, operator or admin
spectrumiq-server user list | remove <name>  the users; the last admin stays
spectrumiq-server audit verify -c config/sim.toml
                                             the audit trail's chain checked: exit code 1 if broken
```

- **Where it listens:** `[server] bind` and `port`, 127.0.0.1:5000 by default, or `--port`.
- **The web app** is at `/` (ADR-0036). A release build embeds `web/dist`, so build the web app
  first (`npm run build` in `web/`). Without a build, `/` says how to make one.
- **Logging:** `[log]`, to the console and to daily files.
- **Users** (ADR-0065) are in `users.json` in `[server] settings`. A password is asked for twice,
  without being shown, or read as two lines of standard input from a pipe. The service warns at
  start while there is no admin, and reads the file again when the commands change it. It makes
  `[auth] local_user` itself, an operator who logs in only by the local secret (ADR-0066); an
  admin manages the rest from the web app's Admin page too (ADR-0068).
- **Shutting down** stops live view, then motion, then the Things.
- **Exit codes:** 0 on a clean stop, 1 on an error, such as a configuration or a device that won't
  open. The message says which.

With the simulated instrument, from the repository. `config/sim.toml` asks for a login, so add a
user first, an admin while there is none, and give the script its name and password:

```text
cargo run --release -p spectrumiq-server -- user add ada -c config/sim.toml
cargo run --release -p spectrumiq-server -- -c config/sim.toml
set SIQ_USER=ada& set SIQ_PASSWORD=<its password>
python examples/python/live_and_move.py
```

The script logs in, takes control of the instrument, moves the stage, and saves an overview frame
before and after. The web app at <http://127.0.0.1:5000/> asks for the same login.

## Place in the architecture

- **Layer:** App.
- **Depends on:** `siq-server`, `siq-auth`, `siq-config`, `siq-telemetry`, `clap`, `tokio`,
  `anyhow`.
- **Phase:** P2.3; P3.1 added the web app, `--port` and `--webapp-dir`; P9.1 the `user`
  commands; P9.3 `audit verify`; P9.4 the local console's user; P10.1 the `euresys` feature. The
  Windows service mode follows in P15.1.

## Features

- **`euresys`:** the Euresys camera, through `siq-server`'s feature of that name:
  `cargo run --release -p spectrumiq-server --features euresys -- -c <configuration>`, with
  `[camera] kind = "euresys"` and its feature map (`config/cameras/`). It needs the eGrabber SDK.
