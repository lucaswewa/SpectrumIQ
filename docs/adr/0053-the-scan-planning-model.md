# ADR-0053: The scan planning model

- **Status:** Accepted
- **Date:** 2026-10-26
- **Phase:** P6.2
- **Related:** ADR-0049, ADR-0051, ADR-0052, REQ-WF-001, REQ-STG-005, REQ-STI-003; `microscope`'s
  ADR-0028

## Context

- **A tile scan covers a region of the slide** with frames that overlap their neighbours, so
  that stitching (M8) can register them and blend the seams. The overlap asked for is the
  overlap stitching gets.
- **The camera is never square to the stage** (ADR-0049): 0.4° on the simulator. A grid along
  the stage's axes would make neighbours overlap more on one side than the other.
- **Regions aren't always rectangles:** a tissue section is drawn as a polygon, and the tiles that
  miss it are time and disk wasted.
- **The UI (P6.6) shows a scan before it runs:** its tiles, how long, how many bytes, and whether
  the disk holds them.
- **`microscope`'s planners** (its ADR-0028) give the next site one at a time, for its spiral that
  follows the sample. Regular grids don't need to be told what each site found.

## Decision

- **A pure planner** (`siq-workflow::scan::plan`): a region, the calibration `L`, the frame in
  sensor pixels, the overlap and an order give a `Grid` of tiles, each with its column, row and
  stage position.
- **The grid runs along the camera's axes:**
  - a step along a row moves the stage by `L⁻¹·[(1 − overlap)·width, 0]`, and along a column by
    `L⁻¹·[0, (1 − overlap)·height]`;
  - so neighbours overlap by exactly `overlap` of the frame, and the image moves straight across
    and down, whatever the camera's rotation.
- **Coverage:**
  - the grid is laid in steps over the region's extent, centred on it, with enough tiles each way
    that their frames reach its edges. A frame's centre is where the stage is (architecture
    §13);
  - **a tile is kept if its frame meets the region:** a vertex of either inside the other, or
    their edges crossing. A polygon may be concave.
- **Orders:**
  - `snake` (the default): row by row, turning at each end;
  - `raster`: every row the same way;
  - `nearest`: from the first tile, the nearest not yet visited, for a clipped region whose rows
    have gaps.
- **The engine's `tile_scan` step** (ADR-0052) holds the region, overlap, order, channels, settle
  time and an optional fixed focus. When a run starts it is made into the engine's own steps, by
  the calibration in force and the camera's region:
  - for each tile: `index` p, `move_to`, a `wait` to settle, then each channel and its `snap`;
  - one channel is put in place once, before the first tile;
  - moves approach from the same side (REQ-STG-005).

  A plan stays as written in the manifest, and the grid goes beside it in the dataset's
  `layout.json`: each tile's column, row and planned position, for stitching. The frames carry the
  pose the stage reported. A dataset with a tile scan is of the kind `tile_scan`.
- **A tile scan needs the stage–camera calibration in force,** and is refused without it: the
  gating rule of ADR-0051.
- **The estimate** (`scan::estimate`):
  - each move's time at the stage's speed and acceleration, with an overhead for the driver
    seeing it done, plus the settle time;
  - one channel switch, or one per channel per tile;
  - each frame's exposure and readout;
  - the bytes of every frame, and `Estimate::fits` for the disk. The frames pass through the pool
    to the disk, so a scan's RAM doesn't grow with it.

  `Timing`'s defaults are the simulator's, as measured: 50 mm/s, 200 mm/s², 0.1 s a move (the
  position is read at 5 Hz), 0.15 s a channel, 0.12 s a readout.

## Alternatives considered

- **A grid along the stage's axes:** simpler, but neighbours at 0.4° overlap unevenly, by up to
  `height × tan 0.4°` more on one side.
- **The planner told site by site, as `microscope`'s is:** its spiral needs it, but a grid is
  known whole. Knowing it whole gives the count, the estimate and the layout before the run.
- **Tiles whose centres are in the region:** cheaper to test, but the region's edge would fall
  between frames.
- **Every channel at a tile before moving on, or every tile in a channel before switching:** per
  tile keeps each tile's channels at one pose; per channel saves switches but moves the stage
  over the region again for each. Per tile is the default; a plan can do the other with one
  `tile_scan` per channel.
- **The layout in the manifest:** the manifest records the plan that was asked for; the layout is
  what it became on this instrument, beside it.

## Consequences

- **The overlap is exact in the image,** so stitching's search can be narrow.
- **A calibration changed after a scan** doesn't move its tiles: `layout.json` holds where they
  were planned.
- **The estimate is only as good as `Timing`:** on the simulator it came within 3 % of a run. A real
  stage gives its own values (P6.6, P10.3).
- **A scan of many tiles in several channels switches filters at every tile.** It's the default
  for the same pose, at the cost of wheel time.

## Verification

- `crates/siq-workflow/tests/scan.rs`:
  - as a property, a rectangle covered with the overlap asked, whatever the rotation, each tile
    once;
  - a concave polygon keeping only the tiles that meet it;
  - the three orders;
  - a 3 × 3 scan on the simulator, through the GRBL driver: its dataset, its layout, every frame
    within 1 µm of its tile, in 3.38 s against 3.48 s estimated.
- **`tilescan_sim`:** a 5 × 5 scan in two channels (3 × 3 in smoke mode), its estimate and its
  run.
