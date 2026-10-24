# ADR-0020: Instrument configuration format and layering

- **Status:** Accepted
- **Date:** 2026-10-23
- **Phase:** P0.3b
- **Related:** ADR-0003, ADR-0012, ADR-0016, ADR-0018, ADR-0019, REQ-NFR-002, REQ-SIM-006

## Context

Something must describe the instrument:

- which devices exist, and whether each is simulated or real (with its port);
- the objectives and channels;
- the peripherals;
- where data goes;
- the memory budget;
- logging;
- the simulation's parameters.

teta-wot has its own JSON configuration (Things and their arguments) and JSON settings files
(runtime values the service persists). People edit the instrument description by hand, at the
instrument, often while bringing hardware up. Mistakes must be found at start-up, with a message
that says where.

## Decision

### Layers

| Layer | Format | Written by | Holds | Changes take effect |
|---|---|---|---|---|
| Instrument configuration | TOML, `siq-config` | People | What the instrument is: devices and kinds, ports, optics, channels, peripherals, paths, memory budget, logging, simulation | At start-up |
| teta-wot settings | JSON, teta-wot | The service | Runtime-tunable values, such as stage speed, default exposure, display settings and saved positions | At once, persisted |
| Calibration registry (P5.2) | Versioned JSON | The service | Calibrations per objective × channel | When a calibration is accepted |
| `RUST_LOG` | Environment | People | A log filter that overrides `[log] filter` | At start-up |

The service builds its `ThingServer` programmatically from the `InstrumentConfig` (P2.3). It does
not use teta-wot's JSON configuration files, which would tie the instrument's description to the
server's internals.

### Format

- **Sections:**
  - `[server]`, `[memory]`, `[log]`;
  - `[camera]`, `[stage]`, `[focus]`, each tagged by `kind` (`sim` | `euresys`, `sim` | `grbl`,
    `sim` | `ascii`), with the kind's own keys;
  - `[[objectives]]`, `[[channels]]`;
  - `[peripherals.illumination | filter_wheel | objective_turret | modality]`, each optional;
  - `[sim]`, with `[sim.world]` and `[sim.errors]`.
- **Simulated stage and focus** run the real drivers against protocol emulators (ADR-0012).
- **Defaults.** Every section and key a working instrument can do without has a default. The
  memory defaults are the 32 GB / 8 GB workstation's (ADR-0005). Defaults appear in the JSON Schema.
- **Paths** are relative to the configuration file's folder.
- **Objective and channel ids** are the numbers the file gives (ADR-0018). Channels refer to
  illumination sources and filters by name, and the names must exist.

### Strictness and messages

- **Unknown keys are errors** (`deny_unknown_fields` everywhere), so a typo never passes as a
  default.
- **Every problem is reported at once,** sorted by position, one per line:

  ```text
  <file>:<line>:<column>: <path>: <message>
  sim.toml:30:6: channels[1].id: channels[0].id has it too
  ```

  - Syntax and type errors take their location from the TOML parser.
  - Rule violations take theirs from the span of the value at their path, found in the parsed
    document. When the value is missing, the deepest part of the path that exists is used.
- **Validation is pure.** It reads no files and opens no devices. Whatever opens a dialect file or
  a port reports its own errors.
- **The JSON Schema** (`siq_config::json_schema()`) is snapshot-tested, and served by
  `spectrumiq-server --print-config-schema` (P2.3) for editors.

### Evolution

New keys get defaults, so older files keep working. Renaming or removing a key needs an amendment
to this ADR, and a note in the release.

## Alternatives considered

- **JSON, as teta-wot:** no comments, and noisy to edit by hand.
- **YAML:** implicit typing (`no` → false, `1.10` → 1.1) and indentation errors in a file people
  edit at the instrument.
- **teta-wot's configuration for everything:** it describes Things and their arguments, not an
  instrument. Kinds, optics and channels would leak into server internals.
- **Stopping at the first problem:** a fix-run-fix loop at the instrument.

## Consequences

- Each device phase adds its kind's keys to `siq-config`, with defaults, and an example in
  `config/`.
- Errors are pleasant at start-up. Producing them costs a second parse of the file, only when
  there are problems.

## Verification

`siq-config` tests:

- `the_simulated_instrument_is_valid`, with a snapshot of the effective configuration;
- `the_hardware_example_is_valid_and_keeps_absolute_paths`;
- `load_reads_the_file_and_resolves_against_its_folder`;
- `defaults_fill_what_a_minimal_configuration_leaves_out`;
- `every_problem_is_reported_with_its_location`: syntax, typo, type, missing section, unknown kind,
  bad colour, missing key, stage, turret and cross-reference rules, pinned by snapshot;
- the JSON Schema snapshot.
