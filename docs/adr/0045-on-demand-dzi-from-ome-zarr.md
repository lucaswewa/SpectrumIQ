# ADR-0045: Deep-zoom tiles rendered on demand from OME-Zarr

- **Status:** Accepted
- **Date:** 2026-10-25
- **Phase:** P4.5
- **Related:** ADR-0042, ADR-0043, ADR-0044, REQ-DATA-005, REQ-NET-003, REQ-VIS-001;
  architecture §6.2, §7.2

## Context

- **The 2D viewer is OpenSeadragon** (architecture §7.2). It reads Deep Zoom (DZI):
  - a descriptor;
  - tiles of 254 pixels with an overlap of 1, at levels from 1 × 1 up to full size.
- **Every image is OME-Zarr with its pyramid** (ADR-0042, ADR-0044). Its levels halve, rounding
  up, as DZI's do.
- **The user changes the display mapping as they look:** per channel, the window, gamma and
  colour. Multi-channel images are composited from one tiled image per channel.
- **The server has a tile budget:** `[memory] tile_cache_mb`, 1 GB by default (architecture §5).

## Decision

- **Tiles are rendered on demand** from the OME-Zarr levels, with the mapping in the request, and
  cached.
- **The geometry is OpenSeadragon's `DziTileSource`:**
  - `max = ⌈log₂ max(width, height)⌉`;
  - level `l` is the image halved `max − l` times, rounding up;
  - tile (`col`, `row`) starts at `254·i − 1` (0 for the first) and is `254 + 1` or `+ 2` wide,
    cut at the level's edge.
- **DZI level `l` is Zarr level `max − l`,** which has the same size.
  - The levels coarser than the image's top level (about 256 pixels, ADR-0044) are halved from it
    by the same 2 × 2 mean, down to one pixel.
- **A tile** is the region read at its level, mapped, and encoded:
  - **`u16`** through `siq-imgproc`'s display tables, the live previews' own;
  - **`f32`** by the same formula;
  - **colours:** grey, inverted, or black to a colour;
  - **encoding:** JPEG at quality 85 (grey or RGB, opaque) or PNG (lossless).
- **The mapping comes from the query:** `min`, `max`, `gamma` (as `display.lut`'s: above 1
  brightens the shadows) and `cmap` (`grey`, `inverted` or `rrggbb`).
  - A value not given is the channel's own, from the image's `omero` window and colour.
  - A window needs `min < max`, and a gamma from 0.05 to 20.
- **The routes** name the dataset by its id and the image by its name in `zarr/`, since a dataset
  holds several:
  - `GET /api/siq/data/{dataset}/{image}/{c}/{z}/{t}.dzi`: the descriptor;
  - `GET …/{t}_files/{level}/{col}_{row}.{jpeg|png}`: a tile. OpenSeadragon repeats the
    descriptor's query on every tile, so the mapping goes on the descriptor's URL;
  - `GET /api/siq/data/{dataset}/{image}/value?c&z&t&level&x&y`: the true value at a pixel of a
    Zarr level, not a shown one.
- **Errors:**
  - an unknown dataset, image, plane, level or tile is a 404;
  - a mapping that isn't valid is a 400;
  - both say why, as `{"detail": …}`.
- **The cache** (`TileService`):
  - an LRU of encoded tiles, bounded by their bytes to `tile_cache_mb`;
  - keyed on the image, its generation, the tile, the mapping and the format;
  - **the generation** is the time the image's metadata last changed: rebuilding its pyramid makes
    new tiles. An image is opened once per generation;
  - **each tile has an ETag,** a hash of its key, stable across restarts. It is sent with
    `Cache-Control: private, no-cache`, so browsers revalidate and get `304 Not Modified` while
    the tile stands.
  - **Rendering** runs on tokio's blocking threads.
  - **Datasets made since the service started** (by a job, or by `siq-datagen`) are found by
    scanning the catalog again on a miss.
- **A static export** for other viewers (`siq_jobs::kinds::DziExportJob`) writes:
  - `<name>.dzi`;
  - every level's tiles, as `<name>_files/<level>/<col>_<row>.<ext>`;
  - with the same rendering, as a background job.

## Alternatives considered

- **Pre-rendering every tile when the pyramid is built:**
  - 1145 tiles for an 8192 × 6144 image, per channel and mapping;
  - every change of window or colour would need them all again;
  - on-demand tiles cost a read and an encode for the tiles looked at.
- **Client-side mapping of 16-bit tiles (WebGL):** the right end state for interactive window and
  level, and P14.1's. It needs a renderer the browser doesn't have yet.
- **Tile size 256 with no overlap:** OpenSeadragon's default is 254 + 1, and overlap avoids seams
  when tiles are drawn scaled.
- **`max-age` caching in browsers:** fewer requests, but stale tiles after a pyramid is rebuilt.
  ETags give correctness at the cost of a revalidation.
- **The architecture's route without the image** (`{dataset}/dzi/{channel}/{plane}.dzi`): a
  dataset has a mosaic, a depth map and a volume, each an image.

## Consequences

- **The viewer (P4.8) changes the display by changing the descriptor's query:** new tiles are
  rendered, and the old ones stay cached for going back.
- **A cold tile costs a region read and an encode:** milliseconds for 255 × 255. A warm one is a
  memory copy.
- **Revalidation is a round trip per tile in view.** Fine on the LAN; P14.1's 16-bit tiles change
  the trade-off.
- **Architecture §6.2's table is updated** to the routes as built.

## Verification

- `crates/siq-pyramid/tests/tiles.rs`:
  - a property test of the geometry against OpenSeadragon's formulas, written apart;
  - neighbours sharing two pixels, and the descriptor;
  - a tile's pixels through a LUT, decoded from PNG;
  - the channel's own window and colour, and JPEG;
  - levels beyond the top, down to one pixel;
  - bad mappings and missing tiles;
  - the value probe;
  - the cache's eviction and ETags;
  - 8 threads asking for the same tiles.
- `crates/siq-server/tests/data.rs`:
  - the descriptor, a JPEG tile and its 304, a PNG tile's pixels, and the probe over HTTP;
  - each 404 and 400;
  - a dataset made after the service started.
- `crates/siq-jobs/tests/kinds.rs`: the export writes every tile of every level, and the
  descriptor, and is recorded.
- **`export_dzi`:** an 8192 × 6144 image's pyramid and its 1145-tile export through the
  scheduler.
