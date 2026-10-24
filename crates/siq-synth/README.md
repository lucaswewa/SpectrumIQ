# siq-synth

SpectrumIQ's synthetic specimens: deterministic worlds with exact ground truth, for the camera
simulator and for generated datasets, and real images as worlds.

- A world is a pure function of sample (stage) coordinates. Regions sampled apart agree exactly
  wherever they meet, so overlapping tiles see the same specimen.
- Sampling is told each pixel's footprint, and averages finer detail away instead of aliasing.
- A world's description rebuilds it, and its ground truth is written as JSON plus raw `f32`.

The design is in [ADR-0024](../../docs/adr/0024-synthetic-worlds-and-ground-truth.md).

## Place in the architecture

- **Layer:** L1 Core.
- **Depends on:**
  - `siq-types` (coordinates, image views);
  - `siq-config` (`WorldConfig`);
  - `png` and `tiff` (the image world);
  - `serde`, `serde_json`, `schemars`, `thiserror`.
- **Used by:**
  - P1.6's image formation and P1.7's simulated camera;
  - `siq-datagen` (P4.9, P12.4, P13.1);
  - the ground-truth tests of M5, M8 and M11–M13.
- **Phase:** P1.4.

## API

| Item | What |
|---|---|
| `World` | `intensity(at, footprint)` from 0 to 1, in focus. Also `surface_z(at)`, `density(at, z)`, `beads(region)`, `info()` |
| `sample(&world, &PixelToStage, &mut ImageViewMut<f32>)` | Renders on any pixel grid: pixel `(c, r)` at the mapping of `(c + 0.5, r + 0.5)` |
| `grid(origin, scale)` | The axis-aligned grid of `scale` µm pixels from `origin` |
| `Cells`, `Grains`, `HeightField`, `Beads` | Procedural worlds, each with a `…Params` (seed and sizes, all with defaults) |
| `ImageWorld::open(ImageParams { path, pixel_size })` | A PNG or TIFF, centred on the origin, with a pyramid for coarse samplings. `from_image` takes an image already in memory |
| `WorldInfo` | `{"kind": "cells", "seed": 7, …}`: `build()` makes the world again, and `from_config(&WorldConfig, seed)` maps `[sim.world]` |
| `write_ground_truth(world, region, scale, dir, name)` → `GroundTruth` | `<name>.json`, plus `<name>.height.f32` for worlds with a surface. `HeightMap::read` reads the heights back |
| `Bead` | x, y, z, σ and brightness |
| `SynthError` | `Io` or `Image`, naming the file |

```rust
let world = WorldInfo::from_config(&config.sim.world, config.sim.seed).build()?;
let mut image = OwnedImage::new(2048, 1536);
sample(world.as_ref(), &grid(Vec2Um::from_um(-512.0, -384.0), Micrometers(0.5)), &mut image.view_mut());
write_ground_truth(world.as_ref(), region, Micrometers(2.0), &dataset, "tile_0001")?;
```

## Examples

| Example | What it shows | Run |
|---|---|---|
| `render_world` | Renders 2048 × 1536 pixels of each world to 16-bit PNGs, at 0.5 µm (beads at 0.1 µm), with ground truth beside them. It then reads the cells' PNG back as an image world and renders it at 1 µm. Smoke mode renders 256 × 192 | `cargo run --release -p siq-synth --example render_world [-- <folder>]` |

## Features

None.

## Testing

`cargo test -p siq-synth` runs two binaries:

| Binary | What |
|---|---|
| `worlds` | Determinism by seed; seams, as a proptest over the five worlds, bit for bit; statistics; footprint averaging; descriptions that round-trip and rebuild; ground truth written and read back |
| `image_world` | 16-bit PNG and TIFF read and laid out at their pixel size; the pyramid's 2 × 2 means; errors naming the file |

## Limitations

- **Single-threaded sampling,** at 67–133 ns per pixel in a release build. P1.6 parallelizes and
  caches (ADR-0024).
- **An image world is loaded whole,** as `f32` with its pyramid: a 20 000 × 20 000 image takes
  about 2 GB.
- **Worlds are in focus and noiseless.** Defocus, vignetting and noise are P1.6's.
