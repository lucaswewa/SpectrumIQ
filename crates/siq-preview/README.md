# siq-preview

SpectrumIQ's live previews. The acquisition's frames become two JPEG streams and statistics:

- **Overview:** the whole frame, area-averaged to fit 1920 × 1080.
- **ROI:** a region in sensor pixels, at 1:1 or 2:1, clamped to the frame.
- **Statistics:** the ROI's histogram, its window and its focus measure.

A colour frame, raw Bayer samples (ADR-0071), is demosaiced: the overview from whole filter cells,
the ROI by interpolation at 1:1 and from whole cells at 2:1. Its window is green's, and red's and
blue's are the window over their white-balance gains, automatic from each frame's brightest or
fixed. Its statistics are per colour, and its focus measure is on luminance.

How it works:

- **Each stream is a latest-only consumer,** on its own thread, with its kernels on a shared
  frame-path rayon pool, and its own rate cap. A slow stream skips frames, and never holds up the
  camera.
- **Display mapping is the server's:** a fixed window and gamma, or auto-contrast from the frame's
  percentiles, with a grey, inverted or colour map. Only 8-bit JPEGs and statistics leave, never
  raw frames.
- **Two latency histograms per stream:** from mid-exposure, and of the pipeline's own work.
- **The flat-field correction, if turned on** (ADR-0050): the dark level and the flat in force,
  `(value − dark) × gain`, applied after shrinking, as the flat is smooth. The gains are kept
  between frames while the region, size and flat stay. The auto-contrast window still comes from
  the raw frame.

The design is in [ADR-0032](../../docs/adr/0032-preview-stream-design.md).

## Place in the architecture

- **Layer:** L4 Engines.
- **Depends on:**
  - `siq-acquire`, for the frames;
  - `siq-imgproc`, for the kernels;
  - `siq-calib`, for the flat;
  - `siq-buffer`, `siq-devices` (the watch), `siq-telemetry` (the histograms), `siq-types`;
  - `jpeg-encoder`, with `simd`. Its licence includes IJG, with the attribution in the README
    (ADR-0028);
  - `rayon`, `thiserror`, `tracing`.
- **Used by:** P2.3's server, whose camera Thing feeds its MJPEG streams from a `PreviewSink`, and
  whose display Thing sets the `DisplaySettings`.
- **Phase:** P2.2; P5.3 the flat-field correction; P7.3 the pool shared.

## API

| Item | What |
|---|---|
| `Previews::start(&acquisition, &config, preview_sink, stats_sink, clock, registry)` | Starts the three streams. Dropping it stops them |
| `set_display(DisplaySettings)`, `display()` | `Contrast::Manual { min, max, gamma }` or `Contrast::Auto { low_pct, high_pct }`, a `Colormap`, and a colour frame's `WhiteBalance::{Auto, Fixed { red, blue }}`. The default is auto from 0.1 to 99.9 %, in grey, with automatic white balance |
| `set_roi(RoiSettings)`, `roi()` | `RoiSettings { region: Option<RectPx> in sensor pixels, zoom: Zoom::Native or Zoom::Half }`. `None` shows the frame's middle |
| `set_flat(PreviewFlat)`, `flat()` | `PreviewFlat { enabled, shading: Option<Arc<Shading>> }`, a `Shading` being a dark level and a `FlatField`. Off by default |
| `pool()` | The frame-path pool, for other consumers of the frames to share: the live mosaic's binning |
| `PreviewConfig` | Largest sizes (1920 × 1080 each), JPEG quality (85), the rates `[overview, roi, stats]` (10, 10, 5), pool threads (half the cores) |
| `PreviewSink::push_jpeg(StreamKind, Vec<u8>, &FrameMeta)` | Where each JPEG goes |
| `StatsSink::push_stats(&FrameStats)` | `FrameStats { meta, region, histogram: [u32; 256], window, focus, colours }`: a colour frame's `histogram` and `window` are green's, `colours` red's, green's and blue's, and `focus` its luminance's |
| `Renderer`, `fit`, `roi_region`, `measure` | The pipelines' steps, for tests and other consumers. `Renderer::mapping(frame, bits, cfa, settings)` gives a `Mapping::{Mono, Colour}`; `Renderer::show(frame, region, size, mapping, shading)` takes the shading and the frame's metadata, or `None`. `measure(frame, meta, bits, region, &mut Measuring)` |

**Metrics,** with a registry:

- `preview.overview.latency_ms` and `preview.roi.latency_ms`: from the frame's mid-exposure to its
  JPEG;
- `preview.overview.work_ms` and `preview.roi.work_ms`: from taking the frame to its JPEG.

```rust
let previews = Previews::start(&acquisition, &PreviewConfig::default(), mjpeg_sink, stats_sink, clock, Some(&registry))?;
previews.set_roi(RoiSettings { region: Some(RectPx::new(5000, 4000, 1920, 1080)), zoom: Zoom::Native });
previews.set_display(DisplaySettings { contrast: Contrast::Manual { min: 500, max: 9000, gamma: 1.2 }, colormap: Colormap::Grey });
```

## Examples

| Example | What | Command |
|---|---|---|
| `preview_to_files` | 3 s of simulated live view from `config/sim.toml`, at 100 MP. It writes the last overview and 2:1 ROI JPEGs, and prints how many each stream made and their mean work and latency. A filter array, such as `gbrg`, makes the sensor a colour one. Smoke mode uses a 1024 × 768 sensor | `cargo run --release -p siq-preview --example preview_to_files [-- <folder> [<cfa>]]` |

## Features

None.

## Testing

`cargo test -p siq-preview` runs `preview`:

- the overview's size and aspect, checked in the JPEG itself;
- the ROI through the frame's region and binning, clamped, cut and aligned. A proptest covers any
  frame and region;
- the LUT, through its window, gamma and colormaps;
- auto-contrast percentiles;
- a frame lit by a known flat over a dark level shown flat once corrected, shrunk and at 1:1,
  over a binned region, and not without the correction;
- statistics: the histogram's counts, the window and the focus measure;
- a colour frame shown shrunk and at 1:1 from an odd corner, raw and white balanced, and its
  statistics per colour, with its sharpness on luminance;
- the running streams on the simulated camera: their rate caps, and the latency histogram.

## Limitations

- **16-bit formats only:** an 8-bit frame is skipped.
- **Each stream computes the frame's auto-contrast histogram,** about 1 ms each at 100 MP, and
  about 2 ms per colour frame.
- **Automatic white balance** shows each frame's brightest white: right for brightfield, wrong for
  a fluorescent colour, whose white balance should be fixed.
- **The ROI's 15 ms budget is missed in simulation** (24–31 ms at 2:1 of 1080p, beside the
  simulator). P10.x measures on the target PC.
