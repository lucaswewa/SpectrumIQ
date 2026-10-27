# ADR-0059: The live canvas: format, paging, blending, and painting policy

- **Status:** Accepted
- **Date:** 2026-10-27
- **Phase:** P7.2
- **Related:** ADR-0045, ADR-0049, ADR-0050, ADR-0058, REQ-STI-001, REQ-STI-003, REQ-PRV-001

## Context

- **The live mosaic** paints every frame where the calibration (ADR-0049) and the stage's pose put
  it, as the stage moves, on the GPU (ADR-0058). It is a navigation map, not the stitched result:
  M8 stitches by registration.
- **Placement is deterministic:** the frame's affine, from `sensor px = c + L·(S − P)`. Its error
  is the stage's, about half a µm (M7's criterion: within 2 canvas pixels at scale ½).
- **The area is large and sparse:** a slide is 75 × 25 mm, at 1.33 canvas pixels a µm 100 000 ×
  33 000 pixels; a session paints a small part of it. The GPU has 8 GB, shared.
- **Frames overlap:** neighbours 10 % or more, and the stage revisits places. Seams should not show,
  and what is newest should be what the map shows.
- **Moving frames** carry a pose interpolated between readings, with its uncertainty, or no pose.

## Decision

- **The canvas is the stage's plane,** x right and y down as the stage's, at `canvas_scale` canvas
  pixels a sensor pixel (½ by default). Stage µm and canvas pixels differ by a scale and an origin,
  so a viewport in µm is a rectangle of pixels. The camera's rotation is in each frame's affine:
  frame pixels from the frame's centre, times the binning, through `L⁻¹` onto the stage, plus the
  pose, then onto the canvas.
- **Pages of 4096² pixels,** made where a frame first falls, keyed by their place: the canvas has
  no bounds.
- **A pixel is a 32-bit word,** its value (u16) in the low half and its coverage (0 to 1, as u16)
  in the high half: 64 MB a page, in a storage buffer. A value alone couldn't tell unpainted from
  black; f32 pairs would double the memory.
- **Painting, per page:** for each page the frame's footprint meets, a dispatch over the
  rectangle of it covered. Each pixel's centre is mapped back into the frame (the inverse affine,
  composed with the page's corner in f64, then f32, so its numbers are a frame's), sampled
  bilinearly between the four pixel centres around it, and composited:
  - the alpha ramps from 0 at the frame's edge to 1 a feather in (64 frame pixels by default,
    at most an eighth of the frame's shorter side: P7.3);
  - the coverage becomes `a + c·(1 − a)`, and the value `(n·a + v·c·(1 − a)) / coverage`.

  So **the newest frame wins** wherever its alpha is 1, and blends with what was there only inside
  its feather band; over what nothing covered, its edge keeps its own value, not darkened.
- **The VRAM budget** (2 GB, 32 pages, by default): when a page is needed past it, the least
  recently used page not in this paint is read back and kept in host RAM, shrunk 4 × 4, each
  block's value weighted by its coverage: 4 MB a page. A shrunk page painted again is grown back,
  each of its pixels over its block, and painted at full detail from then on.
- **The painting policy:** a frame paints if it is settled, or moving with its pose's uncertainty
  under the threshold; a frame without a pose never does.
- **A CPU reference** paints one page with the kernel's arithmetic, in f32 in the same order: the
  tests hold the GPU to it.

## Alternatives considered

- **A canvas along the camera's axes:** frames would paint without rotation, but every viewport
  and every stage position would need the calibration to place, and changing the calibration
  would move the canvas.
- **`R32Float` pages with a separate weight:** twice the memory for the same pages. u16 holds the
  camera's 14 bits.
- **Averaging every frame that covers a pixel:** the map would show stale content after the
  specimen changes or the focus moves, and a moving frame's blur would stay mixed in.
- **One big buffer with a page table:** fewer bindings, but a fixed size, and allocating in it is
  ours to write. A buffer a page is what wgpu already allocates.
- **Textures, sampled by the hardware:** `R16Uint` can't be filtered, and the composite needs a read
  and a write of the same pixel, which storage buffers give simply.
- **Evicting to disk:** a live map's evicted pages are wanted back soon, and host RAM holds 4 MB
  each; disk is for M8's datasets.

## Consequences

- **The map shows the newest frame of each place,** feathered into its neighbours: seams are soft,
  not averaged.
- **Placement is the affine's:** a spot lands within 0.0001 canvas pixels of where a known affine
  puts it; the stage's error remains, by design.
- **32 pages on the GPU** cover 131 000 × 4 096 pixels' worth of area, a slide's width at scale ½;
  past it, evicted pages show at a quarter of the detail until painted again.
- **A frame's paint waits for the GPU,** a submission a page: 1.4 ms a 1024 × 768 frame on the RTX
  4060. A 100 MP frame comes binned (P7.3).

## Verification

- `crates/siq-stitch-live/tests/canvas.rs`: the paint against its CPU reference; a spot's
  placement; a frame across four pages; eviction to host RAM and back; the geometry; the policy;
  the WGSL.
- **`paint_sim`:** a 6 × 6 grid of synthetic frames, turned 0.4°, each 0.5 µm off, painted across a
  page corner into a coherent mosaic.

## Amendment (P7.3, 2026-10-27): the viewport, its stream, and the Thing

- **The client names what it looks at,** `mosaic_live.viewport`: the stage point at its centre in
  µm, a zoom (view pixels a canvas pixel, from 1/1024 to 64) and a size up to 1920 × 1080. The
  server renders that view and streams it as JPEGs, `mosaic_live/view`, so the bandwidth is the
  view's, whatever the mosaic's size.
- **The view kernel** draws a view from the pages it meets, a dispatch each over only the view's
  pixels the page may show: each view pixel takes the canvas pixel its centre falls on, through
  the display's LUT to a byte, times its coverage, so unpainted canvas is black and a mosaic's
  edge fades. Four pixels share a word, so the dispatches of neighbouring pages never write the
  same word at once. A page in host RAM is uploaded for the view, and drawn at its shrunk detail.
- **The window** is the display's (`display.lut`): fixed, or automatic from the newest frame
  painted, as the live view is.
- **A view is rendered** at most 10 times a second, and only when something changed: a frame
  painted, the viewport, a clear. While nothing is painting, a change of viewport renders at once.
- **Frames come prepared,** on the frame-path pool the previews share: binned by the whole factor
  nearest to the canvas's scale, then corrected, `(value − dark) × gain`, by the dark and flat in
  force. The frame's slot is free again before its paint starts.
- **The painting policy's threshold** is `max_uncertainty_px`, 2 canvas pixels by default: M7's
  placement criterion. A moving frame's uncertainty is how far the stage went between the readings
  either side, so at 5 readings a second a frame paints while moving only below about 7 µm/s at
  10×; above it, frames wait for the stage to stop. A client may raise it to paint while jogging.
- **The feather** is at most an eighth of the frame's shorter side, so a small frame isn't all
  edge.
- **`save_snapshot`** writes the painted box to a new dataset of kind `mosaic`: one OME-Zarr image
  at canvas scale, its origin and pixel size on the stage, a band of chunk rows at a time read
  back from the canvas; a background job builds its pyramid.
- **The GPU opens** when the mosaic is first enabled, by `[gpu]`: without one, enabling is refused
  (HTTP 422), saying why. `[gpu] mosaic_vram_gb` (2) is the pages' budget.
- **Alternatives considered:**
  - **The whole canvas sent as tiles to the client:** the deep-zoom viewer's way (ADR-0045), but
    the live canvas changes with every frame, and the client would hold it all.
  - **Bilinear or box-filtered sampling of the view:** smoother zoomed out, but a box over 16
    canvas pixels a view pixel costs 16 reads; the navigation map shows each place's newest frame
    and is refreshed as soon as the stage moves. Nearest, as the viewer zoomed in shows pixels.
  - **The LUT on the CPU:** a read-back of 32-bit words, four times the bytes, and the mapping at
    a frame's rate on the frame-path pool.
- **Verification:** `crates/siq-stitch-live/tests/view.rs` (a viewport on the canvas and its
  checks; a frame binned to the scale and flat-corrected; the view as the canvas's pixels through
  the LUT say, with no pixel off by more than 1; a spot at the centre of a view about it, within
  0.01 view pixels; an evicted page shown shrunk); `crates/siq-server/tests/mosaic.rs` (3 × 3
  fields painted, their bounds within 3 µm, the mosaic correlating 0.87 with the specimen where
  the stage stopped, best at no offset; a 1920 × 1080 view streamed while painting, no frame lost,
  the pages within the budget; the settings checked; refused without a GPU); `live_mosaic_sim`.
