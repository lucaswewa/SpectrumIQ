# siq-stitch-live

The live mosaic's canvas (ADR-0059): while the stage moves, each frame painted where its
calibration and pose put it, on a sparse canvas of GPU pages, feathered at its edges, within a VRAM
budget.

- **The canvas is the stage's plane,** x right and y down as the stage's, at `canvas_scale` canvas
  pixels a sensor pixel (½ by default): stage µm and canvas pixels differ by a scale and an origin
  (`CanvasGeometry`). A frame's affine, frame pixels to canvas pixels, is its binning, the
  calibration's inverse, its pose and that scale (`frame_to_canvas`): the camera's rotation is in
  it.
- **Pages** of 4096² canvas pixels are made where a frame first falls. A pixel is a 32-bit word,
  its value in the low half and its coverage in the high half: 64 MB a page.
- **Painting:** for each page the frame's footprint meets, a dispatch over that rectangle. Each
  pixel is mapped back into the frame, sampled bilinearly, and laid over the page with an alpha
  that ramps from 0 at the frame's edge to 1 a feather in (64 frame pixels by default), weighted
  by the coverage there: the newest frame wins inside its feather band, and its edge never darkens
  what it covers alone.
- **The VRAM budget** (2 GB, 32 pages, by default): past it, the least recently used page is read
  back and kept in host RAM shrunk 4 × 4, its values weighted by their coverage. A page painted
  again is grown back onto the GPU.
- **The painting policy,** `paints`: a frame settled, or moving with its pose's uncertainty under
  the threshold.
- **Readback:** any rectangle of the canvas, from the GPU, or at its shrunk detail from host RAM.

## Place in the architecture

- **Layer:** L3 Processing.
- **Depends on:** `siq-gpu`, `siq-types`; `bytemuck`, `schemars`, `serde`, `thiserror`; `naga`,
  `png`, `siq-synth` and `tempfile` in the tests and the example.
- **Used by:** the `mosaic_live` Thing and its viewport (P7.3).
- **Phase:** P7.2.

## API

| Item | What |
|---|---|
| `Canvas::new(options)` | An empty canvas; its methods run on the GPU's thread, in `siq-gpu` jobs |
| `canvas.paint(&mut gpu, frame, &m)` | `frame` painted through `m` (frame pixels to canvas pixels) → `Painted { pages, made, restored, evicted }` |
| `canvas.region(&mut gpu, origin, size)` | The canvas's words of a rectangle, in canvas pixels: 0 where nothing was painted |
| `canvas.clear()`, `on_gpu()`, `on_host()`, `bounds()` | Every page gone; the pages on the GPU and in host RAM; the box of everything painted |
| `CanvasOptions { budget_bytes, feather_px, shrink }` | 2 GB, 64 frame pixels, 4 |
| `CanvasGeometry { origin, px_per_um }` | `for_calibration(&l, canvas_scale)`, `to_canvas`, `to_stage` |
| `frame_to_canvas(&l, size, binning, pose, &geometry)` | A frame's affine onto the canvas, its centre where the stage is |
| `paints(&quality, uncertainty)` | Whether a frame of `quality` paints |
| `paint::Frame { samples, width, height }`, `footprint(&m)` | A frame to paint, binned and flat-corrected, and its box on the canvas |
| `paint::Params::for_page(&frame, &m, origin, side, feather)`, `paint::cpu(&mut page, &samples, &params)` | One page's paint, and the CPU reference the kernel is held to; `paint::SHADER` |
| `PAGE` | 4096 |

## Examples

| Example | What it shows | Run |
|---|---|---|
| `paint_sim` | A 6 × 6 grid of synthetic 1024 × 768 frames of grains at 10 % overlap, the camera turned 0.4°, each reported 0.5 µm off, painted across a page corner, and the painted region written as a viewport PNG (2 × 2 of 256 × 192 in smoke mode) | `cargo run --release -p siq-stitch-live --example paint_sim [-- <out.png>]` |

On the RTX 4060, `paint_sim` paints a frame in 1.4 ms, its upload included.

## Features

None.

## Testing

`cargo test -p siq-stitch-live`, on this machine's GPU, each GPU test skipped with its reason where
there is none:

- the WGSL validated by naga; the geometry and the policy, which need no GPU;
- two overlapping frames painted as the CPU reference paints them, within 1 in value and coverage;
- a spot where a known affine puts it, within 0.05 canvas pixels (0.0001 here);
- a frame across the corner of four pages, as one;
- a budget of two pages: the third evicts the first, which reads back from host RAM, and comes
  back when painted again; clearing.

## Limitations

- **The frame comes prepared:** binned and flat-corrected by the caller (P7.3).
- **A page's paint waits for the GPU,** one submission a page.
- **An evicted page loses detail:** 4 × 4 in host RAM, and grown back so.
- **No CPU canvas:** without a GPU there is no live mosaic; `paint::cpu` paints one page, for the
  tests.
