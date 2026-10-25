# siq-pyramid

Multiscale pyramids (ADR-0044): an OME-Zarr image's levels 1 and up, built from level 0 by 2 × 2
mean, out of core and in parallel. The DZI tile service (P4.5) follows in the same crate.

## Place in the architecture

- **Layer:** L3 Processing.
- **Depends on:** `siq-storage` (the OME-Zarr images), `siq-types`; `rayon`.
- **Used by:** `siq-jobs` (`kinds::PyramidJob`).
- **Phase:** P4.4.

## API

| Item | What |
|---|---|
| `build(image, top, control)` | Adds the levels `top` needs, and builds them from level 0, a chunk row at a time, its chunks in parallel on the calling rayon pool |
| `levels_for(width, height, top)` | Level 0 and each halving until both fit `top` |
| `estimate(info, threads)` | The RAM a build takes at its peak |
| `Control` | `progress(done)` and `cancelled()`; `()` is a control that does neither |
| `mean_2x2(view)` | A view halved, by the 2 × 2 mean, edges included |
| `Mean` | `u16` (rounded to the nearest) and `f32` |
| `TOP` | 256: the top level's largest size |

## Examples

| Example | What it shows | Run |
|---|---|---|
| `build_pyramid` | A synthetic 20 000 × 20 000 image's level 0 written, its 7 levels built, the time and the peak memory | `cargo run --release -p siq-pyramid --example build_pyramid` |

Every example also runs in smoke mode (`SIQ_SMOKE=1`) in `cargo xtask ci`.

## Features

None.

## Testing

`cargo test -p siq-pyramid`:

- every level against a naive pyramid, on odd sizes, in `u16` and `f32`;
- the level count, and the metadata as built;
- cancelling;
- the peak memory of a build, measured by a counting allocator (`tests/memory.rs`, the only test
  in its binary).

## Limitations

- One filter: the 2 × 2 mean.
- A cancelled build leaves its levels part-built; the next build redoes them.
