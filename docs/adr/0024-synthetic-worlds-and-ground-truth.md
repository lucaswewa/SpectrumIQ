# ADR-0024: Synthetic world model and ground-truth conventions

- **Status:** Accepted
- **Date:** 2026-10-24
- **Phase:** P1.4
- **Related:** ADR-0012, ADR-0018, REQ-SIM-004, REQ-SIM-005

## Context

- **The camera simulator** (P1.6, P1.7) renders frames of a virtual specimen at any stage pose,
  ROI, binning and defocus.
- **Generated datasets** (P4.9, P12.4, P13.1) need exact ground truth:
  - the overlap of stitching tiles;
  - the heights for depth from focus;
  - the bead positions for deconvolution and light field.
- **Stitching tests compare overlapping tiles,** so two tiles must see the same specimen where
  they overlap.
- **The simulator samples coarsely** when the image is defocused or the stage is moving (P1.6), so
  the specimen must not alias when it is.
- **Real images must be able to serve as the specimen** (REQ-SIM-005).

## Decision

### A world is a pure function of sample coordinates

- **Sample coordinates are stage coordinates** (ADR-0018).
- **`World::intensity(at, footprint)`** is the in-focus, noiseless intensity at `at`, from 0
  to 1. Optics and the sensor are P1.6's.
- **A world holds no state that sampling changes,** and does no per-region normalization.
  - Two regions sampled apart therefore agree exactly wherever they share a point.
  - The seam test checks that bit for bit, on grids where the shared points' coordinates are
    computed exactly.
- **The other queries:**
  - `surface_z(at)`: the surface height, for 2D and 2.5D worlds. Flat worlds are at z = 0;
  - `density(at, z)`: for volumes;
  - `beads(region)`: the beads in a region;
  - `info()`: the world's description.
- **`sample(world, &PixelToStage, out)`** renders on any pixel grid, including a frame's own
  mapping with its rotation and flip. `grid(origin, scale)` gives the axis-aligned grid.

### The footprint prevents aliasing

The footprint is the pixel's width on the sample. A world averages away detail finer than it:

- **fractal octaves** fade to their mean as their period drops from two footprints to one;
- **edges** are linear ramps one footprint wide;
- **beads** widen by the variance of a box one footprint wide, footprint² / 12, keeping their
  total;
- **the image world** reads a pyramid of 2 × 2 means, linear between the two levels around the
  footprint.

### Deterministic randomness without state

- **Every random choice is a hash of a lattice point.** It is SplitMix64's finaliser over a key
  per world and use, plus the coordinates times large odd constants.
  - There is no generator state, so the order of evaluation doesn't matter.
  - It is thread-safe, and the worlds are unbounded.
- **Objects sit on jittered lattices,** at most one per square or cube. A point looks only at the
  squares within an object's reach.
- **Grain boundaries are exact.**
  - The distance to the boundary is the distance to the bisector with each neighbouring seed:
    (d² − d₁²) / 2|p − p₁|.
  - The 3 × 3 seeds suffice unless the nearest seed and its boundary could reach past them. Then
    the 5 × 5 seeds are searched.
- **Each octave of value noise is turned by its own random angle,** so the octaves' lattices
  don't line up into streaks.

### The worlds

| World | What | Defaults |
|---|---|---|
| `Cells` | Elliptical cells with nuclei and texture, on an uneven background | 40 µm grid, 60 % occupied, semi-axes 8–16 µm |
| `Grains` | Voronoi grains, each with its own reflectance, with dark boundaries | 60 µm grains, 1.5 µm boundaries |
| `HeightField` | A textured relief of known height | ±40 µm over 400 µm, texture 5 µm |
| `Beads` | Gaussian beads in a slab, seen from above in focus, with a 3D density | 15 µm grid, 50 % occupied, z −20 to 20 µm, σ 0.1 µm |
| `ImageWorld` | A PNG or TIFF at a pixel size, centred on the origin, dark outside | — |

Every parameter has a default, and every world has a seed.

### Descriptions and ground truth

- **`WorldInfo`** is a JSON object tagged by `kind`, holding the parameters and the seed.
  - `build()` makes the same world again. Missing parameters take their defaults.
  - `from_config(&WorldConfig, seed)` gives the world that `[sim.world]` names.
- **`write_ground_truth(world, region, scale, dir, name)`** writes:
  - `<name>.json`: the `GroundTruth`, with the world's description, the region, the height map's
    entry and the bead list (x, y, z, σ, brightness);
  - for a world with a surface, `<name>.height.f32`: raw little-endian `f32` heights in µm, row
    after row. Pixel `(c, r)` is the height at the region's origin plus `(c + 0.5, r + 0.5)`
    times the scale.
- **Cells and grains have no further ground truth.** Their description rebuilds them exactly.

### The image world

- **Files:**
  - PNG, through `png` 0.18;
  - TIFF, through `tiff` 0.11, uncompressed, LZW or deflate;
  - grey or colour, reduced to luminance with the Rec. 709 weights;
  - 8 or 16 bits, or 32-bit float for TIFF.
- **Loading:** the format is chosen by the file's first bytes. The image is loaded whole as
  `f32`, with its pyramid: a 20 000 × 20 000 image takes about 2 GB.

### Brief deviations

1. **`sample(region: RectUm, scale, out)` became `sample(world, &PixelToStage, out)` plus
   `grid(origin, scale)`.** P1.6 renders through a frame's own mapping, which can rotate and
   flip; `grid` gives the brief's axis-aligned case.
2. **Point evaluation, `intensity(at, footprint)`, is the method a world implements.** `sample`
   is built on it, so every world samples the same way.
3. **`surface_z` and `density` take a `Vec2Um`,** and return `Option`s: `None` where the world
   has no surface, or isn't a volume.
4. **Added:** `beads(region)`, `info()`, `WorldInfo::build` and `WorldInfo::from_config`.

## Measurements

Single-threaded, at 0.345 µm per pixel, on the development PC:

| World | Time per pixel |
|---|---|
| `Cells` | 133 ns |
| `Grains` | 133 ns |
| `HeightField` | 67 ns |
| `Beads` | 83 ns |

## Alternatives considered

- **Pre-rendered textures or tiles:** bounded, large in memory, and they alias or blur off their
  own scale.
- **A random-number generator seeded per region:** the result depends on the order of
  evaluation, so seams would differ.
- **Gradient (Perlin or simplex) noise:** fewer lattice artefacts, but slower. Value noise with
  rotated octaves is enough for texture.
- **The `image` crate:** a large dependency for the two formats needed.

## Consequences

- **A 100 MP frame takes 7–13 s on one core.**
  - P1.6 must parallelize, cache the noiseless render for a still pose, and render at a reduced
    resolution when defocus or motion blur allows it.
  - Faster, coherent sampling of whole rows is a possible later optimization.
- **Datasets can be regenerated** from their `WorldInfo` alone.
- **A large real image costs RAM** in proportion to its size.

## Verification

- `siq-synth` tests:
  - `every_world_is_the_same_for_a_seed_and_different_for_another`;
  - `regions_sampled_apart_agree_wherever_they_meet`: a proptest over the five worlds, five
    scales and overlapping regions, bit for bit;
  - `every_world_stays_within_its_expected_statistics`: cell cover, boundary share, relief, bead
    count and peaks;
  - `a_coarse_footprint_averages_detail_away`;
  - `a_world_description_round_trips_and_builds_the_same_world`;
  - `ground_truth_is_written_beside_a_dataset_and_reads_back`;
  - the image world's PNG and TIFF reading, pyramid, and errors.
- The `render_world` example.
