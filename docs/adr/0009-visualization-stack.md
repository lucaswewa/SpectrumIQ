# ADR-0009: Visualization: OpenSeadragon first, WebGL2 16-bit later; VTK.js; optional native viewer

- **Status:** Accepted
- **Date:** 2026-10-23
- **Phase:** planning
- **Related:** REQ-VIS-001–004

## Context

The goals mention OpenSeadragon, a custom WebGPU tile renderer, VTK.js, Three.js, Babylon.js, and a
native winit + wgpu window. The data is 14-bit, which needs window/level control. Browsers
(WebView2, Chrome, Edge) support WebGL2 everywhere. WebGPU support varies.

## Decision

- **2D:** OpenSeadragon with server-rendered 8-bit tiles that carry the current LUT (P4.5, P4.8).
  Multi-channel images are layered with additive compositing.
- **Later 2D:** a WebGL2 16-bit tile renderer (`R16UI` textures, shader LUT) behind the same tile
  API (P14.1).
- **3D:** VTK.js only, in both the desktop app and the browser: height maps, volumes, slices,
  client-side isosurfaces, and point clouds.
- **No Three.js or Babylon.js.**
- **Optional native viewer:** a desktop-only winit + wgpu viewer for very large volumes, in a late
  milestone (P14.6–P14.7).

## Alternatives considered

- **A custom 16-bit renderer from the start:** about 2 more phases up front.
- **Three.js or Babylon.js:** general-purpose engines with weaker scientific volume rendering.

## Consequences

- Contrast changes in v1 refetch tiles, which are cached per LUT.
- WebGPU is not required in the client.

## Verification

P4.8: OpenSeadragon multi-channel compositing. P11.4 and P12.5: VTK.js views mount and unmount
without leaks.

## Amendment (P4.8, 2026-10-25): the 2D viewer, and multi-channel compositing

**The 2D viewer** (`web/src/viewer2d/`) is OpenSeadragon 6.1.1. It is loaded when the viewer
first opens, and destroyed when the viewer unmounts. It is adapted from `microscope`'s deep-zoom
view, and serves the Viewer page (`/#/viewer?dataset=<id>&image=<name>`).

**Multi-channel compositing:**
- **One tiled image per channel,** each the plane's DZI descriptor (ADR-0045), all in one place,
  drawn with the canvas's `lighter` operation: the channels' colours add, as in a fluorescence
  composite.
- **A channel's mapping is in its descriptor's query** (`min`, `max`, `gamma`, `cmap`), which
  OpenSeadragon repeats on every tile. The server renders each channel's tiles black to its colour.
- **A hidden channel** is drawn at opacity 0, and its tiles are kept.
- **A changed mapping or plane is a new URL.** Its tiled image is loaded hidden beside the one
  shown, which is removed once the new one is fully loaded. The view never blanks, and keeps its
  zoom and pan.

**The controls:**
- **Per channel:** shown or hidden, the colour, the window's ends, and the gamma, from 0.2 to 5.
  Reset gives back the channel's own window and colour, from the image's `omero` metadata.
- **The LUT store** keeps what the controls show, and applies it to the tiles once they rest for
  250 ms. A slider being dragged fetches tiles only where it stops; showing or hiding is at once.
  Queries are rounded to six significant digits, so near-equal mappings share their cached tiles.
- **The z and t sliders** choose the plane, applied once they rest, likewise.
- **The scale bar** is the longest round length (1, 2 or 5 × 10ⁿ µm) within 120 screen pixels. It
  comes from the image's pixel size and OpenSeadragon's zoom.
- **The probe:**
  - shows the true values of the visible channels under the pointer, from level 0, through
    `…/value`;
  - asks once the pointer rests for 120 ms;
  - cancels the request for a position the pointer has left.
- **The image's metadata** comes from a new route, `GET /api/siq/data/{dataset}/{image}/info`:
  - the size, sample type, levels, pixel size and origin;
  - the channels: label, colour and window.

**Overlay layers**, `microscope`'s, are imported (`web/src/overlay/`). They are shapes in the
image's pixels, drawn in an SVG over the image, placed again whenever the view moves, and chosen
in the Layers menu. The viewer takes them as a prop; M8 fills them (tile outlines, seams).
`/#/dev/viewer2d` shows a demo layer.

**Verification:**
- `web/tests/unit/viewer2d/`:
  - the tile URL builder, against OpenSeadragon's DZI tile paths;
  - the LUT store: rest, rounding, valid mappings, Reset;
  - the probe's debounce and cancellation;
  - the scale bar;
  - the deep-zoom view on a mocked OpenSeadragon: a `lighter` image per layer, swapped only once
    loaded, a stale one dropped, the pointer's pixel and the zoom, and the viewer destroyed on
    unmount.
- **In Edge** (Playwright), on a 12 000 × 8000 two-channel synthetic dataset:
  - the tiles were asked for with each channel's mapping;
  - the composite was drawn;
  - the probe read 16000 and 12000 where the pattern puts them;
  - moving a black point fetched 33 new tiles with it;
  - the scale bar went from 500 to 200 µm on zooming in;
  - no console errors.
