# siq-stitch

Post-acquisition stitching (REQ-STI-002, ADR-0013): a scan's tiles placed by their stage poses,
their overlaps registered pairwise, the placement solved globally (P8.2), and the mosaic
composited out of core into OME-Zarr (P8.3).

- **The tiles** (`Mosaic::place`) are placed through the calibration: a frame taken at pose `P`
  has its corner at `L·P / binning − size / 2` on the mosaic, whose pixels are the frames', along
  the camera's axes. Rotation, scale and shear are the calibration's; the tiles differ by
  translations alone.
- **The overlap graph** (`Mosaic::overlaps`) joins the tiles whose nominal rectangles overlap by at
  least 32 pixels each way, over at least 5 % of a tile: the sides of a grid, not its corners.
- **Each pair** (`pairs::register`, ADR-0061) is registered by phase correlation (`siq-register`)
  of the two tiles' nominal overlaps, as whole pixels of each:
  - flat-corrected, `(value − dark) × gain`, when a flat is given;
  - searched shrunk to 512 pixels a side, then refined at full resolution;
  - only within the reach the stage's uncertainty allows: 4 standard deviations of a pair's
    offset, √2 of the stage's 5 µm, each way;
  - only at shifts that leave a quarter of the overlap in common: a sliver matches by chance.
- **An edge** is the measured offset of the second tile's corner from the first's, its energy,
  margin and correlation, the overlap, and whether it is trusted: an energy of at least 300 and a
  correlation of at least 0.5. A pair that doesn't register is an edge that isn't trusted.
- **Memory:** at most `threads` pairs are registered at once, on the calling rayon pool, each
  holding at most `bytes_per_pair`.
- **The pixels** come from a `TileSource`: a dataset's raw frames mapped read-only
  (`MappedTiles`), or images in memory.

## Place in the architecture

- **Layer:** L3 Processing.
- **Depends on:** `siq-register`, `siq-calib`, `siq-storage`, `siq-types`; `rayon`, `schemars`,
  `serde`, `thiserror`; `siq-datagen`, `serde_json` and `tempfile` in the tests and the example.
- **Used by:** the stitch job (P8.4).
- **Phase:** P8.1.

## API

| Item | What |
|---|---|
| `Mosaic::place(&frames, &l)` | The tiles of `(FrameKey, FrameMeta)`s placed by their poses through `l`, sensor pixels a µm: `Mosaic { tiles, px_per_um }` |
| `mosaic.overlaps(min_px, min_share)` | The pairs `(i, j)`, `i < j`, whose nominal rectangles overlap by `min_px` each way over `min_share` of a tile |
| `Tile` | `key`, `size`, `nominal` (its corner, in mosaic pixels), `sensor` (its first pixel on the sensor) and `binning` |
| `pairs::register(&source, &mosaic, &pairs, shading, &options, threads)` | Each pair's `Edge`, in order; `shading` the dark level and the flat, or `None` |
| `Edge` | `i`, `j`, `shift` (`j`'s corner from `i`'s, in mosaic pixels), `pce`, `margin`, `ncc`, `overlap_px`, `confident` |
| `PairOptions` | `min_overlap_px` (32), `min_overlap_share` (0.05), `stage_sigma_um` (5), `reach_sigmas` (4), `min_shared` (0.25), `min_pce` (300), `min_ncc` (0.5); `reach_px(px_per_um)` |
| `pairs::bytes_per_pair(&mosaic, &pairs)` | The most a pair's registration holds, from the largest overlap |
| `TileSource`, `MappedTiles::open(&reader, keep)` | Where pixels come from: a dataset's frames that `keep` takes, mapped, and their metadata; `[OwnedImage<u16>]` too |
| `StitchError` | No pose, binnings that differ, a tile that isn't there or isn't 16-bit, storage, image views |

## Examples

| Example | What it shows | Run |
|---|---|---|
| `pairs_on_dataset` | A `siq-datagen` scan of 5 × 4 tiles of 2048 × 1536 over cells, or a dataset of yours: its pairs registered, those not trusted, and the offsets against the truth and the stage's own error | `cargo run --release -p siq-stitch --example pairs_on_dataset [-- <dataset folder>]` |

On the 16-thread workstation, `pairs_on_dataset` registers the 31 pairs of its 20 tiles in 51 ms,
every one trusted, within 0.025 pixels RMS of the truth (0.054 at worst), where the stage was
22.7 pixels RMS off. Smoke mode (`SIQ_SMOKE=1`) makes 3 × 3 tiles of 512 × 384: their 57-pixel
strips over sparse cells leave 2 of the 12 pairs untrusted, and the rest within 0.21 pixels RMS.

## Features

None.

## Testing

`cargo test -p siq-stitch`:

- tiles placed by their poses through a turned calibration and binning; a grid's sides joined, its
  corners only below a lower share;
- a `siq-datagen` scan of 3 × 3 tiles over grains, the stage off by 5 µm: its 12 sides registered
  within 0.2 pixels RMS and 0.5 at worst of the truth (0.064 and 0.126 here), uncorrected and
  through a flat;
- an overlap of noise alone not trusted, and a textured one 3 and 2 pixels off its nominal place
  found within 0.05;
- `tests/memory.rs`: a counting allocator holds the registration within `threads` ×
  `bytes_per_pair`, one pair at a time and two.

## Limitations

- **Translations only,** by design (ADR-0013): a calibration's error shows as residuals.
- **Sparse overlaps aren't trusted:** cells far apart leave a side with nothing to register; the
  solver (P8.2) leans on the tile's other sides and its stage prior.
- **Corners aren't registered** by default: under 5 % of a tile, they add little.
