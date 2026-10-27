# ADR-0061: Pairwise registration for mosaics

- **Status:** Accepted
- **Date:** 2026-10-27
- **Phase:** P8.1
- **Related:** ADR-0013, ADR-0047, ADR-0048, ADR-0050, REQ-STI-002, REQ-NFR-002

## Context

- **A scan's tiles** are placed nominally by their stage poses through the calibration
  (ADR-0013): the stage misses each pose by its error, several µm, tens of pixels at 10×.
  Stitching measures how far each pair of neighbours truly is from the other, then solves all the
  measurements together (P8.2).
- **The registrar** (ADR-0048) finds any shift by phase correlation, coarse to fine, with its
  confidence: the peak's energy, its margin, and the overlap's correlation.
- **A tile's overlaps are a few percent of it:** 15 % strips at the sides, 2 % at the corners. Large
  scans have thousands of pairs; a 100 MP tile's side strip is 15 MP.
- **Specimens vary:** grains are dense; cells leave empty background, where a strip may have
  nothing to register.
- **A job's RAM is budgeted** (ADR-0043): the registration must hold what its estimate says.

## Decision

- **The mosaic's pixels are the frames',** along the camera's axes: a tile at pose `P` has its
  corner at `L·P / binning − size / 2`. The tiles differ by translations alone.
- **The overlap graph** joins tiles whose nominal rectangles overlap by at least 32 pixels each way
  and over at least 5 % of a tile: a grid's sides, not its corners. A pair is registered once,
  `i < j`.
- **A pair's overlaps** are the nominal intersection, as whole pixels of each tile, so that the
  measured shift is the deviation from the nominal offset; flat-corrected, `(value − dark) × gain`,
  when a flat is given, as the flat's gains on the tile's sensor pixels.
- **The search reaches only as far as the stage allows:** 4 standard deviations of a pair's
  offset, √2 of the stage's (5 µm by default), each way, and a pixel for rounding. The registrar
  gains `Options::reach`: peaks whose shift lies beyond it are not weighed, and with none left it
  says `Unreached`. A shift must also leave a quarter of the overlap in common (the registrar's
  `min_overlap`): over a sliver, blobby content correlates by chance. Coarse to fine is the
  registrar's: shrunk to 512 pixels a side, then refined at
  full resolution in windows of the overlap.
- **An edge** is the measured offset of `j`'s corner from `i`'s, with the energy, margin,
  correlation and overlap; it is trusted at an energy of 300 and a correlation of 0.5 (amended in
  P8.2: a correlation of 0.8, a margin of 0.3 and an energy of 50). A pair that
  doesn't register, or has under 16 pixels each way, is an edge that isn't trusted, at its nominal
  offset. A tile that can't be read is an error.
- **Memory:** `bytes_per_pair` is 16 bytes a pixel of the largest overlap, for the corrected
  copies and the registrar's, and 12 MB for its FFTs and windows. At most `threads` pairs run at
  once, in contiguous groups on the calling pool, so a job estimates `threads × bytes_per_pair`.
- **The pixels come mapped:** a dataset's raw frames are mapped read-only; only the overlaps are
  copied.

## Alternatives considered

- **Registering whole tiles:** the same answer for many times the work, and the 85 % of each tile
  that doesn't overlap dilutes the peak.
- **A search without a reach:** a periodic or sparse overlap can show a stronger peak far from
  where the stage could have put the tile. The stage's uncertainty is known; the search uses it.
- **Corners as edges:** they make the graph redundant, but at 2–4 % of a tile they hold little,
  and on sparse specimens rarely register. A lower `min_overlap_share` takes them.
- **A semaphore over every pair on the pool:** finer balancing, but contiguous groups bound the
  memory as simply, and keep the order.

## Consequences

- **Accuracy:** on a 5 × 4 `siq-datagen` scan of 2048 × 1536 tiles, 15 % overlap, the stage 5 µm
  off, every pair is trusted, within 0.025 pixels RMS of the truth. The solver has measurements
  far finer than its 0.3 pixel target.
- **Sparse specimens** leave sides untrusted, which the solver works around. On 512 × 384 tiles
  over cells, a 57-pixel strip registered falsely 32 pixels off with an energy of 129 and a
  correlation of 0.62: the threshold of 300 leaves it out, where every full-size side measures
  thousands.
- **Speed:** 31 pairs of 2048 × 1536 tiles in 51 ms on 16 threads.
- **A calibration's error** in rotation or scale shows as residuals: translations can't absorb
  it.

## Verification

- `crates/siq-stitch/tests/pairs.rs`: tiles placed by their poses; the overlap graph's sides and
  corners; a scan's 12 sides recovered within 0.2 pixels RMS and 0.5 at worst of the truth,
  uncorrected and through a flat; noise alone not trusted.
- `crates/siq-stitch/tests/memory.rs`: the peak heap within `threads × bytes_per_pair`.
- `crates/siq-register/tests/register.rs`: `the_search_reaches_only_as_far_as_asked`.
- `pairs_on_dataset`.

## Amendment (P8.2, 2026-10-27): trusting by correlation and margin

- **An edge is trusted** at a correlation of 0.8, a margin of 0.3, and an energy of 50, not at an
  energy of 300 and a correlation of 0.5.
- **Why:** the energy under-rates an overlap the stage shifted far. On a 4 × 4 scan over grains, two
  sides that the stage had put 37 and 30 pixels off registered within 0.3 pixels of the truth with
  energies of 56 and 220, but correlations of 0.985 and margins near 0.9. The false match of
  smoke mode had a correlation of 0.62 and a margin of 0.20: the correlation and the margin tell
  them apart where the energy can't. The energy stays as a floor against unrelated images (about
  30).
- **Verification:** `crates/siq-stitch/tests/solve.rs` uses all 24 pairs of its scan;
  `crates/siq-stitch/tests/pairs.rs` still distrusts noise alone.
