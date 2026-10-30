# siq-imgproc

SpectrumIQ's CPU image kernels, for the frame path, the simulator, calibration and storage:

- flat-field correction;
- resampling;
- blur;
- histograms;
- display mapping;
- focus measures;
- a colour filter's mosaic shown in colour (ADR-0071).

Every kernel writes into a buffer its caller owns. It splits the rows into bands on rayon's pool,
and allocates nothing when called from a pool thread.

The design is in [ADR-0025](../../docs/adr/0025-cpu-kernel-strategy.md).

## Place in the architecture

- **Layer:** L1 Core.
- **Depends on:** `siq-types` (image views, `Binning`), `rayon`, `thiserror`.
- **Used by:**
  - P1.6's image formation;
  - P2.2's previews and statistics;
  - live stitching (P7);
  - flat-field calibration (P5.3, P5.4);
  - pyramids and storage (M4).
- **Phase:** P1.5; P10.6 added the colour filter's kernels.

## API

| Kernel | What |
|---|---|
| `correct(raw, &Calibration { dark, gain }, max, out)`, `correct_in_place` | `(raw − dark) × gain`, rounded and clamped to `max` |
| `downsample_area(input, out)` | Shrinks to `out`'s size, by area averaging |
| `bin(input, Binning, BinMode::{Sum, Mean}, out)` | Combines blocks: a saturating sum, or a rounded mean |
| `copy(input, out)` | Copies; a crop is `copy(&input.sub_view(rect)?, out)` |
| `gaussian_blur(input, sigma, scratch, out)` | Separable, `f32`, σ up to `MAX_SIGMA` (170 px) |
| `upsample_bilinear(input, out)` | Enlarges between pixel centres |
| `histogram(input, step, &mut Histogram)` | 4096 bins, every `step`-th pixel and row. `Histogram::percentile`, `auto_window(low, high)` |
| `Lut::new(&DisplayMapping { min, max, gamma, colormap })` | 64 Ki levels; `apply` to `u8`, `apply_rgb` to `[u8; 3]`. `Colormap::{Grey, Inverted, Colour(rgb)}` |
| `focus(input, FocusMeasure::{LaplacianVariance, Tenengrad, NormalizedVariance})` | Larger is sharper |
| `demosaic(mosaic, cfa, out)` | A Bayer mosaic, `cfa` at its top left, in red, green and blue (`[u16; 3]`): at its size by bilinear interpolation, each pixel's other colours the mean of its 3 × 3 neighbours of each, mirrored at the edges; smaller, each pixel each colour's mean over the whole cells it covers |
| `histogram_cfa(mosaic, cfa, step, &mut [Histogram; 3])` | Red's, green's and blue's histograms, of every `step`-th cell |
| `white_gains(&[Histogram; 3], fraction)` | The gains that bring red's and blue's `fraction` points to green's |
| `balanced(&DisplayMapping, gains)` | The mapping on each colour: the window over each gain. The colormap shows as grey, or inverted |
| `map_colours(&[Lut; 3], rgb, out)` | Each colour through its own table, to `[u8; 3]` |
| `for_each_row(out, work)` | Runs `work(y, row)` on every row in the kernels' bands: for other crates' per-pixel passes (P1.6) |
| `Sample` | `u8`, `u16`, `f32`: what the resampling kernels and `focus` take |
| `KernelError` | `Size { what, expected, found }` or `Parameter` |

```rust
pool.install(|| -> Result<(), KernelError> {
    correct_in_place(&mut frame, &Calibration { dark: Some(dark), gain: Some(gain) }, 16383)?;
    downsample_area(&frame.as_view(), &mut overview.view_mut())?;
    histogram(&frame.as_view(), 4, &mut counts)?;
    let (min, max) = counts.auto_window(0.001, 0.999);
    Lut::new(&DisplayMapping { min, max, gamma: 1.0, colormap: Colormap::Grey })?
        .apply(&overview.view(), &mut shown.view_mut())
})?;
```

## Examples

| Example | What it shows | Run |
|---|---|---|
| `kernels_bench` | Times every kernel on a 102 MP frame, with half the cores, against the per-frame budget: overview 19 ms, statistics 2.8 ms, 2 × 2 binning 16 ms on the development PC. A colour frame's overview, from whole cells, takes 6.7 ms, and its 1920 × 1080 ROI, interpolated, 4.8 ms on the instrument's PC (P10.6). Smoke mode uses 1 MP | `cargo run --release -p siq-imgproc --example kernels_bench` |

## Features

None.

## Testing

`cargo test -p siq-imgproc` runs three binaries:

| Binary | What |
|---|---|
| `bayer` | Demosaicing at size and shrunk against naive references, on every pattern; linear colours exact inside a large image; a single colour at any size; each colour's histogram, the white gains, and the mapping on each colour |
| `kernels` | Every kernel against a naive reference, as proptests with padded rows; one large image in several bands; single pixels, saturation and refusals |
| `no_alloc` | A counting global allocator over every thread sees no allocation in 20 runs of every kernel on a rayon pool. It is the test files' one `unsafe` exception, needed for an allocator |

## Limitations

- **Allocation-free only on a pool thread.** From outside one, rayon's injection queue allocates
  now and then.
- **The blur stops at σ = 170 px.** Blur a downsampled image for more.
- **Demosaicing is bilinear:** exact on linear colour, but it softens edges, and may fringe them
  in colour at 1:1.
- **`upsample_bilinear` aliases when shrinking.** Use `downsample_area` to shrink.
- **Binning sums saturate at `u16::MAX`:** a 4 × 4 sum of 14-bit pixels can.
