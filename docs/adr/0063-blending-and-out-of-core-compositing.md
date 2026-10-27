# ADR-0063: Blending and out-of-core compositing

- **Status:** Accepted
- **Date:** 2026-10-27
- **Phase:** P8.3
- **Related:** ADR-0041, ADR-0043, ADR-0044, ADR-0050, ADR-0062, REQ-STI-002, REQ-STI-003,
  REQ-NFR-002

## Context

- **The solve** (ADR-0062) gives each tile's corner on the mosaic, to a few hundredths of a pixel.
  The mosaic must now be written: 500 MP and more, of every channel and plane, into OME-Zarr
  (ADR-0041), within a job's RAM budget of 14 GB (ADR-0043), on a disk that holds the frames.
- **Tiles overlap** by 10–20 %, and differ in their overlaps by the vignetting the flat corrects
  (ADR-0050), by noise, and by the sub-pixel error left.
- **The frames are on disk** as raw files, each mappable read-only; a 100 MP tile is 200 MB.

## Decision

- **The image** is the box of the tiles at their corners, to whole pixels, in OME-Zarr chunks of
  512 (16 bits). Its pixel size is the calibration's, binned; its origin the stage point of its
  first pixel through the calibration's inverse. Each `(t, c, z)` of the frames is a plane, and
  every plane shares the reference's corners by the frame's position `p`.
- **The work** is blocks of 2 × 2 chunks, in parallel on the job's pool: each block gathers the
  tiles that meet it from a grid of cells as large as the largest tile, accumulates each tile's
  weighted samples into a sum and a weight a pixel, divides, and writes its chunks. A pixel no tile
  covers is 0.
- **Sampling:** each mosaic pixel's centre is mapped into the tile by its corner, and sampled
  bilinearly, clamped to the edge's pixel within half a pixel of it; or, with `integer`, the corner
  rounded and the pixel copied.
- **The flat:** `(value − dark) × gain`, the gains kept on a grid 16 pixels apart and interpolated,
  once for each place on the sensor and size of tile.
- **Blending:** linear feathering, a weight `ramp(u) · ramp(v)` that rises from 10⁻³ at a tile's
  edge to 1 at 128 pixels in (at most half its side), so that in an overlap each tile fades into
  the next, and at the mosaic's border a lone tile keeps its pixels.
- **Memory:** a block's sum and weight (8 bytes a pixel) and its chunks as filled, copied whole and
  encoded, and the encoder's context, a thread: `composite::bytes`. The tiles are mapped as they
  are read, at most a few a thread kept mapped, the least recently used let go first.
- **The job** (`kinds::CompositeJob`, in `siq-jobs`): the frames by key and their corners, the
  calibration, the channels and the flat; the composite as 80 % of its progress, then the pyramid
  (ADR-0044) as the rest. Its estimate is the larger of the two stages'. The stitch job (P8.4) runs
  it after the registration and the solve.

## Alternatives considered

- **Averaging the tiles equally in overlaps:** a step in brightness at each tile's edge wherever the
  flat is off, as the average changes from one tile to two.
- **Multi-band blending:** seamless where the flat leaves a step, but several times the work and
  memory, and it smooths what a seam shows. The flat is corrected first; the feather is enough.
- **The newest or nearest tile wins**, as the live canvas does (ADR-0059): hard seams wherever the
  sub-pixel error is left.
- **Reading each tile whole into RAM:** 200 MB a 100 MP tile; a few blocks touch a few tiles, whose
  pages the mapping reads.
- **A full-size accumulator:** 8 bytes a pixel of a 500 MP mosaic is 4 GB; a block is 8 MB.

## Consequences

- **Quality:** a 3 × 3 scan over grains composited at its true corners through its flat matches the
  ideal mosaic at 28.0 dB, against 16.2 dB at the stage's; the ideal has neither the tiles' blur
  nor their noise, which set that ceiling. End to end on 4 × 3 tiles of 2048 × 1536 over cells,
  registered, solved and flat-estimated from the frames: 35.4 dB.
- **Seams:** two neighbours composited apart differ by 0.1 % of the level in their overlap, through
  the flat; without it, by 3 to 5 %.
- **Speed:** 30 MP composited in 0.20 s on 16 threads (149 MP/s), its pyramid in 0.17 s.
- **Memory:** a 3481 × 2610 mosaic on two threads held at most 5.0 MB of heap, against 13.4 MB
  estimated and 18.2 MB of mosaic.
- **Sub-pixel resampling smooths a little:** bilinear interpolation softens each tile by up to half
  a pixel's blur; `integer` keeps the pixels as they are, at half a pixel's error at most.

## Verification

- `crates/siq-stitch/tests/composite.rs`: the PSNR at the true and the stage's corners; the
  neighbours' steps with and without the flat; whole pixels copied, and uncovered ones 0.
- `crates/siq-stitch/tests/composite_memory.rs`: the heap's peak within `composite::bytes`, under
  half the mosaic.
- `crates/siq-jobs/tests/kinds.rs`: a scan composited as a job, with its pyramid, and recorded.
- `composite_dataset`.
