# ADR-0059: The live canvas: format, paging, blending, and painting policy

- **Status:** Accepted
- **Date:** 2026-10-27
- **Phase:** P7.2
- **Related:** ADR-0049, ADR-0050, ADR-0058, REQ-STI-001, REQ-STI-003

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
  - the alpha ramps from 0 at the frame's edge to 1 a feather in (64 frame pixels by default);
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
