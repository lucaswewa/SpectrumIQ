# ADR-0047: Synthetic datasets and their ground-truth files

- **Status:** Accepted
- **Date:** 2026-10-25
- **Phase:** P4.9
- **Related:** ADR-0018, ADR-0024, ADR-0026, ADR-0041, ADR-0042, ADR-0044, REQ-SIM-004,
  REQ-STI-004

## Context

- **Algorithms need inputs with exact answers:**
  - stitching (M8) needs the tiles' true positions;
  - depth from focus (M11) needs the true surface;
  - deconvolution (M12) needs the true volume.

  Later, SIM phase sets (M12) and light fields (M13).
- **The pieces exist:**
  - `siq-synth`'s worlds and `write_ground_truth` (ADR-0024);
  - `siq-sim`'s image formation (ADR-0026);
  - the dataset layout and raw frames (ADR-0041);
  - OME-Zarr with pyramids (ADR-0042, ADR-0044).
- **M4 asks for a 500 MP synthetic mosaic** to browse in the viewer.

## Decision

- **`apps/siq-datagen`** writes a dataset per run into a data root, as the service writes them.
  The service, its viewer and its jobs read it with no step between.
  - Each kind of dataset is a **generator**: a subcommand (`tilescan`, `zstack`, `beads`)
    implementing `Generator` (`command`, `generate`), listed in `GENERATORS`. M12 and M13 add
    theirs there.
  - `--seed` (1 by default) seeds the world, the stage errors and the noise. The same seed and
    arguments give the same dataset.
  - `--smoke` gives small sizes, for CI.
- **The dataset:**
  - **its kind** is `synthetic`;
  - **its manifest's plan** is `{generator, seed, args}`;
  - **`raw/`:** its frames, written as the recorder writes them (`siq_storage::write_frame`: raw
    little-endian `u16` and the sidecar). They are keyed by tile (`p`) or by plane (`z`), Mono14
    at 10 ms, and their metadata gives the pose the stage reported;
  - **`zarr/`:** its images, with their pyramids;
  - **`truth/`:** its ground truth:
    - `world.json`: `siq-synth`'s `GroundTruth` over the dataset's region (ADR-0024), with the
      world's description, and its height map (`world.height.f32`) and beads where it has them;
    - `<generator>.json`: what the generator knows beyond the world.
- **The camera** is the simulated instrument's:
  - a 3.76 µm sensor behind a 10×/0.30 objective, so 0.376 µm a pixel;
  - an ideal calibration: stage and image axes aligned;
  - the simulator's optics and sensor models.

  Positions are stage µm (ADR-0018). A frame's pose is its centre's.
- **`tilescan`:**
  - tiles on a grid over a cells or grains world;
  - each frame rendered at its **true pose**, the nominal one plus an error drawn per tile and
    axis (normal, 2 µm a standard deviation), and **recorded with its nominal pose**;
  - `tilescan.json`: each tile's frame, `nominal`, `true` and `error`, and the grid, step and
    region;
  - **the ideal mosaic,** `zarr/truth.ome.zarr`: the world sampled in focus over the nominal tiles'
    extent, at the camera's pixel size, as a frame's levels without vignetting or noise (intensity
    × flux × exposure / e⁻ per ADU, plus the dark offset). It is what a perfect stitch gives, and
    what M8's accuracy is measured against.

  8 × 7 tiles of 4096 × 3072 at 15 % overlap, by default, give a mosaic of 28 467 × 18 739: 533 MP.
- **`zstack`:**
  - a height field seen at evenly spaced focus positions centred on 0;
  - its truth is the height under every pixel (`world.height.f32`);
  - as the simulator has it, the defocus is the focus plus the height, so a plane at focus `f` is
    sharp where the height is `−f`;
  - the stack is also `zarr/stack.ome.zarr`, the planes along `z`.
- **`beads`:**
  - a bead volume, through a 3D PSF that the simulator's frames don't have, since they treat a
    volume as flat;
  - each bead is a Gaussian, its σ its own size and the optics' blur at its defocus (focus plus its
    height) added in quadrature;
  - it keeps its light: it peaks at its brightness in focus, and dims as 1/σ² out of it;
  - then the simulator's noise and digitisation;
  - its truth is the bead list in `world.json`;
  - the stack is `zarr/stack.ome.zarr`.

## Alternatives considered

- **Running the simulated instrument through a scan:**
  - realistic timing, but slow, and bound to the workflow engine (M6);
  - the truth would have to be recovered from the simulator's state.

  Offline rendering is the same image formation, with the truth known by construction.
- **Ground truth in the manifest:** a large file, and the manifest is a dataset's provenance, not
  its answers. `truth/` keeps them apart, and only synthetic datasets have it.
- **A stitched mosaic from the tiles as the 500 MP image:** that is M8's work. The world sampled
  directly is exact, and the reference stitching is measured against.
- **A voxel volume as the beads' truth:** the bead list is exact and small. A volume can be
  rendered from it at any sampling.
- **Generators as separate binaries:** one binary with a registry keeps the dataset helpers and the
  CLI in one place.

## Consequences

- **The viewer browses the 533 MP mosaic** (P4.9 notes):
  - its 2–6 ms tiles;
  - each view whole once OpenSeadragon's animation ends.
- **M8 measures stitching** against `tilescan.json`'s true poses and the ideal mosaic, M11 depth
  from focus against `world.height.f32`, and M12 deconvolution against the beads.
- **The calibration is ideal, and the stage errors independent:** M5's calibration and M8's tests
  of drift add theirs as arguments.
- **A synthetic dataset's frames are noisy, but deterministic.**

## Verification

- `apps/siq-datagen/tests/datagen.rs`, at smoke sizes:
  - each tile's recorded pose is nominal, and its truth's error is consistent and within five
    σ;
  - each tile correlates with the ideal mosaic above 0.9 where the truth puts it, better than
    where the stage said;
  - a z-stack's sharpest plane is within one of the true surface in at least 80 % of blocks;
  - at least 90 % of the beads are brightest within a plane of their depth;
  - the CLI writes a dataset that the catalog finds, and refuses an unknown world.
- `crates/siq-storage/tests/raw.rs`: a frame written from memory reads back as a recorded one.
- **`siq-datagen tilescan`** (release): 56 tiles and the 533 MP mosaic with eight levels, in about
  25 s. They were browsed in the viewer in Edge.
