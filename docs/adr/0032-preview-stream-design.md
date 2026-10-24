# ADR-0032: Preview stream design: two streams, server-side display mapping, latest-only, and rate caps

- **Status:** Accepted
- **Date:** 2026-10-24
- **Phase:** P2.2
- **Related:** ADR-0010, ADR-0022, ADR-0025, ADR-0028, ADR-0031, REQ-PRV-001, REQ-PRV-002,
  REQ-PRV-003, REQ-PRV-004

## Context

- **A 100 MP frame is 200 MB, at 9 frames a second.** Clients, local or across the LAN, see
  1080p at most (ADR-0010).
- **Users need two views:**
  - the whole field, to navigate;
  - a region at the sensor's resolution, to focus and to judge detail.
- **Raw 14-bit data must not cross the UI or network boundary in live operation** (REQ-PRV-004).
  So the display mapping (window, gamma, colormap and auto-contrast) happens on the server.
- **The frame budget (architecture §4.2):**
  - the overview, ≤ 35 ms;
  - the ROI, ≤ 15 ms;
  - the statistics, ≤ 10 ms;
  - all on half the cores, beside the camera, the recorder and live stitching.
- **A preview must never slow the camera.** The fan-out's latest-only mailboxes keep only the
  newest frame (ADR-0022).

## Decision

### Three streams, each a latest-only consumer

| Stream | What | Default largest size | Default rate cap |
|---|---|---|---|
| Overview | The whole frame, area-averaged to fit, aspect kept | 1920 × 1080 | 10 frames/s |
| ROI | A region in sensor pixels, at 1:1 or 2:1 | 1920 × 1080 | 10 frames/s |
| Statistics | The ROI's histogram (256 bins), its 0.1–99.9 % window, and its Tenengrad focus measure | — | 5 frames/s |

- **Each stream has its own thread,** reading its subscriber. Its kernels run on one shared
  frame-path rayon pool, half the cores by default.
- **The rate cap:** a stream takes no frame before its turn. The mailbox keeps the newest frame
  meanwhile, so a capped stream always shows the latest.
- **A slow stream skips frames.** It never holds a slot for more than one frame, and never holds
  up the camera.

### The ROI

- **Its region is in sensor pixels,** so it doesn't move when the camera's region or binning
  changes. Each frame maps it through its ROI offset and binning.
- **Clamping:** the region is clamped to the frame. If nothing is asked, or nothing of it is in
  the frame, it shows the frame's middle.
- **Size:** it is cut, about its middle, to the largest size times the zoom.
- **Alignment:** its origin and size are whole multiples of the zoom, so 2:1 averages whole pairs.

### Display mapping, on the server

- **`DisplaySettings`** is held in a watch, and every stream reads it for each frame:
  - `Manual { min, max, gamma }`, or `Auto { low_pct, high_pct }` at gamma 1;
  - a `Colormap`: grey, inverted, or black to a channel's colour.
- **Auto-contrast takes its window from the whole frame,** a strided histogram of about a million
  samples. So both views of a frame are mapped alike.
- **The 64 Ki LUT is rebuilt only when the mapping changes** (P1.5's kernels). Each pipeline keeps
  its buffers between frames, so a steady stream allocates only its JPEGs.

### Encoding and sinks

- **JPEG, with `jpeg-encoder`,** at quality 85 by default:
  - grey for grey and inverted maps, RGB for a colour map;
  - its `simd` feature on;
  - its IJG licence needs the attribution in the README (ADR-0028).
- **Sinks:**
  - `PreviewSink::push_jpeg(stream, bytes, meta)`, where the bytes are a `Vec<u8>`. teta-wot's
    `MjpegStream::add_frame` takes them without a copy (P2.3);
  - `StatsSink::push_stats(stats)`.
- **Only JPEGs and statistics leave the engine.**

### Timing

Two histograms per JPEG stream, in the registry:

- `preview.<stream>.latency_ms`: from the frame's mid-exposure to its JPEG, as the brief asks.
  This includes the camera's readout.
- `preview.<stream>.work_ms`: from taking the frame to its JPEG. This is the pipeline's own cost,
  which the 35 ms and 15 ms budgets are about.

## Alternatives considered

- **One stream, the client cropping:** the client would need the full resolution to zoom, which
  is raw data's size.
- **Client-side display mapping on 16-bit tiles:** this breaks REQ-PRV-004, and puts 14-bit
  bandwidth on the LAN. P14.x's WebGL2 16-bit path is a later, local option.
- **A bounded queue per stream:** older frames would be shown late. Latest-only is right for live
  view.
- **A separate auto-contrast per view:** the ROI would brighten its own region, and the two views
  would disagree about the same frame.
- **H.264 now:** that is P14.3, with hardware encoding. MJPEG is simple, and seekless clients are
  fine with it.

## Consequences

- **Previews cost the camera nothing:** at worst they skip frames.
- **Each stream computes the frame's histogram for auto-contrast:** about 1 ms each at 100 MP.
  Sharing one per frame is a possible saving.
- **The budgets are met by the overview and missed by the ROI** in the simulated measurement,
  below. The camera simulator competes for the same cores. P10.x measures on the target PC, with
  the real camera.

## Verification

- **`siq-preview` `preview` tests:**
  - `the_overview_fits_its_box_and_keeps_the_frame_s_aspect`, with the JPEG's own size;
  - `a_roi_maps_through_the_frame_s_region_and_binning_and_stays_inside_it`;
  - `any_roi_is_inside_the_frame_aligned_to_its_zoom_and_within_the_largest_size` (proptest);
  - `a_lut_maps_the_window_through_its_gamma_and_colormap`;
  - `auto_contrast_takes_the_frame_s_percentiles`;
  - `statistics_count_the_region_and_measure_its_sharpness`;
  - `running_streams_keep_to_their_rate_caps_and_record_their_latency`, with the simulated
    camera.
- **The `preview_to_files` example,** at 100 MP in release:
  - the overview's work, 34–35 ms;
  - the ROI's at 2:1, 24–31 ms;
  - from mid-exposure, about 310 ms, beside the simulator's own rendering.
