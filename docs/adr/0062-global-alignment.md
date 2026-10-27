# ADR-0062: Global alignment of a mosaic's tiles

- **Status:** Accepted
- **Date:** 2026-10-27
- **Phase:** P8.2
- **Related:** ADR-0013, ADR-0061, REQ-STI-002, REQ-STI-004

## Context

- **The pairs** (ADR-0061) measure how far each tile truly is from its neighbours, to a few
  hundredths of a pixel where they are trusted. Some aren't: sparse overlaps, and the odd false
  match that looked sure.
- **The measurements disagree** around loops by their errors, and say nothing about the mosaic's
  place as a whole: the stage does, to its σ, a few µm.
- **Rotation, scale and shear are the calibration's** (ADR-0013): the unknowns are each tile's
  translation, two numbers.
- **Scans range** from a few tiles to thousands; a stage may drift along a scan.
- **The goal:** within 0.3 pixels RMS and 1 at worst of the truth, on `siq-datagen` scans.

## Decision

- **Weighted least squares over the tiles' deviations from their stage positions,** `d`:
  each trusted edge says `d_j − d_i = m`, its offset less the nominal one, at a weight of
  `1 / σ_edge²` (a quarter pixel by default); each tile says `d_i = 0`, at `1 / σ_stage²` (the
  stage's 5 µm, in mosaic pixels). The normal equations, `(λI + L) d = B m`, with `L` the edges'
  weighted Laplacian, share their matrix between x and y.
- **Two solvers:** Cholesky's factoring of the dense matrix below 500 tiles, and conjugate
  gradients, preconditioned by the diagonal, to a relative residual of 10⁻¹², at or beyond. Either
  may be asked for.
- **Robustness:**
  - Huber's weights, at 1.345 of the edges' robust standard deviation (the median residual over
    1.18, as for 2D Gaussian residuals, and at least `σ_edge`), reweighted up to 10 times;
  - then the edge that disagrees most is rejected while its residual is over 4 standard deviations
    and a pixel, and the rest solved again: one at a time, as an outlier pulls its neighbours too;
  - untrusted edges, low in correlation or margin, are never used.
- **Connectivity:** the prior anchors each connected component of the used edges, so each keeps
  its tiles' mean at their stage positions, and a tile without an edge stays at its own.
- **The report:** each edge's residual, final weight and use; each tile's move from the stage,
  worst residual and component; the components, the method, the used edges' RMS and largest
  residual, and the solves made.
- **Channels and planes** share the reference's solution: registration runs on one.

## Alternatives considered

- **Holding one tile fixed** instead of a prior on all: it leaves other components unplaced, and
  puts the whole error of a long scan's drift at its far end.
- **A full bundle adjustment** with rotation and scale per tile: the calibration fixes them, and
  per-tile freedom would absorb the measurements' noise as distortion.
- **Rejecting every edge over the bound at once:** an outlier's pull puts its neighbours over the
  bound too; on a 4 × 4 scan with two outliers, two good edges went with them.
- **Dense below 2000 tiles,** as first planned: a factoring takes 0.78 s at 1960 tiles, against
  2 ms by conjugate gradients, and the solve reweights up to 10 times a round.

## Consequences

- **Accuracy:** 30 tiles of 2048 × 1536 over cells, the stage 5 µm off, placed within 0.058 pixels
  RMS of the truth (0.175 at worst), where the stage was 17.2 RMS; two outliers of 15 and 9 pixels
  rejected on a 4 × 4 scan, which then places within 0.071 RMS.
- **Speed:** 39 µs for 30 tiles; 15 ms for 10 000 tiles by conjugate gradients.
- **The prior is weak, not absent:** a row drifting 0.7 pixels a tile is placed within 0.12 pixels
  of the truth over ten tiles; its ends are pulled toward the stage.
- **A component without a link** to the rest is placed only as well as the stage: the report names
  it.

## Verification

- `crates/siq-stitch/tests/solve.rs`: a scan's pairs with two outliers solved within the targets,
  the outliers rejected; a drift ramp removed; disconnected components at their stage means; the
  two solvers within 10⁻⁶ pixels.
- `solve_on_dataset`.

## Amendment (P8.5, 2026-10-27): weighing the edges by their correlation, and the accuracy suite

- **Each edge weighs `ncc / (1 − ncc) / 19` of `1 / σ_edge²`**, its correlation held within 0.5 and
  0.95: 1 at 0.95 and above, a fifth at 0.8. Huber's weights temper it as before.
- **Why:** two noisy views of the same content correlate as signal / (signal + noise), and an
  offset's variance goes as noise / signal, so as `(1 − ncc) / ncc`. With equal weights, a weak
  false match won where a tile had only two edges:
  - in 2 of 30 scans of 3 × 3 tiles of 512 × 384 over grains, a tile's true edge (a correlation of
    0.995) and its false one (0.806, 18 pixels off) shared their cycle's misclosure evenly;
  - the stage's prior settled it, toward the false one, which lay nearer where the stage said;
  - so the true edge was rejected, and the tiles were placed at 5.6 and 4.3 px RMS.

  Weighed by correlation, the same scans were placed within 0.061 and 0.118 px RMS. Over 40 seeds
  the mean was 0.104, the worst 0.35: the narrow strip of ADR-0061's amendment.
- **Why not more:** above 0.95, a correlation no longer tells a better match. A false edge 9 pixels
  off at 0.994 outvoted two true ones at 0.967 and 0.973 when it weighed five times as much.
  Held at 0.95, true edges count alike, and the majority wins.
- **Huber's k stays at 1.345.** With every fifth edge 0.8 pixels wrong, under the rejection's
  pixel, k from 0.5 to unbounded placed the tiles within 0.334 to 0.359 px RMS. 1.345 is the
  usual 95 % efficiency, and in the middle of them.
- **The accuracy suite** (`tests/accuracy.rs`) covers 18 scans of 2048 × 1536 tiles:
  - grains and cells, 3 × 3, 5 × 5 and 10 × 10 at 15 % and 5 µm;
  - each of a 5 × 5 scan's overlap (10 and 20 %), stage error (2 and 10 µm) and light (a tenth and
    a fiftieth) either way.

  Every scan was placed within the targets. All were within 0.035 px RMS and 0.067 at worst but
  one: sparse cells in a fiftieth of the light, at 0.115 RMS and 0.289 at worst, 35 of its 40
  pairs trusted. The stage alone was 6.5 to 37.5 px RMS. CI runs three small scans, the matrix
  runs in release in 48 s.
- **Verification:** `crates/siq-stitch/tests/solve.rs`:
  `where_a_tile_s_two_edges_disagree_the_better_match_wins`; `crates/siq-stitch/tests/accuracy.rs`;
  `tune_stitch`.
