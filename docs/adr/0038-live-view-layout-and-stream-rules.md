# ADR-0038: The Live view's layout, and when its streams open and close

- **Status:** Accepted
- **Date:** 2026-10-24
- **Phase:** P3.3
- **Related:** ADR-0008, ADR-0010, ADR-0032, ADR-0037, REQ-PRV-001, REQ-PRV-002, REQ-PRV-003

## Context

- **The server sends two live streams** (ADR-0032):
  - the **overview**, the whole frame within 1920 × 1080;
  - the **ROI**, a region at 1:1 or 2:1, cut to 1920 × 1080 preview pixels about its middle.

  Both are mapped through `display.lut` on the server. The ROI is set in sensor pixels
  (`display.roi_region`).
- **A 100 MP overview is about 6 times coarser than the sensor,** so focusing needs the ROI at
  1:1.
- **A browser has 6 connections for the server.** The stream budget runs at most 3 MJPEG streams
  (ADR-0037).
- **`microscope`'s `LiveImage`** draws a stream to a canvas, contain-fit, with shared streams per
  URL and a frame-rate cap. ADR-0008 allows importing it.

## Decision

### The layout

- **The control pane** (wide, on the left, as on `microscope`'s pages) holds three sections:
  - **Camera:** start or stop live, snap (the image flashes, and the snap is shown below), the
    exposure, readout presets (whole sensor, the ROI only, 2 × 2 binning), and the rate, frames
    and frames lost (lost frames shown in red);
  - **Display:** automatic or manual contrast, the black, white and gamma sliders, the colours
    (grey, inverted, or the channel's colour), and the histogram;
  - **Focus:** the focus measure and a sparkline of its last 60 values (half a minute).
- **The view:**
  - the overview fills it, with the ROI outlined on it;
  - the ROI is beside it (split) or in its corner (picture-in-picture), as the user chooses,
    remembered in this browser;
  - a toolbar has the ROI's zoom, 1:1 or 2:1, and the layout.
- **The ROI is chosen by dragging a rectangle on the overview.**
  - The drag is mapped from the box to the frame (contain-fit), then to sensor pixels through
    the camera's region and binning.
  - A drag under 6 px is a click, which keeps the image's double-click for click-to-move (P3.4).
  - **The outline shows what the ROI stream shows:** the region asked, cut as `siq-preview` cuts
    it (`roiArea`, which reproduces `roi_region` and its test cases).
- **The histogram** has 256 bars over the pixel format's whole range, logarithmic by default.
  - Its two markers are the window's ends. Dragging one, or moving it with the arrow keys (1/256
    of the range, or 1/16 with Shift), makes the mapping manual from the window shown.
  - With auto-contrast, that window is the histogram's own percentiles.
- **Settings are sent one at a time:** a slider being dragged sends its latest value when the
  last write completes, not one per step.

### When streams open and close

- **A live image streams only while all of these hold:**
  - the app is connected;
  - the camera's TD has the stream;
  - its view is in the page;
  - the page is visible;
  - the image is in view;
  - the stream budget gives it a place.

  It closes as soon as any of them stops holding.
- **On the Live page,** the overview and the ROI take 2 of the 3 places. Leaving the page closes
  both: the budget releases them when their components unmount.
- **An image waiting for a place says so** ("Paused: other live images are open") and starts by
  itself.
- **Viewers of one URL share one request** (`watchStream`, from `microscope`), which stays open
  1 s after its last viewer, for a quick return. Each viewer still takes a place in the budget,
  which errs towards fewer connections.
- **Turning the live image off** is kept in this browser, and closes every stream.

## Alternatives considered

- **The ROI as a second page:** the overview and the ROI are used together, to find and then
  focus.
- **Drawing the ROI as asked, uncut:** the outline would not match the ROI view when the region is
  larger than 1920 × 1080.
- **A budget place per URL:** fewer places used when two views show the same stream, but the
  budget would have to know about stream sharing. It doesn't arise on the Live page.
- **Client-side display mapping:** rejected in ADR-0032; the server maps for every client.

## Consequences

- One Live page serves focusing (ROI at 1:1, focus sparkline) and finding (overview, presets).
- The ROI preview's 1920 × 1080 limit is a constant in the client (`ROI_MAX`), matching
  `siq-preview`'s default. A server configured otherwise would need it from the server.
- P3.4 adds the stage, focus and peripherals to the same page, and uses the image's double-click.

## Verification

- `web/tests/unit/live/geometry.spec.ts`: the box, frame and sensor mapping, and `roiArea` on
  `siq-preview`'s cases.
- `web/tests/unit/live/histogram.spec.ts`: the range, the bars, the window shown, moving an end,
  and the markers' keys.
- `web/tests/unit/live/LiveImage.spec.ts` (copied, adapted): drawing, errors, pausing out of view
  or hidden, the double-click, the flash, the preference, and the fourth image waiting.
- `web/tests/unit/live/LivePage.spec.ts`: the layout and its switch, the ROI from a drag, the
  zoom, the histogram's marker.
- **By hand,** in Edge against the simulator:
  - live at 9.0 frames/s with nothing lost;
  - the ROI dragged;
  - picture-in-picture;
  - both themes.
