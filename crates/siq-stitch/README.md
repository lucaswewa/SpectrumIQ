# siq-stitch

Post-acquisition stitching (REQ-STI-002, ADR-0013): a scan's tiles placed by their stage poses,
their overlaps registered pairwise, the placement solved globally, and the mosaic composited
out of core into OME-Zarr.

- **The tiles** (`Mosaic::place`) are placed through the calibration: a frame taken at pose `P`
  has its corner at `L·P / binning − size / 2` on the mosaic, whose pixels are the frames', along
  the camera's axes. Rotation, scale and shear are the calibration's; the tiles differ by
  translations alone.
- **The overlap graph** (`Mosaic::overlaps`) joins the tiles whose nominal rectangles overlap by at
  least 32 pixels each way, over at least 5 % of a tile: the sides of a grid, not its corners.
- **Each pair** (`pairs::register`, ADR-0061) is registered by phase correlation (`siq-register`)
  of the two tiles' nominal overlaps, as whole pixels of each:
  - flat-corrected, `(value − dark) × gain`, when a flat is given;
  - searched shrunk to 1024 pixels a side, then refined at full resolution in windows of up to
    1024 (ADR-0061, amended in P8.5);
  - only within the reach the stage's uncertainty allows: 4 standard deviations of a pair's
    offset, √2 of the stage's 5 µm, each way;
  - only at shifts that leave a quarter of the overlap in common: a sliver matches by chance.
- **An edge** is the measured offset of the second tile's corner from the first's, its energy,
  margin and correlation, the overlap, and whether it is trusted: a correlation of at least 0.8, a
  margin of at least 0.3, and an energy of at least 50 (ADR-0061, amended). A pair that doesn't
  register is an edge that isn't trusted.
- **The solution** (`solve::solve`, ADR-0062) is each tile's corner, by weighted least squares
  over translations: the trusted edges at a quarter pixel each at a correlation of 0.95 or more,
  less below it (a fifth at 0.8: ADR-0062, amended), and each tile held to its stage position as
  loosely as the stage's 5 µm. The normal equations are factored below 500 tiles, and
  solved by conjugate gradients beyond. Huber's weights temper the edges that disagree; the edge
  that disagrees most is rejected while it is over 4 robust standard deviations and a pixel, and
  the rest solved again. Each connected component keeps its tiles' mean where the stage put them;
  a tile without an edge stays at its own. All channels and planes share the reference's solution.
- **The mosaic** (`composite::composite`, ADR-0063) is written into OME-Zarr a block of 2 × 2
  chunks at a time, in parallel: each block from the tiles that meet it, found by a grid of cells,
  each mosaic pixel sampled bilinearly from each tile at its solved corner (or copied, with
  `integer`), flat-corrected, and weighted by ramps that rise from a tile's edge to 1 at 128 pixels
  in; then normalised. A block holds its sums and weights and its chunks: `composite::bytes`.
- **Memory:** at most `threads` pairs are registered at once, on the calling rayon pool, each
  holding at most `bytes_per_pair`.
- **The report** (`Report`, ADR-0064) is where each tile went, how far it moved and its worst
  residual, and each pair's residual, trust and use, with a `Summary`: the stitch's QA, written
  beside its image, and its measures for the dataset's history.
- **The pixels** come from a `TileSource`: a dataset's raw frames mapped read-only as they are
  read, at most `capacity` kept mapped, the least recently used let go first (`MappedTiles`), or
  images in memory.

## Place in the architecture

- **Layer:** L3 Processing.
- **Depends on:** `siq-register`, `siq-calib`, `siq-storage`, `siq-types`; `rayon`, `schemars`,
  `serde`, `thiserror`; `siq-datagen`, `siq-sim`, `siq-pyramid`, `serde_json` and `tempfile` in the
  tests and the examples.
- **Used by:** `siq-jobs`' `CompositeJob` and `StitchJob`; `siq-server`, for the report's overlays.
- **Phase:** P8.1; P8.2 added the global solve; P8.3 the compositing; P8.4 the report and
  `StitchOptions`; P8.5 the accuracy suite, the search's size and the edges' correlation weights;
  P10.7 colour tiles (ADR-0071, amended): registered on their luminance, and composited in red,
  green and blue.

## API

| Item | What |
|---|---|
| `Mosaic::place(&frames, &l)` | The tiles of `(FrameKey, FrameMeta)`s placed by their poses through `l`, sensor pixels a µm: `Mosaic { tiles, px_per_um }` |
| `mosaic.overlaps(min_px, min_share)` | The pairs `(i, j)`, `i < j`, whose nominal rectangles overlap by `min_px` each way over `min_share` of a tile |
| `Tile` | `key`, `size`, `nominal` (its corner, in mosaic pixels), `sensor` (its first pixel on the sensor), `binning`, and a colour frame's `cfa`. A colour tile's pairs are registered on its luminance |
| `pairs::register(&source, &mosaic, &pairs, shading, &options, threads)` | Each pair's `Edge`, in order; `shading` the dark level and the flat, or `None` |
| `Edge` | `i`, `j`, `shift` (`j`'s corner from `i`'s, in mosaic pixels), `pce`, `margin`, `ncc`, `overlap_px`, `confident` |
| `PairOptions` | `min_overlap_px` (32), `min_overlap_share` (0.05), `stage_sigma_um` (5), `reach_sigmas` (4), `search_px` (1024), `min_shared` (0.25), `min_pce` (50), `min_ncc` (0.8), `min_margin` (0.3); `reach_px(px_per_um)` |
| `pairs::bytes_per_pair(&mosaic, &pairs, &options)`, `pairs::search_bytes(&options)` | The most a pair's registration holds, from the largest overlap; its FFTs' and windows' share, from `search_px` |
| `TileSource`, `Pixels` | Where pixels come from, by a tile's index: `pixels(i)` → `Pixels`, borrowed or a mapping held, whose `view()` is the image; `[OwnedImage<u16>]` is one |
| `MappedTiles::open(&reader, keep)`, `.with_capacity(n)` | A dataset's frames that `keep` takes, and their metadata, mapped as read; at most `n` kept mapped |
| `composite::composite(&source, &planes, &bounds, &image, shading, &options, &control)` | `planes` written into level 0 of `image`, a block at a time on the calling pool; `control` hears the progress and may stop it |
| `Bounds::of(&corners, &sizes)` | The box of the tiles at their corners: `Bounds { origin, size }`, in mosaic pixels |
| `Plane { at, tiles, colour }`, `Placed { source, tile, corner }` | A plane `(t, c, z)` of the image, and its tiles: each the source's index, the tile, and its corner. `colour`, 0 red, 1 green, 2 blue, is the colour the plane takes of colour tiles, each demosaiced where a block samples it, with a margin of 2 |
| `CompositeOptions` | `feather_px` (128), `integer` (false), `block_chunks` (2) |
| `composite::bytes(chunk, &options, threads)` | The most the composite holds |
| `solve::solve(&mosaic, &edges, &options)` | The tiles' corners that best agree with the trusted edges and the stage: a `Solution` |
| `Solution` | `positions` (each tile's corner), `edges` (each edge's `Residual`: `residual`, `weight`, `used`), `tiles` (each tile's `TileReport`: `moved_px`, `worst_px`, `component`), `components`, `method`, `rms_px` and `max_px` of the edges used, `solves` |
| `SolveOptions` | `stage_sigma_um` (5), `edge_sigma_px` (0.25), `huber_k` (1.345), `reject_k` (4), `reject_px` (1), `iterations` (10), `dense_below` (500), `method` (`Dense` or `ConjugateGradient`; by size if `None`) |
| `Report::new(&mosaic, &edges, &solution, &bounds)` | The QA: `bounds`, `tiles` (`TileQa`: `p`, `corner` in the image, `size`, `moved_px`, `worst_px`, `component`), `edges` (`EdgeQa`: `i`, `j`, `residual_px`, `pce`, `ncc`, `confident`, `used`), and `summary` (`Summary`: tiles, pairs, trusted, rejected, components, method, `rms_px`, `max_px`, `moved_px`) |
| `StitchOptions` | `pairs`, `solve` and `composite`: each its options, the defaults if left out |
| `StitchError` | No pose, binnings that differ, a tile that isn't there or isn't 16-bit, storage, image views, cancelled |

## Examples

| Example | What it shows | Run |
|---|---|---|
| `pairs_on_dataset` | A `siq-datagen` scan of 5 × 4 tiles of 2048 × 1536 over cells, or a dataset of yours: its pairs registered, those not trusted, and the offsets against the truth and the stage's own error | `cargo run --release -p siq-stitch --example pairs_on_dataset [-- <dataset folder>]` |
| `composite_dataset` | A `siq-datagen` scan of 4 × 3 tiles of 2048 × 1536 over cells with its ideal mosaic, or a dataset of yours: registered, solved, its flat estimated from its frames, composited as `zarr/stitched.ome.zarr` with its pyramid, and set against the ideal | `cargo run --release -p siq-stitch --example composite_dataset [-- <dataset folder>]` |
| `solve_on_dataset` | A `siq-datagen` scan of 6 × 5 tiles of 2048 × 1536 over cells, or a dataset of yours: its pairs registered and solved, the edges rejected, and the tiles' placement against the truth and the stage's | `cargo run --release -p siq-stitch --example solve_on_dataset [-- <dataset folder>]` |
| `tune_stitch` | The defaults tuned (P8.5): the search's reach and size, Huber's k and the feather, each varied about its default over 5 × 4 tiles of 2048 × 1536 over grains, and over cells in a tenth of the light: the placement against the truth, the pairs trusted and the time; the composite against the ideal mosaic, with the flat and without | `cargo run --release -p siq-stitch --example tune_stitch` |

On the 16-thread workstation:

- **`pairs_on_dataset`** registers the 31 pairs of its 20 tiles in 86 ms, every one trusted,
  within 0.018 pixels RMS of the truth (0.033 at worst), where the stage was 22.7 pixels RMS off.
- **`solve_on_dataset`** places its 30 tiles within 0.013 pixels RMS of the truth (0.039 at
  worst), where the stage was 17.2 RMS and 30.6 at worst; the solve takes 39 µs. Smoke mode
  (`SIQ_SMOKE=1`) makes 3 × 3 tiles of 512 × 384: their 57-pixel strips over sparse cells leave a
  pair untrusted, and the solve places the tiles within 0.087 pixels RMS.
- **`composite_dataset`** composites 30.4 MP in 0.20 s (154 MP/s) and its pyramid in 0.16 s,
  matching the ideal mosaic at 35.4 dB once the stage's mean error is taken out.
- **`tune_stitch`'s** findings are in ADR-0061, ADR-0062 and ADR-0063's P8.5 amendments. In short,
  the search's 1024 pixels place dim cells within 0.027 px RMS, where 512 placed them within
  0.064, and the reach, Huber's k and the feather change little about their defaults.

## Features

None.

## Testing

`cargo test -p siq-stitch`:

- tiles placed by their poses through a turned calibration and binning; a grid's sides joined, its
  corners only below a lower share;
- a `siq-datagen` scan of 3 × 3 tiles over grains, the stage off by 5 µm: its 12 sides registered
  within 0.2 pixels RMS and 0.5 at worst of the truth (0.064 and 0.126 here), uncorrected and
  through a flat; and in colour, on luminance (0.120 and 0.299);
- an overlap of noise alone not trusted, and a textured one 3 and 2 pixels off its nominal place
  found within 0.05;
- `tests/memory.rs`: a counting allocator holds the registration within `threads` ×
  `bytes_per_pair`, one pair at a time and two;
- `tests/solve.rs`: a `siq-datagen` scan's 24 pairs, two made 15 and 9 pixels wrong, solved within
  0.3 pixels RMS and 1 at worst of the truth, the two rejected; a drift ramp of 3.4 pixels removed
  to 0.12; a tile's strong true edge kept over its weak false one; disconnected columns each kept
  at their stage mean, a tile alone at its own; the dense and conjugate-gradient solvers within
  10⁻⁶ pixels;
- `tests/accuracy.rs` (P8.5): `siq-datagen` scans over grains and cells, of different sizes,
  overlaps, stage errors and light, each placed within 0.3 pixels RMS and 1 at worst of the truth:
  three small ones in CI, and the matrix of 18 scans of 2048 × 1536 tiles with
  `cargo test --release -p siq-stitch --test accuracy -- --ignored` (48 s);
- `tests/composite.rs`: a 3 × 3 scan through the simulator's flat, composited at its true corners,
  against the ideal mosaic at 25 dB at least (28.0 here) and 6 dB over the stage's corners (16.2);
  two neighbours composited apart, within 0.5 % of each other in their overlap through the flat
  (0.1 %) and over 2 % without it; whole pixels copied, and uncovered ones 0; a colour scan in
  red, green and blue, each at 25 dB at least (27.1–27.6), and red at 0.743 and blue at 0.545 of
  green, as the lamp gives;
- `tests/composite_memory.rs`: a 4 × 4 mosaic of 18.2 MB composited on two threads within
  `composite::bytes`, 5.0 MB at the peak.

## Limitations

- **Translations only,** by design (ADR-0013): a calibration's error shows as residuals.
- **A colour mosaic costs five times a mono one** to composite: three planes, each block
  demosaicing what it reads of a tile once for each.
- **Sparse overlaps aren't trusted:** cells far apart leave a side with nothing to register; the
  solver leans on the tile's other sides and its stage prior.
- **Narrow overlaps can mislead:** where a pair's stage error is a large share of its overlap, as
  15 % of 512 pixels against 5 µm, a smooth specimen's narrow common strip can register a pixel
  off and still be trusted: one scan in 40 of 3 × 3 such tiles was placed at 0.35 px RMS
  (ADR-0061, amended in P8.5).
- **The stage prior is weak, not absent:** it pulls a drifting row's ends toward where the stage
  said, by about a tenth of a pixel over ten tiles.
- **Bilinear resampling softens** each tile by up to half a pixel's blur; `integer` copies, at
  half a pixel's error at most.
- **Feathering, not multi-band blending:** a flat that is off shows as a gentle ramp across the
  overlap, not a step.
- **The dense solve is cubic:** 0.78 s a solve at 1960 tiles, against 2 ms by conjugate gradients
  (15 ms at 10 000 tiles), so it stops at 500.
- **Corners aren't registered** by default: under 5 % of a tile, they add little.
