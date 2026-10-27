# ADR-0064: Stitch QA, re-processing, and provenance

- **Status:** Accepted
- **Date:** 2026-10-27
- **Phase:** P8.4
- **Related:** ADR-0009, ADR-0041, ADR-0043, ADR-0056, ADR-0061, ADR-0062, ADR-0063,
  REQ-STI-002, REQ-WF-001, REQ-VIS-001

## Context

- **Stitching is a pipeline** (ADR-0061 to ADR-0063): pairs, solve, composite, pyramid. A scan's
  user wants its mosaic without asking, and to see how good it is: which pairs were trusted,
  which rejected, how far each tile moved from where the stage said.
- **A dataset records its processing** (ADR-0041): each job's kind, version, parameters, outputs and
  state in the manifest's history. A stitch's quality is known only when it ends.
- **The viewer has overlay layers** (ADR-0009, amended), drawn over an image at any zoom; they had
  no colour.
- **A scan's dataset keeps** its calibration's snapshot and its frames' poses; the flat in force is
  the shading registry's, by objective and channel (ADR-0050).

## Decision

- **One job, `stitch`** (`siq-jobs::kinds::StitchJob`): the reference plane (the first frame's
  `(t, c, z)`) registered and solved, then every frame composited at its tile's corner by its
  position `p`, a channel at a time through its own flat, then the pyramid. Its progress is the
  pairs to 25 %, the solve to 30 %, the composite to 86 %, the pyramid to the end.
- **The service makes the job** from what it knows: the calibration the dataset kept (refused
  without one), the channels' names and colours from the configuration, each channel's window from
  its first frame's 0.1 and 99.9 percentiles, and the dark and flat in force for the frames'
  objective and each channel.
- **The QA report,** `qa/<image>.json` (`siq-stitch::Report`): each tile's corner in the image, its
  size, how far it moved from the stage's place, its worst residual and component; each pair's
  residual, energy, correlation, trust and use; and a summary. The summary goes into the
  manifest's history as the record's new `results`, which a job's output now carries; the
  parameters and the version are the record's as before.
- **Re-processing** is a stitch into another image: `jobs.stitch(dataset, image, options)`, the
  options the pairs', the solve's and the composite's. Each run is a record of its own; images and
  reports of other names stay.
- **Auto-stitch:** a hook told of each finished run (ADR-0056) submits a `stitched` stitch of a
  finished tile scan, at background priority, while `jobs.auto_stitch` is on (on at start).
- **Overlays from the report:** `GET /api/siq/data/{dataset}/{image}/overlays` gives two layers:
  `stitch-tiles`, each tile's dashed outline labelled with its position, and `stitch-residuals`, a
  line between each pair's tiles' centres, its tone its residual's: `success` to half a pixel,
  `warning` above, `danger` rejected, `muted` not trusted; dashed when unused. A shape's `tone` is
  a meaning the theme colours, not a colour.
- **The UI:** the Jobs page lists the jobs running, with their progress and message, and those
  waiting, each with Cancel, and turns auto-stitch on and off; the Datasets page's details offer
  Stitch… for a dataset of more than one frame, with the stage's uncertainty, the feather and
  whole pixels; the Viewer shows a stitched image's two layers in its Layers menu.

## Alternatives considered

- **Stitch QA as a Thing's property:** the report belongs with the image it describes, and a
  dataset moved elsewhere keeps it.
- **The overlays built in the browser** from the report: the same work in every client, and the
  report's format tied to the web app's; the server's layers are what the viewer draws.
- **Colours in the overlays:** the theme couldn't adapt them; a tone is what the server knows.
- **Auto-stitch as a configuration key:** a restart to change it. A property is changed where the
  jobs are watched, and starts on.
- **Overwriting `stitched` on re-processing:** the earlier result, and its report, would be lost
  while the new one runs.

## Consequences

- **A scan becomes a mosaic** with no one asking, a minute or so after it finishes for scans of a
  few dozen tiles.
- **Every stitch is accountable:** its options, version and measures in the history, its report
  beside its image, its residuals drawn over it.
- **Two stitches of a dataset** take twice the disk: the user deletes what they don't keep.
- **Only the reference plane is registered:** a channel whose content differs only shares its
  corners.

## Verification

- `crates/siq-server/tests/stitch.rs`: a 2 × 2 scan stitched by itself, its record's results, its
  mosaic's tiles, its overlays; stitched again into another image; a dataset without a
  calibration refused; auto-stitch turned off.
- `crates/siq-jobs/tests/kinds.rs`: the composite job, with its pyramid.
- `web/tests/unit/jobs/JobsPage.spec.ts`: the jobs running and waiting, Cancel, auto-stitch, and
  the stitch options dialog; `web/tests/unit/overlay/overlay.spec.ts`: a shape's tone.
- `examples/python/stitch_dataset.py`.

## Amendment (P8.5, 2026-10-27): the 500 MP benchmark, and its targets

- **The benchmark** (`bench_stitch_500mp`): `siq-datagen`'s 8 × 7 tiles of 4096 × 3072 over cells,
  overlapping by 15 %, the stage off by 5 µm. Their flat is estimated from the frames, then the
  stitch job runs within the service's 14 GB budget. On the 16-thread workstation, on the jobs'
  8 threads, with the frames in the system's cache:

  | Stage | Wall | Processors' time | Peak working set, private | Written |
  |---|---|---|---|---|
  | The flat | 0.86 s | 0.9 s | | |
  | The pairs, 97 | 1.11 s | 8.0 s, 91 % | 0.95 GB, 0.40 GB | |
  | The solve | < 0.01 s | | | |
  | The composite, 535 MP | 3.98 s | 27.4 s, 86 % | 1.29 GB, 0.10 GB | 0.74 GB |
  | The pyramid | 1.87 s | 9.3 s, 62 % | 1.26 GB, 0.06 GB | 0.25 GB |
  | End to end | 7.82 s | 45.5 s, 73 % | 1.29 GB, 0.40 GB | 0.99 GB |

  The tiles were placed within 0.016 px RMS of the truth (0.031 at worst). Over grains, the same
  took 7.62 s, and placed them within 0.011.
- **The targets** come from the first measurement, 7.6 s and 1.24 GB with the 512-pixel search,
  with room for a slower disk or slower processors:
  - the flat and the stitch within 30 s, against the brief's 3 minutes;
  - the stitch's working set within 3 GB, mapped tiles included, against the budget's 14 GB;
  - the tiles within 0.3 px RMS and 1 px.

  The example fails outside them.
- **The reads are of mapped tiles,** which the process's I/O counters don't count. The 1.41 GB of
  frames were in the system's cache, as a scan's are when it finishes and auto-stitch begins.
  Read cold, they add their time at the disk's speed: about 3 s at 500 MB/s.
- **A scan becomes a mosaic** in seconds, not the minute or so first thought.
- **Verification:** `bench_stitch_500mp`, in smoke mode in `cargo xtask ci`.
