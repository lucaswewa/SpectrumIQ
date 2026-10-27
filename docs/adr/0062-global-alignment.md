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
