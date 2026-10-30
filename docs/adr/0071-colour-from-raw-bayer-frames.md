# ADR-0071: Colour from raw Bayer frames: the simulator, the previews, the pyramid and the tiles

- **Status:** Accepted
- **Date:** 2026-10-29
- **Phase:** P10.6
- **Related:** ADR-0026, ADR-0032, ADR-0044, ADR-0045, ADR-0070, REQ-ACQ-001, REQ-PRV-001,
  REQ-PRV-002, REQ-PRV-003, REQ-VIS-001

## Context

The instrument's camera, the Adimec D-103A12c, is a colour camera. On 2026-10-29 the owner decided
that its frames stay raw Bayer: one 12-bit sample a pixel, of the colour its filter passes,
recorded and stitched as they are, and demosaiced only for display (ADR-0070, M10's amendment).
`FrameMeta.cfa` names a frame's 2 × 2 filter cell at its top left.

Until now every consumer took a frame as mono. On a mosaic, that goes wrong:

- the previews' area average mixes the colours into grey, and at 1:1 the pattern shows as a fine
  checkerboard;
- the histogram mixes three distributions, and the window is no colour's;
- the pyramid's 2 × 2 mean takes one red, two greens and one blue, so every level above the first
  is grey: the viewer couldn't show colour at any zoom but the full one.

Registration, autofocus, calibration, the live mosaic and the stitch also take frames as mono.
They are P10.7's.

## Decision

- **The simulator** gets a colour sensor, so that colour is tested in CI:
  - `[camera] cfa` for `kind = "sim"`, mono if not given;
  - each pixel takes its colour's share of the light, the response of a warm lamp behind the
    filters, red at 75 % and blue at 55 % of green;
  - its capabilities give the filter array, and each frame's metadata gives it shifted by the
    region's odd offsets, as the Euresys camera's does;
  - binned, a pixel keeps its place in the pattern, as the Adimec's does (ADR-0070).
- **Demosaicing** (`siq-imgproc` `demosaic`) has two modes:
  - **at the mosaic's size,** bilinear: each pixel keeps its own colour, and takes for each other
    colour the mean of its neighbours of that colour in the 3 × 3 about it, mirrored at the
    edges. Linear colours come out exactly inside the image;
  - **shrunk,** each output pixel is each colour's mean over the whole cells it covers.
  - A region's pattern is the frame's, shifted by the region's corner (`Cfa::shifted`).
- **The previews:**
  - the overview demosaics from whole cells;
  - the ROI interpolates at 1:1, and takes whole cells at 2:1;
  - the flat-field correction applies its gain to each colour.
- **The display mapping on each colour:**
  - the window, manual or automatic, is green's;
  - red's and blue's windows are the window over their white-balance gains, so there is a table
    per colour, and no pass multiplies the pixels;
  - the colormap doesn't apply, because a colour frame shows its own colours, but inverted still
    inverts.
- **White balance** (`DisplaySettings.white_balance`):
  - **`auto`,** the default: for each frame, red's and blue's gains bring their 99.9th
    percentiles to green's, so the brightest shows white. For brightfield, that is the lamp;
  - **`fixed`:** red's and blue's gains, green's being 1. The `display` Thing's
    `white_from_roi` action sets them from the ROI's latest statistics, so the ROI on a blank of
    the slide makes it white;
  - the web app has a toggle, "Automatic white balance", and a button, "White from ROI".
- **The statistics:**
  - a colour frame's ROI is counted per colour (`FrameStats.colours`; over HTTP, the histogram's
    `colours`);
  - its `histogram` and `window` are green's, which the manual window is on;
  - its focus measure is taken on luminance, each cell's mean: a cell's colours aren't detail;
  - the web app's histogram draws red, green and blue as a line each.
- **The pyramid keeps the cells whole:**
  - `ImageInfo.cfa` names a colour image's pattern, kept in the group's `zarr.json` under
    `spectrumiq`, since NGFF has no word for it;
  - each pixel of level `l + 1` is the mean of its colour in the 2 × 2 cells below its cell, so
    every level is a mosaic of the same pattern;
  - a chunk of the level above is still the mean of exactly four chunks below, if chunks are even.
    A colour image's chunks must be even;
  - a snap's image takes its frames' filter array, and its channels' windows are green's.
- **The tiles:**
  - a colour image's tile is demosaiced from its level, read with a margin of 2 pixels so that
    its edges interpolate as its inside does;
  - it is balanced by the image's own white: red's and blue's 99.9th percentiles at its top level,
    brought to green's;
  - it is shown through the window on each colour, as the previews are;
  - before the pyramid is built, an image too large to read for every tile is shown unbalanced;
  - the thumbnail does the same.

## Alternatives considered

- **Demosaicing on arrival, and keeping RGB.** Refused by the owner's decision. It would also
  triple the frames' size, and invent two samples in three.
- **A better demosaic,** such as Malvar–He–Cutler or an edge-directed one. Bilinear is exact on
  linear colour, and fast: 4.8 ms for a 1920 × 1080 ROI. It softens edges and can colour-fringe
  them at 1:1. A sharper one can replace the kernel without changing anything around it, if the
  owner's slides ask for it.
- **The overview from bilinear, then shrunk.** It would cost three planes of 100 MP. Whole cells
  give the same colours at a shrink of six or more, and read each sample once: 6.7 ms for the
  overview at 102 MP, where the mono area average takes 10.4 ms.
- **Multiplying the pixels by their gains.** That is a pass over the image. Dividing the window by
  the gain gives the same picture through the tables.
- **A pyramid of luminance, or of RGB.** Luminance loses colour at every zoom but the full one.
  RGB triples the levels and changes the image's channels. Whole cells keep the levels a mosaic
  of the same size as before.
- **The viewer's white balance kept with the dataset,** as the operator saw it in live view.
  That is better for fluorescence, whose brightest isn't white. It waits until the owner's
  datasets ask for it: the image's own white suits brightfield.
- **Grey-world white balance.** A stained slide's mean isn't grey. The brightest is the lamp,
  through the slide's blank parts.

## Consequences

- Every colour frame shows in colour, live and in the viewer. A mono camera's frames are
  unchanged: the same code paths, the same results, the same tests.
- `Renderer::mapping` takes the frame's filter array and gives a `Mapping`, mono or colour, and
  `measure` keeps a `Measuring` from frame to frame.
- `ImageInfo` has a `cfa`. Images written before this phase read as mono.
- The live mosaic still paints a colour frame as mono, through its green's mapping. Registration,
  autofocus, calibration and the stitch still take the mosaic as it is. All of these are P10.7's.
- The simulator's world is grey. Its colour is the lamp's, which automatic white balance takes
  away, so it shows the mechanics, not a stain. A world with colours would cost a signal per
  colour.

## Verification

- `siq-imgproc` `tests/bayer.rs`:
  - both modes against naive references, on every pattern and padded rows;
  - linear colours exact inside a 700 × 400 image, which splits into bands;
  - a single colour at any size;
  - each colour's histogram, the white gains, the mapping balanced and through its tables.
- `siq-types`: `each_pixel_has_the_colour_its_cell_names_and_a_shift_keeps_it`.
- `siq-sim` `tests/optics.rs`: `a_colour_sensor_samples_each_pixel_s_colour_through_its_filter`,
  which checks the shares, binned, and a region at an odd corner as an exact crop.
- `siq-preview` `tests/preview.rs`:
  - a colour frame shown shrunk and at 1:1 from an odd corner, raw and balanced;
  - its statistics per colour, and its sharpness on luminance.
- `siq-pyramid`:
  - `a_colour_image_s_levels_keep_its_filter_s_cells_whole`, against a naive reference, and odd
    chunks refused;
  - `a_colour_image_s_tiles_and_thumbnail_are_demosaiced_and_balanced`, at the full level, the
    top and beyond it.
- `siq-server` `tests/colour.rs`, the simulated colour camera over HTTP:
  - the histogram per colour;
  - `white_from_roi`;
  - a snap's image keeping its filter array, with a tile in colour.
- `kernels_bench` on a 102 MP frame:
  - the colour overview takes 6.7 ms, and the colour ROI 4.8 ms, within the 35 ms and 15 ms
    budgets;
  - the per-colour histograms take 1.9 ms.
- `preview_to_files -- <folder> gbrg`: the 100 MP colour sensor's overview and ROI JPEGs, looked
  at.

## Amendment (P10.7, 2026-10-29): processing on luminance, and the stitch in colour

**Context.** After P10.6, everything that measured a colour frame still took the mosaic as it
was: autofocus, the stage–camera calibration, the flat, the live mosaic and the stitch.

**Decision.**

- **Luminance** (`siq-imgproc` `luminance`): each pixel the 3 × 3 binomial mean about it, mirrored
  at the edges. Wherever it is centred, it weighs red, green and blue 1 : 2 : 1, so it needs no
  pattern, and keeps a pixel's detail and the frame's geometry. On linear colours it is exact
  inside the image.
- **On luminance:**
  - autofocus's sharpness;
  - the stage–camera calibration's frames;
  - the flat, both captured and estimated from a dataset. There is one flat, for every colour.
- **The live mosaic** bins a colour frame by an even factor, rounded up: each pixel is the mean of
  whole cells, which is luminance at no cost. A camera's 2 × 2 binning keeps the pattern, so a
  half-scale canvas bins its frames 2 more, not 1.
- **The stitch:**
  - a tile keeps its frame's filter array (`Tile::cfa`);
  - its pairs are registered on luminance;
  - each colour channel of the dataset is composited as three planes, red, green and blue
    (`Plane::colour`). Each block demosaics, by `demosaic`'s bilinear, only what it samples of a
    tile, with a margin of 2, and takes the plane's colour at the tile's sub-pixel offset. Tiles
    placed at sub-pixel offsets can't keep a Bayer pattern;
  - the stitched image's channels are `<label> red`, `green` and `blue`, pure colours, each in
    the 0.1–99.9 % window of its colour in the first frame: the viewer's additive composite is
    then white balanced. The image isn't a mosaic, so its `cfa` is `None`.
- **Datasets** keep their raw frames, the filter array in each sidecar's `FrameMeta`.
- **`config/sim.toml` stays mono.** Its fluorescence channels keep their tints, and every earlier
  milestone's manual test its look. Colour is tested by the colour variants of each routine's
  test, and `cfa = "gbrg"`, commented in, turns it on.

**Alternatives considered.**

- **A half-resolution luminance,** each cell's mean, as the statistics take it. It would change
  the geometry every consumer has: the calibration's scale, the registration's shifts, autofocus's
  windows. The binomial keeps it, for one more pass.
- **A single colour plane,** green. That is half the samples, and needs interpolating as well.
- **One demosaic for a block's three colours.** It would save two of a block's three demosaics,
  about a quarter of the composite's time, but a block would hold and write three planes. The
  500 MP benchmark meets its target without it.

**Consequences.**

- The stitched colour mosaic is three times the planes. Its composite, at 535 MP, takes 19.7 s
  where mono takes 4.0, and the whole stitch 27.1 s of the 30 s target.
- The registration is a little less sharp on luminance: 0.12 px RMS of the truth on the
  generator's scan, where mono gives 0.064. The target is 0.2 px.

**Verification.**

- `siq-imgproc` `bayer`: `luminance_takes_the_pattern_away_and_keeps_linear_detail`.
- `siq-workflow` `autofocus`: `a_colour_camera_autofocuses_on_luminance`.
- `siq-server`:
  - `calibration`: `a_colour_camera_s_calibration_recovers_its_mount_from_luminance`, to 0.1 %
    and 0.05°;
  - `shading`: `a_colour_camera_s_flat_is_its_luminance_s`, within 2 %.
- `siq-stitch-live` `view`: `a_colour_frame_is_binned_by_whole_cells`.
- `siq-stitch`:
  - `pairs`: every pair of a colour scan within 0.2 px RMS of the truth;
  - `composite`: `a_colour_scan_composites_in_red_green_and_blue`, each plane at 27 dB against the
    ideal mosaic, and red at 0.743 and blue at 0.545 of green, as the lamp gives.
- `siq-jobs` `kinds`: `a_colour_scan_is_stitched_in_red_green_and_blue`.
- `bench_stitch_500mp -- --colour`: 27.1 s and 1.38 GB, within the targets.
