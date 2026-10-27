# ADR-0060: Mosaic navigation UX

- **Status:** Accepted
- **Date:** 2026-10-27
- **Phase:** P7.4
- **Related:** ADR-0037, ADR-0039, ADR-0057, ADR-0059, REQ-STI-001, REQ-WF-001

## Context

- **The live mosaic** (ADR-0059) is painted on the server as the stage moves, and the server
  renders one viewport of it into a stream: a stage point, a zoom, and a size up to 1920 × 1080.
- **It is a navigation map:** the user looks around what was imaged, goes back to a place, and
  plans a scan over what they have seen. The Scan page's stage map (ADR-0057) plans in stage µm.
- **The image lags the view:** a viewport written takes a round trip and a render, up to 100 ms
  between renders, before its image arrives. Panning by the image alone would stutter.
- **The stream budget** (ADR-0037) allows three MJPEG streams at once.

## Decision

- **The Mosaic page** is the mosaic filling the main view, beside a narrow pane:
  - **Paint**, a switch for `mosaic_live.enabled`, and a status line: not painting, waiting for a
    frame, or the frames painted and the pages' GPU memory against its budget;
  - **Show it all** fits what was painted; **Go to the stage** centres the stage;
  - **Clear**, once confirmed ("What was painted is gone"), and **Save**, a dataset, told in a
    toast.
- **Navigation is the stage map's:** the same view (`scan/geometry`: a stage point, map pixels a
  µm, a size), dragged to pan and zoomed with the wheel about the pointer. The page first shows
  what was painted, or else the stage.
- **The view is the viewport:** a view becomes a viewport of its centre and size, the zoom its
  scale over the canvas's `px_per_um`, shrunk to fit 1920 × 1080 and stretched back over the view.
  It is written at most every 100 ms, the renders' interval: the first change at once, the last
  always.
- **An image is placed where its viewport was:** each image is drawn at the stage box of the
  viewport the server last took, in the view as it is now. While the view moves, the last image
  moves with it, and the next replaces it in place: no stutter, and no jump when it arrives.
- **Overlays in stage µm,** in an SVG over the image: the camera's field (`workflow.field`) where
  the stage is, or a dot without a calibration; and a minimap in the corner of what was painted
  and the view within it, which a click recentres.
- **A double-click moves the stage there,** within its travel: the field then centres on the
  point, as the Live page's double-click does (ADR-0039).
- **The stage map** of the Scan page draws over the mosaic once something was painted: its view
  writes the viewport, and its travel and shapes stay on top. Unpainted stage is black, as the
  server renders it.

## Alternatives considered

- **Panning by asking only, the image redrawn as it comes:** simple, but the image stutters at the
  render rate and lags the pointer.
- **Each image carrying its viewport,** in a JPEG comment, read by the client: exact, where the
  last viewport taken is wrong for at most the one image rendered as it changed; but server and
  stream code for a frame's misplacement.
- **A viewport per client:** two clients, or the Mosaic page and the stage map at once, would not
  fight over one; but the server would render each, and P7.4 has one client at a time. The last
  writer wins.
- **The mosaic as deep-zoom tiles in OpenSeadragon,** as the Viewer page: smooth zooming, but the
  canvas changes with every frame, and the tiles would have to follow it.

## Consequences

- **One viewport per server:** the page and the stage map write it as they show; two clients
  looking at once see each other's views.
- **Zoomed far out, the image aliases** (nearest sampling, ADR-0059, amended); the minimap and
  Show it all keep the whole in reach.
- **The stream takes one of the three places** while the page or a stage map over the mosaic is
  shown.
- **The stage map turns dark** where nothing was painted, once anything was.

## Verification

- `web/tests/unit/mosaic/viewport.spec.ts`: a view's viewport, at most 1920 × 1080 and within the
  zooms rendered; an image placed in a view panned and zoomed since; the throttle, once an
  interval with the last call always made.
- `web/tests/unit/mosaic/MosaicPage.spec.ts`: the painted area shown whole and the camera's field
  placed where the stage is; the viewport written once an interval as the view zooms, the last
  always; a double-click moving the stage there; Clear only once confirmed; Save; the stage map
  over the mosaic, its view the viewport.
- `/#/dev/mosaic`, in Edge: fields painted where the fake stage stood, panned and zoomed, the
  field's outline on its field.
