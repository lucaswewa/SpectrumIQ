# ADR-0025: CPU kernel strategy

- **Status:** Accepted
- **Date:** 2026-10-24
- **Phase:** P1.5
- **Related:** ADR-0005, ADR-0021, REQ-PRV-003, REQ-CAL-002, REQ-NFR-003

## Context

- **The frame path runs several consumers per 100 MP frame,** at 9 frames/s (architecture §4.2):
  - the overview: area average, LUT and JPEG, ≤ 35 ms;
  - the ROI: ≤ 15 ms;
  - the statistics: ≤ 10 ms;
  - live stitching: 2 × 2 binning and flat field, ≤ 60 ms.
- **Memory bandwidth is the shared limit:** one pass over a 200 MB frame costs 5–10 ms.
- **The frame path must not allocate** in steady state (ADR-0005, REQ-NFR-003).
- **The simulator** (P1.6), **calibration** (P5.3, P5.4) and **storage** (M4) need the same
  kernels, on `u16` and `f32` images.

## Decision

### Caller-owned buffers

- **Every kernel writes into an `ImageViewMut` its caller owns,** and returns a `KernelError`
  if a size or parameter is wrong.
- **No kernel allocates.** The only allocation is `Lut::new`'s 64 KiB table, made once per
  display setting.
- **Scratch space is the caller's too:** the blur takes a scratch image. Small per-band state
  lives on the stack:
  - the column plan of the area average;
  - the row sums of binning;
  - the local histogram.

### Rayon row bands

- **A kernel splits its output into bands of about 128 Ki pixels.** It halves them recursively
  with `rayon::join`, through `ImageViewMut::split_at_row` (added to `siq-types`).
- **`join` puts its jobs on the stack.** Called from a pool thread, as the frame path's
  `LatestOnly` consumers are, a kernel allocates nothing; a counting-allocator test checks that
  on every thread.
- **Called from outside a pool,** rayon queues the call, and its injection queue allocates a
  block every few dozen calls.
- **Kernels run on whichever pool calls them:** the frame-path pool of half the cores, or the
  jobs pool.
- **Reductions combine in a fixed order:** the histogram and the focus measures. The bands
  depend only on the image's size, so a result doesn't depend on the scheduling.

### Plain loops first

- **The kernels are slice loops, `zip`s and `windows`,** which the compiler vectorizes.
- **Explicit SIMD waits for a measured need.** The measurements below meet the budget without it.

### The kernels

| Kernel | Types | Notes |
|---|---|---|
| `correct`, `correct_in_place` | `u16` | `(raw − dark) × gain`, rounded, clamped to the format's maximum; dark and gain optional |
| `downsample_area` | `u8`, `u16`, `f32` | Any smaller size, the edge pixels of each area weighted by the share they cover |
| `bin` | `u16` | Any factors (`siq_types::Binning`), sum (saturating) or mean |
| `copy` | any `Copy` | A crop is a copy of a `sub_view` |
| `gaussian_blur` | `f32` | Separable, cut at 3σ, edges held. σ ≤ 170 px; to blur more, blur a downsampled image |
| `upsample_bilinear` | `u8`, `u16`, `f32` | Between pixel centres, edges held |
| `histogram`, `Histogram` | `u16` | 4096 bins over the format's range, on a strided subsample; `percentile`, `auto_window` |
| `Lut` | `u16` → `u8` or `[u8; 3]` | Window, gamma (`t^(1/γ)`) and a colormap: grey, inverted, or a channel colour |
| `focus` | `u8`, `u16`, `f32` | Variance of the Laplacian, Tenengrad (mean squared Sobel gradient), normalized variance |

### Brief deviations

1. **No criterion benches.** The `kernels_bench` example measures what the budget needs, and
   conventions §5.3 allows either. criterion would bring a large dev-dependency tree.
2. **Binning takes any factors,** as `siq_types::Binning`, not only powers of two.
3. **The kernels have only the caller-provided-buffer form.** No kernel allocates its output.
4. **Two helpers were added to `siq-types`'s `ImageViewMut`:** `split_at_row` and `reborrow`.

## Measurements

`kernels_bench`, on a 11 648 × 8742 (102 MP) frame, with 8 threads (half the development PC's),
best of 5:

| Work | Time | Read | Budget |
|---|---|---|---|
| Flat field (dark and gain) | 36.4 ms | 22.4 GB/s | — |
| Overview: area average to 1439 × 1080, plus LUT | 19.3 ms | 10.5 GB/s | ≤ 35 ms with JPEG |
| ROI: crop 1920 × 1080, plus colour LUT | 0.24 ms | — | ≤ 15 ms with JPEG |
| Statistics: histogram at step 4, plus Tenengrad on the ROI | 2.8 ms | — | ≤ 10 ms |
| 2 × 2 binning | 16.0 ms | 12.7 GB/s | ≤ 60 ms with the flat field |
| Blur σ 2, `f32` 5824 × 4371 | 13.2 ms | 15.4 GB/s | simulator |
| Bilinear 2× to 5824 × 4370 | 14.6 ms | — | simulator |

- **The first versions were slower.**
  - The area average took 28 ms; it now plans its columns once per band.
  - Tenengrad took 10.3 ms; it now walks row slices.
  - Binning took 24 ms; it now sums rows into a stack accumulator.
- **The flat field is memory-bound,** reading 8 bytes per pixel. Live stitching (P7) should bin
  first and correct the binned image with a binned gain map: a quarter of the data.

## Alternatives considered

- **Kernels that return new images:** an allocation per frame on the frame path.
- **`par_chunks_mut` over raw slices:** it needs the views' data, and views with a stride are the
  common case. Splitting views keeps the stride.
- **Explicit SIMD (`std::arch`, `wide`):** unsafe or a dependency, for gains not yet needed.
- **The GPU for previews:** the readback and its latency cost more than these passes. The GPU is
  for stitching and reconstruction (M7, M8, M11–M13).

## Consequences

- **Callers own and reuse their output buffers.** P2.2 keeps one set per preview stream.
- **Kernels must be called from a pool thread** to be allocation-free.
- **The overview leaves about 15 ms for JPEG within its budget.** P2.2 measures the encoder, and
  may merge the overview and statistics passes into one read of the frame (architecture §4.2).

## Verification

- `siq-imgproc` tests:
  - every kernel against a naive reference, as proptests on small images with padded rows;
  - one large image split into several bands;
  - single pixels, saturation, and refused arguments;
  - `the_kernels_allocate_nothing_on_the_pool`: a counting global allocator over every thread,
    20 runs of every kernel.
- `siq-types`: `split_rows_cover_the_image_once_even_without_trailing_padding`.
- The `kernels_bench` example.
