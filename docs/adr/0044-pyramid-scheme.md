# ADR-0044: The pyramid scheme: filter, level count and chunk alignment

- **Status:** Accepted
- **Date:** 2026-10-25
- **Phase:** P4.4
- **Related:** ADR-0042, ADR-0043, REQ-DATA-003, REQ-DATA-005, REQ-STI-003

## Context

- **Every image the viewer shows is an OME-Zarr image** (ADR-0042). Deep zoom needs its lower
  resolutions: a 100 MP snap is 46 × 34 tiles at full resolution, and one at the top.
- **Mosaics reach 500 MP and more** (REQ-STI-003). Their levels must be built without holding
  the image: a 20 000 × 20 000 `u16` mosaic is 800 MB, and jobs share 14 GB.
- **The levels' metadata** (scale and translation) is ADR-0042's: level `l` halves `l − 1` in
  `y` and `x`, rounding up, and its translation centres its pixels on level 0's.
- **Building runs as a job** (ADR-0043), on its below-normal pool, cancellable, with progress.

## Decision

- **The filter is the 2 × 2 mean:** each pixel is the mean of the 2 × 2 block below it.
  - At the right and bottom edges of an odd-sized level, it is the mean of the one or two pixels
    left.
  - `u16` rounds to the nearest, halves up; `f32` is exact.
  - It is the box filter that NGFF's `type: "mean"` names.
- **The level count:** level 0, then halving until the image fits `TOP` = 256 pixels both ways:
  about one deep-zoom tile.
  - A 100 MP snap (11648 × 8742) has 7 levels, down to 182 × 137.
  - A 20 000² mosaic has 8.
- **Chunk alignment:** every level has the same 512 × 512 chunks. So a chunk of level `l + 1` is
  the mean of exactly four chunks of level `l`, the 2 × 2 block below it.
  - **Each chunk is built alone:** its four source chunks read, the mean taken, and the chunk
    written, once. There is no read-modify-write, and no chunk is shared between tasks.
  - **The order:** levels in turn, since each needs the one before; within a level, one chunk row
    at a time, its chunks and planes in parallel on the calling rayon pool.
  - **The memory held** is a few chunks per thread: 2.5 MB of `u16` pixels, and the codec's
    buffers, whatever the image's size.
  - **The estimate** given to the scheduler is threads × 5 chunks × 2 for the codec.
- **The builder** (`siq_pyramid::build`):
  - adds the missing levels to the image and lists them in its metadata
    (`ZarrImage::set_levels`);
  - reports progress by chunks done, and stops when cancelled, with the levels part-built;
  - runs through a small `Control` trait, since processing (L3) can't use the job system (L4).
- **The job:** `siq_jobs::kinds::PyramidJob` wraps the builder. Job adapters live in
  `siq_jobs::kinds` for the same layering reason.

## Alternatives considered

- **Gaussian or Lanczos filters before decimation:** smoother levels, but a wider footprint that
  crosses chunk edges, and values a mean wouldn't give. NGFF viewers expect a mean or a nearest
  pyramid, and the box filter keeps intensity sums.
- **Nearest neighbour:** fast, but aliasing in textured specimens, and noise kept at full
  strength.
- **Down to one pixel:** tiny levels that the viewer never asks for.
- **Building level `l + 2` straight from level `l`:** one read fewer, but 4 × 4 blocks of chunks
  held, and rounding that differs from building level by level.
- **Holding whole chunk rows of every level:** the brief's bound of about three chunk rows a level.
  Chunk-sized tasks need less, and don't grow with the image's width.

## Consequences

- **Levels are consistent:** each is the mean of the one below, exactly, as a naive build would
  give.
- **The build is out of core and parallel.** On this PC (16 threads, NVMe), a 20 000² image's 7
  levels took 1.3 s, and the process's peak working set was 84 MB.
- **A cancelled build leaves part-built levels.** Running the job again rebuilds them all.
- **Changing the chunk size between levels isn't possible:** the alignment depends on one size.

## Verification

- `crates/siq-pyramid/tests/pyramid.rs`:
  - every level of an odd-sized `u16` image (300 × 173, 2 channels, chunks of 64) equals a naive
    in-memory pyramid;
  - `f32` levels are exact means;
  - the level count;
  - the metadata as built (`snapshots/pyramid__datasets.snap`), and the levels' shapes;
  - a build cancelled after three chunks.
- `crates/siq-pyramid/tests/memory.rs`: a counting allocator measures the build of a 4096²
  image's levels on two threads. Its peak of live heap is 8.4 MB, below three chunk rows
  (12.6 MB), against the image's 32 MB.
- `crates/siq-jobs/tests/kinds.rs`: a dataset's image built as a job, with its levels, values and
  record.
- **`build_pyramid`:** a 20 000 × 20 000 image's pyramid.
