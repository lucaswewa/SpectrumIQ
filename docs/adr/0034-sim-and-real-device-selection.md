# ADR-0034: Simulated or real devices, chosen per device by the configuration

- **Status:** Accepted
- **Date:** 2026-10-24
- **Phase:** P2.3
- **Related:** ADR-0012, ADR-0005, ADR-0016, ADR-0020, ADR-0027, ADR-0030, REQ-SIM-006

## Context

- **The whole application must run without hardware** (REQ-SIM-006), and each device's real
  driver arrives in its own phase: the camera in P10.1, the stage and focus drive in P10.3, and
  the peripherals in P10.5.
- **The configuration already names each device's kind** (ADR-0020): `[camera] kind = "sim"` or
  `"euresys"`, `[stage] kind = "sim"` or `"grbl"`, `[focus] kind = "sim"` or `"ascii"`.
- **The simulators must exercise the real drivers,** not stand in for them (ADR-0012): the stage
  and focus are simulated at the protocol level (ADR-0027, ADR-0030).

## Decision

`siq_server::devices::open(config, clock)` makes each device from its own section, independently:

| Device | `kind = "sim"` | Real |
|---|---|---|
| Camera | `siq-sim`'s camera, on the configured world, sensor and `fast` profile | `"euresys"`: refused until P10.1, with a message saying so |
| Stage | The GRBL driver over an in-memory link to the GRBL emulator, with `[sim.errors]` | `"grbl"`: the GRBL driver on the serial `port` at `baud` |
| Focus drive | The ASCII driver over an in-memory link to the dialect emulator, in `dialect` or the example dialect | `"ascii"`: the ASCII driver on the serial `port`, in `dialect` |

- **The simulators share one `TrueState`.** The emulators move it, and the camera sees it.
- **The drivers are the same either way:** only the transport changes, a duplex or a port.
- **The frame pool's memory:**
  - for a real camera, slots locked in RAM (ADR-0005);
  - for the simulator, the heap, so a development PC needn't lock gigabytes.

  Either way, each slot holds a full frame at 16 bits, and there are `[memory] pool_slots` of
  them.
- **A device that can't be opened stops the service from starting,** with the device and port
  named.
- **Mixing is allowed:** for example, the real stage with the simulated camera, for bring-up.

## Alternatives considered

- **One switch for the whole instrument:** bring-up needs real devices one at a time.
- **Mocks for the simulated kinds:** they don't run the drivers' protocol code (ADR-0012).
- **Choosing at compile time, with features:** one binary serves both, and the configuration
  decides.

## Consequences

- **`config/sim.toml` runs the whole service** with `cargo run -p spectrumiq-server -- -c
  config/sim.toml`.
- **A simulated stage with a real camera** sees nothing move: the true state is only the
  simulators'.
- **Each real driver's phase** replaces its `bail!` or adds its kind here.

## Verification

- **`siq-server` `server` tests** build the service from `config/sim.toml` on a small sensor, and
  drive it.
- **By hand:** the `examples/python/live_and_move.py` script against the running service.
