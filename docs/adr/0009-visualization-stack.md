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
