# siq-config

SpectrumIQ's instrument configuration: one TOML file describes the instrument.

- `load` checks it and reports every problem with its line and column.
- Omitted keys take their defaults.
- Relative paths are resolved against the file's folder.

The format and its layering are in
[ADR-0020](../../docs/adr/0020-instrument-configuration.md).

## Place in the architecture

- **Layer:** L0 Foundation.
- **Depends on:**
  - `siq-types`, for units, ids and the pixel format;
  - `siq-telemetry`, whose `LogConfig` is `[log]`;
  - `toml`, `serde`, `schemars`, `serde_json`, `thiserror`.
- **Used by:** `spectrumiq-server` (P2.3) builds the service from it. Every device phase extends
  its kind's section.
- **Phase:** P0.3b.

## API

| Item | What |
|---|---|
| `load(path)` | Reads, checks and resolves a file |
| `from_toml(text, origin, base)` | The same, from text. `origin` names it in messages; `base` resolves relative paths |
| `InstrumentConfig` and its sections | The configuration, with defaults filled in (`model` module) |
| `ConfigError`, `Problem` | Every problem, displayed one per line as `<file>:<line>:<column>: <path>: <message>` |
| `json_schema()` | The JSON Schema of the format |

## The format

The shipped files are the reference:

- [`config/sim.toml`](../../config/sim.toml), the simulated instrument with every section;
- [`config/hardware.example.toml`](../../config/hardware.example.toml), a template for the real
  one.

| Section | Keys (default) |
|---|---|
| `[server]` | `bind` (`127.0.0.1`), `port` (5000), `data_root` (`data`), `scratch` (`scratch`), `settings` (`settings`, the calibrations, P5.2, and the users, P9.1), `audit` (`audit`, the audit trail, P9.3), `min_free_gb` (20) |
| `[memory]` | `pool_slots` (12), `job_ram_gb` (14), `job_vram_gb` (4), `tile_cache_mb` (1024) |
| `[gpu]` | `enabled` (true), `software` (false), `mosaic_vram_gb` (2): the live mosaic's GPU and its pages' budget |
| `[log]` | `filter` (`info`), `console` (true), `directory` (none), `keep_days` (30) |
| `[auth]` | `enforce` (true: who may do what, and the control token, ADR-0066), `control_timeout_s` (30), `local_user` (none: the user the local console logs in as); `idle_minutes` (60), `session_hours` (12), `max_failures` (5), `failure_minutes` (5), `allowed_origins` (none): sessions and logins (ADR-0065). The users are in `users.json` in `[server] settings`. `config/sim.toml` sets `enforce = false` until the web app logs in (P9.4) |
| `[camera]` | `kind = "sim"`: `width` (11648), `height` (8742), `pixel_format` (`Mono14`), `frame_rate` (9), `pixel_size` (3.76 µm), and `cfa` for a colour sensor, such as `gbrg` (mono if not given, P10.6). `kind = "euresys"`: `interface` (0), `device` (0), `features`, and `strobe`, a pulse on a grabber output line in each triggered exposure: `line`, `delay_ms` (0), `duration_ms` (the exposure) (P10.4) |
| `[stage]` | `kind = "sim"`: `poll_hz` (5). `kind = "grbl"`: `port`, `baud` (115200), `poll_hz` (5). Both: `[stage.motion]`, with `unlock` (true), `approach` (`[1, 1, 1]`, each 1 or −1) and `overshoot` (50 µm); and `[stage.expect]`, the settings the controller must have, by name, such as `"$100" = 1600.0`: one that differs is refused (P10.3) |
| `[focus]` | `kind = "sim"`: `dialect`. `kind = "ascii"`: `port`, `baud` (9600), `dialect` |
| `[[objectives]]` | `id`, `name`, `magnification`, `numerical_aperture`, `immersion` (`air`), `turret_position`, `parfocal_offset` (0 µm) |
| `[[channels]]` | `id`, `name`, `exposure` (ms), `illumination = { source, intensity }`, `filter`, `colour` (`#ffffff`) |
| `[peripherals.*]` | `illumination.sources`, `filter_wheel.filters`, `objective_turret.positions`, `modality.modes`; each with `kind = "sim"` |
| `[sim]` | `seed` (0), `profile` (`full`), `focus_at` (0, 2000 in `config/sim.toml`): where on the simulated focus drive's travel the sample is in focus, and where it starts (P6.5). `[sim.world]`: `kind` (`cells`, …, or `image` with `path` and `pixel_size`). `[sim.errors]`: `backlash`, `scale_error`, `position_noise`, `reply_latency`. `[sim.mount]` (P5.2): the camera's `rotation` in degrees, `scale_error` and `shear` against the stage, all 0 |

**What is checked, beyond the types:**

- ranges: ports, pool slots, rates, numerical apertures, intensities;
- unique ids, names, turret positions and list entries;
- every channel's source and filter exists;
- a turret position needs a turret;
- at least one objective and one channel.

## Examples

| Example | What it shows | Run |
|---|---|---|
| `validate_config` | Checks files and prints each one with its defaults filled in. Without arguments, it checks the shipped files; `--schema` prints the JSON Schema | `cargo run -p siq-config --example validate_config -- config/sim.toml` |

## Features

None.

## Testing

`cargo test -p siq-config` (`tests/config.rs`):

- the shipped files are valid; the simulated one is snapshot;
- paths resolve against the file's folder;
- defaults;
- the JSON Schema, snapshot;
- one snapshot of every kind of problem with its location.

After a deliberate format change, run `INSTA_UPDATE=always cargo test -p siq-config` and review
the snapshot diffs.

## Limitations

- **Validation doesn't touch the filesystem.** A dialect file that doesn't exist is reported by the
  driver that opens it.
- **Peripherals are simulated only,** until their drivers exist (P10.5).
- **Resolved paths keep `..` segments** (`config\../data`). They are correct, and not
  canonicalized, so tests don't depend on the working directory.
