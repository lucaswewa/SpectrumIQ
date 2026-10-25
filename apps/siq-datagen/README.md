# siq-datagen

Synthetic datasets with exact ground truth (ADR-0047), for developing and testing algorithms
without the instrument: tile scans with known stage errors, z-stacks over known surfaces, and bead
volumes. The frames are rendered offline by `siq-sim`'s image formation, and written into a data
root as the service writes datasets, so the service, its viewer and its jobs read them.

```text
siq-datagen <generator> [--out <data root>] [--seed <n>] [--smoke] [the generator's arguments]
siq-datagen <generator> --help                 the generator's arguments
```

| Generator | Writes | Ground truth (`truth/`) |
|---|---|---|
| `tilescan` | A grid of tiles over a cells or grains world (8 × 7 tiles of 4096 × 3072 by default, 15 % overlap), each taken at its nominal pose by a stage off by a random error (2 µm a standard deviation, per axis), and recorded with the nominal pose; and the ideal mosaic, `zarr/truth.ome.zarr` (28 467 × 18 739, 533 MP, by default), with its pyramid | `tilescan.json`: each tile's frame, nominal and true pose, and error; the grid, the region and the mosaic. `world.json` |
| `zstack` | 41 frames of a height field, 2 µm apart in focus (2048 × 1536), and the stack as `zarr/stack.ome.zarr` | `zstack.json`: each plane's focus. `world.json` and `world.height.f32`: the height under every pixel |
| `beads` | 41 frames through a bead volume, 1 µm apart (1024 × 1024), each bead blurred by its own defocus, and the stack as `zarr/stack.ome.zarr` | `beads.json`: each plane's focus, and the PSF. `world.json`: every bead's position, size and brightness |

- **`--out`:** the data root, `data` by default: the simulated instrument's when run from the
  repository. The dataset goes in its `datasets/` folder.
- **`--seed`:** the world's, the stage errors' and the noise's: 1 by default. The same seed gives the
  same dataset.
- **`--smoke`:** small sizes, for CI: 2 × 2 tiles of 256 × 192, or stacks of 128 pixels.

The datasets are of kind `synthetic`; their manifest's plan holds the generator and its arguments.
Their frames are in `raw/`, as the recorder writes them, keyed by tile (`p`) or plane (`z`).

With the simulated instrument, from the repository:

```text
cargo run --release -p siq-datagen -- tilescan
cargo run --release -p spectrumiq-server -- -c config/sim.toml
```

Then open the Viewer at <http://127.0.0.1:5000/#/datasets>, and View the tile scan's mosaic.

## Place in the architecture

- **Layer:** App.
- **Depends on:** `siq-sim` (the image formation), `siq-synth` (the worlds and their ground
  truth), `siq-storage` (datasets, raw frames, OME-Zarr), `siq-pyramid`, `siq-devices`,
  `siq-types`; `clap`, `rayon`, `serde`, `serde_json`, `anyhow`.
- **Phase:** P4.9. M12 (SIM phase sets) and M13 (light fields) add their generators.

## Adding a generator

A generator is a type implementing `siq_datagen::Generator`:

- `command()`: its subcommand, typically a `clap::Args` struct's, augmented;
- `generate(args, out)`: writes its dataset, and gives its folder.

It is listed in `GENERATORS`, which `main` makes the subcommands from. The helpers in the crate's
root make a dataset and its `truth/` folder, model the camera (`Camera::render`, `digitize`),
record frames, and write a stack as OME-Zarr with its pyramid.

## Testing

`cargo test -p siq-datagen`, at smoke sizes:

- **`tilescan`:** each tile's recorded pose is its nominal one, and the truth's error is within five
  standard deviations. Each tile matches the ideal mosaic where the truth puts it, better than
  where the stage said (correlation above 0.9).
- **`zstack`:** in at least 80 % of 16-pixel blocks, the sharpest plane is within one of the
  surface's true focus. The stack's planes are the frames.
- **`beads`:** at least 90 % of the beads are brightest within a plane of their depth.
- **The CLI** writes a dataset that the catalog finds, and refuses an unknown world.

## Limitations

- **The calibration is ideal:** stage and camera axes aligned, at the nominal pixel size. M5's
  calibration errors come later.
- **The stage errors are independent** from tile to tile; no drift or backlash.
- **Frames are rendered one after another,** each in parallel inside: 56 tiles of 12.6 MP and the
  533 MP mosaic take about 25 s.
