# Milestones

One file per milestone, holding its phase briefs: [M0](M00-foundations.md),
[M1](M01-memory-devices-simulation.md), [M2](M02-acquisition-preview-server.md),
[M3](M03-frontend-foundations.md), [M4](M04-storage-jobs-viewer.md),
[M5](M05-registration-calibration.md), [M6](M06-workflows.md), [M7](M07-live-stitching.md),
[M8](M08-post-stitching.md), [M9](M09-multi-client-security.md),
[M10](M10-hardware-bringup.md), [M11](M11-depth-from-focus.md),
[M12](M12-deconvolution-sectioning.md), [M13](M13-light-field.md),
[M14](M14-performance-advanced-viewers.md), [M15](M15-production-readiness.md). The
[roadmap](../README.md#3-roadmap) tracks every phase.

## Testing a milestone by hand

Each finished milestone has a section below: what to run, and what you should see, against its
exit criteria. The automated checks run with `cargo xtask ci`; these are the ones a person makes.
A later milestone adds its section when its last phase is merged.

### Before you start

Everything runs on the simulator, from the repository, in a terminal at its root (`cmd` syntax).

1. **Build the web app and the service:**

   ```text
   cd web && npm ci && npm run build && cd ..
   cargo build --release -p spectrumiq-server
   ```

2. **Add users** (since M9, the service asks for a login). The first user is an admin while there
   is none; each password is asked for twice:

   ```text
   target\release\spectrumiq-server user add ada -c config/sim.toml
   target\release\spectrumiq-server user add olga --role operator -c config/sim.toml
   target\release\spectrumiq-server user add vic --role viewer -c config/sim.toml
   ```

3. **Start the service:** `target\release\spectrumiq-server -c config/sim.toml`, and open
   <http://127.0.0.1:5000/>. Log in, and **Take control** before changing the instrument.
4. **Scripts** in `examples/python/` log in as `SIQ_USER`, and take control if they change the
   instrument: `set SIQ_USER=olga& set SIQ_PASSWORD=<her password>`.
5. **The development pages** (`/#/dev/…`) are in development builds only. Run Vite:
   `cd web && npm run dev`, and open <http://localhost:5173/#/dev/components>. They need no login,
   and most need no server.

**Where things go:** `config/sim.toml` keeps its data, settings (the users and calibrations),
audit trail and logs in the repository's `data/`, `settings/`, `audit/` and `logs/`, which git
ignores. To start from a fresh instrument, copy `config/sim.toml` into a folder of its own, such as
`C:\siq-test\config\sim.toml`, and pass that path to `-c`: everything then goes beside it. Add
the users again for it.

**Two users at once:** two tabs of one browser share a session. Use another browser, or an
InPrivate window, for each user.

### M0 — Foundations

- `cargo xtask ci` passes on a clean checkout: formatting, clippy, the tests, the docs, the
  layering, licences, and the web checks.
- `cargo xtask loc --base main` prints the lines by class and the budget, and `cargo xtask layers`
  prints `layers: OK`.
- `cargo run -p siq-config --example validate_config -- config/sim.toml` prints the configuration
  with its defaults filled in, and no error.
- New ADRs, implementation notes and crate READMEs follow `docs/adr/0000-template.md`,
  `docs/impl-notes/TEMPLATE.md` and `docs/templates/crate-README.md`.

### M1 — Frame memory, devices and simulation

- `cargo run --release -p siq-sim --example sim_rig`: the simulated camera, the GRBL driver and the
  ASCII focus driver against their emulators. It homes, moves the stage and focus, and prints each
  frame's mean and sharpness with the reported and true poses. The frames follow the pose, and are
  sharpest at the focus.
- `cargo run --release -p siq-buffer --example fanout_soak`: 60 s at 9 frames/s with 194 MB slots.
  It prints what each consumer received and dropped, and checks the counts and the pool.
- `cargo run --release -p siq-imgproc --example kernels_bench`: every kernel on a 102 MP frame,
  timed against the per-frame budget.
- `cargo run --release -p siq-synth --example render_world -- <folder>`: each synthetic world as a
  16-bit PNG, with its ground truth beside it. Open the PNGs.
- `cargo run -p siq-sim --example grbl_emulator_repl`: a scripted session with the GRBL emulator:
  unlock, a move watched by its status reports, a jog cancelled, a feed hold, a reset in motion,
  and the true stage's errors.

### M2 — Acquisition, live preview and the first server

With the service running:

- `python examples/python/live_and_move.py`: it moves the stage, and saves an overview frame
  before and after, as `before.jpg` and `after.jpg`. They differ by the move.
- `python examples/python/live_soak.py http://127.0.0.1:5000 600`: 10 minutes of live view with
  both previews. It fails if the camera loses a frame.
- `python examples/python/channels.py`: each channel in turn, with a snap of each.
- Open <http://127.0.0.1:5000/api/v1/camera/overview> in a browser while live: the MJPEG stream.
  `/api/siq/system/metrics` and `/api/siq/system/memory` show the pool, the acquisition and the
  previews' latency. `test.http` holds every route, for VS Code's REST Client.
- **With the Euresys SDK installed:**
  `cargo run -p siq-euresys-sys --features sdk --example eg_discover` lists the interfaces,
  grabbers and cameras; with a Coaxlink and the camera,
  `cargo run --release -p siq-euresys-sys --features sdk --example eg_fps` measures the frame rate.

### M3 — Frontend foundations

- **In Edge,** at <http://127.0.0.1:5000/>, on the Live page:
  - Start live: the overview and the ROI stream.
  - Drag on the overview to choose the ROI, and move the histogram's markers.
  - Tap and hold the arrow keys, PgUp and PgDn; turn the wheel on the image; double-click the
    overview; press Esc or STOP.
  - The page stays responsive with both streams open.
- **The themes:** switch the system's light or dark setting. On Vite, `/#/dev/components` and
  `/#/dev/tokens` show every component and token in both themes.
- **Without a server,** on Vite: `/#/dev/live` and `/#/dev/control`, against a fake camera and
  stage, with the commands sent listed beside them; `/#/dev/observe`, every store's values live.
- **The desktop shell:** `cd apps/spectrumiq-desktop`, `npm ci`, `npm run tauri dev`.
  - With the service stopped, it says so, and offers to start the simulated instrument.
  - About says "Running in: The desktop app".
  - A second copy brings the first window forward.

### M4 — Storage, jobs and the 2D deep-zoom viewer

- `python examples/python/snap_and_fetch_tile.py`: a 100 MP snap made into a dataset, its pyramid,
  and a tile and the thumbnail saved, with the time from the snap to the first tile: well under
  10 s.
- **The Datasets page:** the snap's card and its details: its manifest, history and exports. View
  opens the Viewer: zoom and pan, change each channel's LUT, use the probe and the scale bar.
- **A large mosaic:** stop the service, run `cargo run --release -p siq-datagen -- tilescan`,
  start the service again, and View the tile scan's mosaic on the Datasets page. It browses at
  interactive speed.
- **The exports:**
  - `cargo run --release -p siq-jobs --example export_ometiff -- --out <folder>`: open the
    OME-TIFF in Fiji or QuPath, with its pyramid and channels;
  - `cargo run --release -p siq-storage --example zarr_write_read -- --out <folder>`: open the
    OME-Zarr in napari.
- `cargo run -p siq-jobs --example jobs_demo`: the scheduler's priorities, the GPU rule and the
  budget.

### M5 — Registration and calibration

- `cargo run --release -p siq-register --example register_pair`: the shifts found against those
  moved, with their error and confidence.
- `cargo run --release -p siq-server --example calibrate_sim`: the stage–camera calibration's
  scale, rotation and shear against those the simulator injected: within 0.1 % and 0.05°.
- `cargo run --release -p siq-jobs --example estimate_flat`: a tile scan's seams before and after
  its flat, at least 5× lower.
- **The wizard,** on a fresh instrument (see "Where things go"): as an operator, Take control. The
  wizard welcomes you and leads through the stage–camera calibration and each channel's dark and
  flat. Skip, Back and closing early, which asks first, all work. The Calibration page then shows
  each objective's calibrations and whether scans may run.
- **Without a server,** on Vite: `/#/dev/calibration`.

### M6 — Workflows

- **The examples:** `siq-workflow`'s `tilescan_sim` (a 5 × 5 scan), `autofocus_sim` (the focus
  found against the truth), `timelapse_sim` and `run_plan_sim`, each with
  `cargo run --release -p siq-workflow --example <name>`.
- **The scripts,** with the service running and calibrated: `python examples/python/tile_scan.py`,
  `z_stack.py` and `timelapse.py`. Each prints its progress, and makes a dataset.
- **The Scan page,** holding control:
  - draw a region with Rectangle or Polygon; pick a focus map's places;
  - watch the server's estimate follow;
  - Start, then pause, resume and cancel the run, which asks first;
  - the run panel follows it, with thumbnails, and the dataset appears on the Datasets page.
- **Without a server,** on Vite: `/#/dev/scan`, and `/#/dev/run` for pausing, cancelling, and a
  failed step's Retry or Skip.

### M7 — Real-time live stitching

- `cargo run --release -p siq-gpu --example gpu_info`: the adapters, and the LUT on the GPU
  against the CPU.
- `cargo run --release -p siq-stitch-live --example paint_sim -- <out.png>`: a 6 × 6 grid painted
  on the canvas. Open the PNG: a coherent mosaic.
- `cargo run --release -p siq-server --example live_mosaic_sim`: the live mosaic through the
  `mosaic_live` Thing, its views written as JPEGs.
- **The Mosaic page,** calibrated and holding control:
  - start live view, turn on Paint, and step or jog the stage over an area: the fields join into
    one mosaic, with no seams out of place;
  - pan and zoom; double-click to move the stage there;
  - Clear, and Save, which makes a dataset;
  - `/api/v1/mosaic_live/stats` shows the pages' GPU memory within the budget, and
    `/api/siq/system/metrics` no frame lost (`acquisition.lost_device` and `lost_no_slot`).
- **The Scan page's stage map** shows the painted mosaic. Without a server, on Vite:
  `/#/dev/mosaic`.

### M8 — Post-acquisition stitching

- **Accuracy:** `cargo test --release -p siq-stitch --test accuracy -- --ignored`: 18 generated
  scans, each within 0.3 pixels RMS and 1 pixel at worst of the truth. About a minute.
- **The 500 MP benchmark:**
  `cargo run --release -p siq-jobs --example bench_stitch_500mp`: each stage's time, the peak
  memory within 14 GB, and the tiles against the truth.
- **The steps on a dataset:** `siq-stitch`'s `pairs_on_dataset`, `solve_on_dataset` and
  `composite_dataset`, each with `cargo run --release -p siq-stitch --example <name>`.
- **In the app,** holding control:
  - run a tile scan on the Scan page: it stitches by itself when it finishes, and the Jobs page
    shows the job's progress;
  - View the stitched image: no visible seams. Under Layers, the tiles' outlines and residuals
    show the QA;
  - on the Datasets page, Stitch… stitches it again with other options;
  - `python examples/python/stitch_dataset.py` does the same through the API, and prints the
    measures and the worst pairs.

### M9 — Multi-client, accounts, control token and audit

- **The login:** a wrong password is refused. As `vic`, the bar says "View only.", and the
  controls and STOP are disabled, each with its reason. Log out: the login page comes back.
- **Taking control:** as `olga`, Take control. On a fresh instrument the calibration wizard offers
  itself (`vic` never sees it), and the Live page's controls come on. `vic` sees "olga has control
  since …".
- **Taking over:** as `ada`, Take over. `olga`'s controls go back to disabled, and she gets no Take
  over herself. Then Release.
- **The Admin page,** as `ada`:
  - add a user; change `vic`'s role to operator, which applies at once;
  - give a user a new password, which logs them out;
  - remove a user, which asks first;
  - in the audit trail, filter by Who, at Enter, and by What.
- **The token's timeout:** close the holder's browser. About 30 s later, the others see that no one
  has control.
- **The desktop:** with the service running, `npm run tauri dev` in `apps/spectrumiq-desktop`. It
  logs in by itself as `console, operator`, with no password, and can Take over from `olga`.
- **The scripts:** without `SIQ_USER`, `python examples/python/live_and_move.py` is refused with
  401. With it, it takes control, or says who has it.
- **The audit trail's chain:** `target\release\spectrumiq-server audit verify -c config/sim.toml`
  checks every file, and prints the last hash.
- **The automated runs,** with the service on port 5000 stopped, so that they don't share its CPU:
  - `cargo xtask ci --e2e`: the gate, then the end-to-end tests in Edge;
  - `cargo xtask soak --smoke`, 20 s, or `cargo xtask soak --viewers 8 --minutes 30`, the exit
    criterion's soak. It prints its report against ADR-0069's targets, and keeps it in
    `target/soak/report.json`.

### M10 — Hardware bring-up: the camera, colour, the stage's bring-up, trigger and strobe

On the instrument's PC, with the eGrabber SDK installed: the camera's commands build with
`--features sdk`, and the service with `--features euresys`. The stage and the focus drive are
still simulated: what M10 says of them runs on the emulators. Each camera command puts back the
camera's settings as it found them. To keep them yourself, run `eg_features --save before.txt`
first and `eg_features --restore before.txt` after: the
[camera runbook](../../hardware/camera-bringup.md), step 2.

- **The camera** ([runbook](../../hardware/camera-bringup.md)):
  - `cargo run --release -p siq-euresys --features sdk --example eg_live` gives about 12.3 frames
    a second at the full frame, none lost;
  - `--example hw_acquire_bench` gives each mode within a few percent of its model, none lost;
  - the hardware tests, in `siq-euresys`'s README: six pass;
  - `--example strobe_timing -- --triggers 40`: each exposure starts about 0.6 ms after its
    trigger, and as many strobes as triggers are counted on `IOUT11`.
- **The service on the camera:** copy `config/sim.toml` into a folder of its own, set its
  `[camera]` as the runbook's step 8 says, and add a user. Run
  `cargo run --release -p spectrumiq-server --features euresys -- -c <that file>`, and start live
  view:
  - both previews stream in colour, the white balanced;
  - turn off **Automatic white balance**, or press **White from ROI** on a blank of the slide;
  - snap into a dataset: the viewer shows it in colour at every zoom.

  Under load: `cargo xtask soak --euresys --viewers 8 --minutes 10`. No frame is lost.
- **Colour on the simulator:** in a copy of `config/sim.toml`, uncomment `cfa = "gbrg"`:
  - the histogram shows red, green and blue;
  - calibrate, take a dark and a flat, and autofocus, as in M5 and M6: each works on the frames'
    luminance;
  - the live mosaic paints in grey, the luminance;
  - make a colour scan with `cargo run --release -p siq-datagen -- tilescan --colour --columns 3
    --rows 3`, and stitch it from the Datasets page: the mosaic has three channels, red, green and
    blue, and shows in colour.
- **The stage, on the emulator** ([runbook](../../hardware/motion-bringup.md)):
  - `target\release\spectrumiq-server --list-ports` lists this PC's ports, as does the Settings
    page;
  - `cargo run --release -p siq-grbl --example grbl_drive` prints the status's round trip, about
    10 ms;
  - set `"$100" = 800.0` under `[stage.expect]` in a copy of `config/sim.toml`: the service
    refuses to start, naming `$100`;
  - `cargo run --release -p siq-server --example motion_repeatability -- --repeats 20` gives a
    repeatability of about 0.6 µm against the emulator's 0.5, and a backlash of about 2 µm;
  - a calibration's record in `status` has its `repeatability_um`.
- **Not yet, until the real stage and focus drive are in:**
  - the full frame live for 10 minutes with the live mosaic and the recorder;
  - a real 3 × 3 tile scan stitched;
  - the focus drive's dialect, which needs its protocol document.

  On the camera, the stage can't be calibrated against a simulated stage.
