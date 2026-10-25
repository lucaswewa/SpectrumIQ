# SpectrumIQ

Scientific microscope instrument software:

- a ~100 MP 14-bit CoaXPress camera on a Euresys Coaxlink grabber;
- a motorized XYZ stage and a focus drive;
- live and post-acquisition stitching to 500 MP and beyond;
- depth from focus, deconvolution, optical sectioning and light field;
- one Vue web app for the Tauri desktop app and remote browsers;
- a Rust service core built on [`teta-wot`](https://github.com/lucaswewa/teta-wot).

## Status

**In progress:** Milestone 4, storage, jobs and the 2D deep-zoom viewer. P4.3 is the latest:
[`siq-jobs`](crates/siq-jobs/README.md), which runs heavy processing in the background within
the RAM and GPU budgets, by priority and at below-normal priority, with the `jobs` Thing to watch
and cancel them (`cargo run -p siq-jobs --example jobs_demo`).

Done so far:

- Milestone 0 (foundations):
  - P0.1, the workspace and the `cargo xtask` quality gate;
  - P0.2, [`siq-types`](crates/siq-types/README.md), the shared vocabulary;
  - P0.3a, [`siq-telemetry`](crates/siq-telemetry/README.md), logging and metrics;
  - P0.3b, [`siq-config`](crates/siq-config/README.md), the instrument configuration, with
    [`config/sim.toml`](config/sim.toml) for the simulated instrument.
- Milestone 1 (frame memory, devices and simulation), complete: the simulated rig of
  `config/sim.toml` runs with the real drivers against their emulators
  (`cargo run --release -p siq-sim --example sim_rig`):
  - P1.1, [`siq-buffer`](crates/siq-buffer/README.md), the frame pool, with slots locked in RAM and
    frames shared without allocation or copy;
  - P1.2, the frame fan-out in `siq-buffer`: every frame to many consumers, each with its own
    policy, and the producer never waits;
  - P1.3, [`siq-devices`](crates/siq-devices/README.md), the device traits that the engines use,
    with capability descriptors, status watches, and mocks for engine tests;
  - P1.4, [`siq-synth`](crates/siq-synth/README.md), deterministic synthetic specimens with exact
    ground truth, and real images as specimens;
  - P1.5, [`siq-imgproc`](crates/siq-imgproc/README.md), allocation-free CPU kernels for
    correction, resampling, histograms, display mapping and focus, within the 100 MP frame
    budget;
  - P1.6, [`siq-sim`](crates/siq-sim/README.md)'s image formation: realistic 14-bit frames of a
    synthetic specimen for the stage pose and focus, with optics, sensor noise and a noiseless
    cache;
  - P1.7, the simulated camera in `siq-sim`: a `Camera` that renders the true state into pool slots
    at the model's frame rates, free-running or triggered;
  - P1.8, [`siq-serial`](crates/siq-serial/README.md)'s line transports and the GRBL 1.1 emulator
    in `siq-sim`: a stage controller to drive, whose true position carries realistic errors;
  - P1.9, [`siq-grbl`](crates/siq-grbl/README.md), the GRBL and grblHAL stage driver: moves that
    resolve once the stage is still, a backlash-consistent approach, jogs, homing, and a stop that
    keeps the position;
  - P1.10a, [`siq-ascii`](crates/siq-ascii/README.md)'s data-driven dialects: a device's ASCII
    protocol described by a TOML file, and `siq-sim`'s emulator of any dialect;
  - P1.10b, the ASCII focus driver in `siq-ascii`, and the peripheral simulators in `siq-sim`.
- Milestone 2 (acquisition, live preview and the first server), complete: 10 minutes of simulated
  live view lose no frames with both previews streaming, a script sees the stage move, and the
  Euresys shim builds against the SDK:
  - P2.1, [`siq-acquire`](crates/siq-acquire/README.md), the acquisition engine: live view,
    settle-aware snaps and sequences, with every frame numbered and stamped with its pose and
    context;
  - P2.2, [`siq-preview`](crates/siq-preview/README.md)'s live previews: overview and ROI JPEG
    streams and statistics, with the display mapping on the server;
  - P2.3, [`siq-server`](crates/siq-server/README.md) and
    [`spectrumiq-server`](apps/spectrumiq-server/README.md): the simulated instrument served over
    HTTP, with its camera, display, stage and focus Things;
  - P2.4, the peripherals, the channels, and the metrics and memory routes in `siq-server`
    (`python examples/python/channels.py`);
  - P2.5, [`siq-euresys-sys`](crates/siq-euresys-sys/README.md): Euresys eGrabber behind a C ABI
    of our own, which finds this PC's Coaxlink and camera, and measures its frame rate
    (`eg_discover`, `eg_fps`).
- Milestone 3 (frontend foundations), complete: the web app runs from the simulated instrument in
  Edge and in the desktop shell:
  - P3.1, the [web app](web/README.md), started from `microscope`'s, with its Reka UI components,
    and served by the server at `/`;
  - P3.2, its client: one WebSocket per Thing, a budget of three MJPEG streams, and Pinia stores
    mirroring the instrument;
  - P3.3, the Live page: the overview and the ROI streams, the ROI chosen on the overview, and the
    display and camera controls;
  - P3.4, the stage, the focus and the peripherals on the Live page, with the keys, the wheel,
    click-to-move and STOP (`/#/dev/control` without a server);
  - P3.5, the [desktop shell](apps/spectrumiq-desktop/README.md): a Tauri window that finds the
    local service and loads its web app (`npm run tauri dev` in `apps/spectrumiq-desktop`).
- Milestone 4 (storage, jobs and the 2D deep-zoom viewer), under way:
  - P4.1, [`siq-storage`](crates/siq-storage/README.md)'s datasets, with a manifest of their
    provenance, and the recorder that writes the frames marked to save with direct I/O, straight
    from the frame pool (`--example record_sim`);
  - P4.2, OME-Zarr images in `siq-storage`, NGFF 0.5 on Zarr v3, written by chunk from many
    threads and read by region, which napari opens (`--example zarr_write_read`).

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

Third-party components and their licences are in [docs/soup.md](docs/soup.md). The live previews'
JPEG encoder, `jpeg-encoder`, derives from the Independent JPEG Group's code: this software is based
in part on the work of the Independent JPEG Group.
