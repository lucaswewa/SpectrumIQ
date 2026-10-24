# SpectrumIQ

Scientific microscope instrument software:

- a ~100 MP 14-bit CoaXPress camera on a Euresys Coaxlink grabber;
- a motorized XYZ stage and a focus drive;
- live and post-acquisition stitching to 500 MP and beyond;
- depth from focus, deconvolution, optical sectioning and light field;
- one Vue web app for the Tauri desktop app and remote browsers;
- a Rust service core built on [`teta-wot`](https://github.com/lucaswewa/teta-wot).

## Status

**In review:** P1.2, the frame fan-out in [`siq-buffer`](crates/siq-buffer/README.md): every frame
to many consumers, each with its own policy, and the producer never waits.

Done so far:

- Milestone 0 (foundations):
  - P0.1, the workspace and the `cargo xtask` quality gate;
  - P0.2, [`siq-types`](crates/siq-types/README.md), the shared vocabulary;
  - P0.3a, [`siq-telemetry`](crates/siq-telemetry/README.md), logging and metrics;
  - P0.3b, [`siq-config`](crates/siq-config/README.md), the instrument configuration, with
    [`config/sim.toml`](config/sim.toml) for the simulated instrument.
- Milestone 1 (frame memory, devices and simulation):
  - P1.1, [`siq-buffer`](crates/siq-buffer/README.md), the frame pool, with slots locked in RAM and
    frames shared without allocation or copy.

The [roadmap](docs/plan/README.md#3-roadmap) tracks every phase.

## Documentation

| | |
|---|---|
| [docs/goals.md](docs/goals.md) | The goals |
| [docs/plan/](docs/plan/README.md) | The implementation plan: decisions, roadmap and phase briefs |
| [docs/plan/architecture.md](docs/plan/architecture.md) | The target architecture |
| [docs/plan/conventions.md](docs/plan/conventions.md) | How each phase is built, documented and checked |
| [docs/adr/](docs/adr/README.md) | Architecture decision records |
| [docs/requirements.md](docs/requirements.md) | Requirements, with traceability |
| [docs/impl-notes/](docs/impl-notes/) | What each phase did |

## Prerequisites

- **Windows 11 x64**, the only supported platform.
- **Rust through [rustup](https://rustup.rs).** `rust-toolchain.toml` pins 1.99.1, which rustup
  installs on first use.
- **MSVC:** Visual Studio 2022, or its Build Tools, with the C++ workload.
- **Node.js 24 and npm**, for the web app (from P3.1).
- **Optional:**
  - the Euresys eGrabber SDK, for the real camera (`sdk` feature, from P2.5);
  - LLVM, only to regenerate the Euresys bindings;
  - [cargo-deny](https://github.com/EmbarkStudios/cargo-deny), for the licence check
    (`cargo install --locked cargo-deny`).

## Checks

```powershell
cargo xtask ci                 # everything a phase must pass
cargo xtask loc --base main    # the phase's code budget: under 1000 counted lines
cargo xtask layers             # the crate layering
```

## Layout

| Folder | What |
|---|---|
| [`crates/`](crates/README.md) | Libraries, `siq-*`, in the layers of architecture §3 |
| [`apps/`](apps/README.md) | Binaries: the service, the desktop app, tools |
| [`xtask/`](xtask/README.md) | `cargo xtask`: development tasks |
| [`docs/`](docs/plan/README.md) | Plan, ADRs, requirements, implementation notes |

## Licence

MIT. See [LICENSE](LICENSE).
