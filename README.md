# SpectrumIQ

Scientific microscope instrument software:

- a ~100 MP 14-bit CoaXPress camera on a Euresys Coaxlink grabber;
- a motorized XYZ stage and a focus drive;
- live and post-acquisition stitching to 500 MP and beyond;
- depth from focus, deconvolution, optical sectioning and light field;
- one Vue web app for the Tauri desktop app and remote browsers;
- a Rust service core built on [`teta-wot`](https://github.com/lucaswewa/teta-wot).

## Status

**In progress:** Milestone 9, multi-client use: accounts, the control token and the audit trail.
P9.5 is the latest: the end-to-end tests in Edge, two browsers handing the token over among them,
and the soak, 8 viewers and an operator for 30 minutes (`cargo xtask soak`).

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
- Milestone 4 (storage, jobs and the 2D deep-zoom viewer), complete: a 100 MP
  snap is a dataset whose first tile comes 1.5 s after the snap, a 533 MP synthetic mosaic browses
  with tiles in milliseconds, Fiji and QuPath open the OME-TIFF export, and napari the OME-Zarr:
  - P4.1, [`siq-storage`](crates/siq-storage/README.md)'s datasets, with a manifest of their
    provenance, and the recorder that writes the frames marked to save with direct I/O, straight
    from the frame pool (`--example record_sim`);
  - P4.2, OME-Zarr images in `siq-storage`, NGFF 0.5 on Zarr v3, written by chunk from many
    threads and read by region, which napari opens (`--example zarr_write_read`);
  - P4.3, [`siq-jobs`](crates/siq-jobs/README.md): heavy processing in the background, within
    the RAM and GPU budgets, at below-normal priority, with the `jobs` Thing to watch and cancel
    it (`--example jobs_demo`);
  - P4.4, [`siq-pyramid`](crates/siq-pyramid/README.md): an OME-Zarr image's levels by 2 × 2
    mean, out of core and in parallel, as a job (`--example build_pyramid`);
  - P4.5, deep-zoom tiles rendered on demand from the levels through each channel's LUT, cached,
    and served under `/api/siq/data/`, with a static DZI export (`siq-jobs --example export_dzi`);
  - P4.6, the OME-TIFF export: tiled BigTIFF with the pyramid as SubIFDs and the OME-XML, as a
    job (`siq-jobs --example export_ometiff`);
  - P4.7, the `datasets` Thing: a snap made into a dataset, its image and its pyramid, the
    exports, and the manifest and thumbnail routes (`examples/python/snap_and_fetch_tile.py`);
  - P4.8, the Datasets page and the Viewer: OpenSeadragon with a tiled image per channel, added
    in their colours, each channel's LUT, z and t, the scale bar and the probe (`/#/dev/viewer2d`);
  - P4.9, [`siq-datagen`](apps/siq-datagen/README.md): synthetic tile scans, z-stacks and bead
    stacks with their exact ground truth (`cargo run -p siq-datagen -- tilescan --smoke`).
- Milestone 5 (registration and calibration), complete: on the simulator, the calibration
  recovers the camera's mount within 0.03 % in scale and 0.009° in rotation, a tile scan's flat
  takes its seams from 10 % to under 0.1 %, and the wizard leads a new user through each
  objective's calibrations as it comes into use:
  - P5.1, [`siq-register`](crates/siq-register/README.md): sub-pixel translation between two
    images by phase correlation, with its confidence (`--example register_pair`).
  - P5.2, [`siq-calib`](crates/siq-calib/README.md) and
    [`siq-workflow`](crates/siq-workflow/README.md)'s first routine: the stage–camera calibration
    of each objective, measured, fitted, versioned in the settings folder and stamped on every
    frame, through the `calibration` Thing (`siq-server --example calibrate_sim`);
  - P5.3, the shading in [`siq-calib`](crates/siq-calib/README.md): the dark and the flat of each
    objective and channel, captured or estimated from a scan's tiles as a job, applied as frames
    are read and, on request, to the previews (`siq-jobs --example estimate_flat`);
  - P5.4, the Calibration page and its wizard, from `microscope`: each objective's and channel's
    calibrations, the first-run prompt, and the gating rules (`/#/dev/calibration`).
- Milestone 6 (workflows):
  - P6.1, [`siq-workflow`](crates/siq-workflow/README.md)'s engine: one executor for every
    acquisition plan, its dataset with the plan's provenance, progress, pause, cancel and error
    policies (`--example run_plan_sim`);
  - P6.2, tile scans: a grid along the camera's axes with the overlap asked, clipped to a
    rectangle or polygon, ordered and estimated, run by the engine with its layout
    (`siq-workflow --example tilescan_sim`);
  - P6.3, autofocus and focus maps: a coarse and a fine sweep, with their confidence, and a plane
    or thin-plate surface through the focus found at places, which a tile scan follows
    (`siq-workflow --example autofocus_sim`);
  - P6.4, sequences: z-stacks, multiple positions and time-lapses on a schedule, nesting in any
    order, every frame numbered by its place, plane and time point
    (`siq-workflow --example timelapse_sim`);
  - P6.5, the `workflow` Thing: plans run over HTTP under the global lock, followed, paused and
    cancelled, and the saved positions (`python examples/python/tile_scan.py`, `z_stack.py`,
    `timelapse.py`);
  - P6.6, the Scan page: a region drawn on the stage map, its focusing and z-stacks, its tiles and
    estimate from the server, and Start (`/#/scan`, `/#/dev/scan`);
  - P6.7, the run panel and sequences: a run followed wherever it started, with thumbnails, paused,
    cancelled once confirmed, a failed step retried or skipped when its policy asks, a toast as it
    ends; and time-lapses of the saved positions set up on the Scan page (`/#/dev/run`).
- Milestone 7 (live stitching), complete: on the simulator, stepping over a 6 × 6-field area paints
  a coherent mosaic, placed within a canvas pixel of the specimen, with no frame lost and its pages
  within the VRAM budget:
  - P7.1, [`siq-gpu`](crates/siq-gpu/README.md): wgpu on DX12 and Vulkan, the adapter's own
    limits, one thread for the device, u16 uploads and readback, and the display LUT against its
    CPU reference (`siq-gpu --example gpu_info`);
  - P7.2, [`siq-stitch-live`](crates/siq-stitch-live/README.md)'s canvas: each frame painted on
    sparse GPU pages through its affine, feathered so the newest frame wins, within a VRAM budget
    (`siq-stitch-live --example paint_sim`);
  - P7.3, the [`mosaic_live`](crates/siq-server/README.md) Thing: the frames painted as the stage
    moves, binned and flat-corrected on the frame path, and the client's viewport rendered on the
    GPU and streamed as JPEGs; saved as an OME-Zarr dataset (`siq-server --example
    live_mosaic_sim`);
  - P7.4, the Mosaic page: the live mosaic panned and zoomed, the camera's field and a minimap
    over it, a double-click moving the stage there, Clear and Save; and the Scan page's stage map
    over the mosaic (`/#/mosaic`, `/#/dev/mosaic`).
- Milestone 8 (post-acquisition stitching), complete: on generated ground truth, scans of 3 × 3 to
  10 × 10 tiles are placed within 0.02 pixels RMS of the truth, and a 500 MP mosaic is stitched in
  under 8 s within 1.3 GB:
  - P8.1, [`siq-stitch`](crates/siq-stitch/README.md)'s pairs: a scan's tiles placed by their
    poses, the sides of the grid registered by phase correlation within the stage's reach, each
    offset with its confidence (`siq-stitch --example pairs_on_dataset`);
  - P8.2, the global solve: the tiles placed by every trusted edge and, weakly, the stage, with
    outliers rejected; 30 tiles within 0.06 pixels RMS of the truth (`siq-stitch --example
    solve_on_dataset`);
  - P8.3, the compositing: the mosaic written into OME-Zarr a block at a time, each tile
    resampled at its solved corner, flat-corrected and feathered, within a few blocks' memory, and
    its pyramid built, as a job (`siq-stitch --example composite_dataset`);
  - P8.4, the stitch job: a tile scan stitched by itself when it finishes, or on request, its QA
    report and measures kept with the dataset; the Jobs page, Stitch… on the Datasets page, and the
    viewer's tile and residual overlays (`examples/python/stitch_dataset.py`);
  - P8.5, validation and benchmarks: an accuracy suite over generated scans of different sizes,
    overlaps, stage errors, light and specimens, every one within 0.3 pixels RMS of the truth; the
    defaults tuned; and 535 MP stitched in 7.8 s from its flat to its pyramid, within 1.3 GB
    (`siq-jobs --example bench_stitch_500mp`).
- Milestone 9 (multi-client, accounts, control token and audit), under way:
  - P9.1, [`siq-auth`](crates/siq-auth/README.md): local accounts with argon2id hashes and roles,
    managed by `spectrumiq-server user`, browser sessions in `HttpOnly`, `SameSite=Strict` cookies,
    failed logins limited, and changes from other sites' pages refused (`siq-auth --example
    user_cli`);
  - P9.2, who may do what: every route classified, viewers reading, operators processing and
    stopping, the instrument changed only by the holder of the Hardware Control Token, which an
    admin or the local console takes over and a silent holder loses; the local console's
    one-time secret (`python examples/python/two_clients.py`);
  - P9.3, the audit trail: logins, the token's changes, every change and refusal with its caller,
    and the jobs' and runs' lifecycles, hash-chained in a file a month, checked by
    `spectrumiq-server audit verify` and read by admins (`siq-auth --example audit_verify`);
  - P9.4, the web app's side: a login where the service asks for one, the desktop logged in by
    itself, every control disabled with the reason where the role or the token is missing, the
    control token's bar above every page, and the Admin page for the users and the audit trail;
    the examples log in with `SIQ_USER` (`/#/dev/admin`);
  - P9.5, the end-to-end tests (`cargo xtask ci --e2e`), in Edge against a fresh simulated
    instrument: the login, live view, two browsers handing the token over, a silent holder losing
    it, and the connection budget's 3 streams, sockets and tiles at once; and the soak, viewers and
    an operator against a service of its own, reported against ADR-0069's targets
    (`cargo xtask soak --smoke`).

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
