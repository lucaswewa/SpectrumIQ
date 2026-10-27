//! `tilescan`: the tiles of a mosaic over a 2D world, cells or grains, on a grid. The stage misses
//! each nominal pose by a random error: each frame is rendered at its true pose, and recorded with
//! its nominal one. The ground truth is each tile's true pose, and the world itself as the ideal
//! mosaic, `zarr/truth.ome.zarr`: what a perfect stitch gives, and what the viewer browses.

use std::path::PathBuf;

use anyhow::bail;
use clap::{ArgMatches, Args, Command, FromArgMatches};
use rayon::prelude::*;
use serde::Serialize;
use serde_json::json;
use siq_storage::Dataset;
use siq_storage::zarr::{CHUNK, ChannelInfo, ImageInfo, SampleType, Size5, ZarrImage};
use siq_synth::{CellsParams, GrainsParams, World, WorldInfo, grid, sample, write_ground_truth};
use siq_types::{OwnedImage, RectPx, RectUm, StagePose, Vec2Um};

use crate::{Camera, Generator, Output, dataset, gaussian, key, record, um, write_truth};

/// A tile scan's parameters.
#[derive(Debug, Clone, Args, Serialize)]
pub struct TileScanArgs {
    /// The world: `cells` or `grains`.
    #[arg(long, default_value = "cells")]
    pub world: String,
    /// The tiles across.
    #[arg(long, default_value_t = 8)]
    pub columns: u32,
    /// The tiles down.
    #[arg(long, default_value_t = 7)]
    pub rows: u32,
    /// A tile's width, in pixels.
    #[arg(long, default_value_t = 4096)]
    pub width: u32,
    /// A tile's height, in pixels.
    #[arg(long, default_value_t = 3072)]
    pub height: u32,
    /// The share of a tile that its neighbour overlaps, from 0 to 1.
    #[arg(long, default_value_t = 0.15)]
    pub overlap: f64,
    /// The stage's error: a standard deviation, per axis, in µm.
    #[arg(long, default_value_t = 2.0)]
    pub stage_error: f64,
    /// The light, as a share of the camera's usual: less is noisier.
    #[arg(long, default_value_t = 1.0)]
    pub light: f64,
    /// Leaves out the ideal mosaic.
    #[arg(long)]
    pub no_mosaic: bool,
}

/// The `tilescan` generator.
pub struct TileScan;

impl Generator for TileScan {
    fn command(&self) -> Command {
        let about = "A tile scan with known stage errors, and its ideal mosaic";
        TileScanArgs::augment_args(Command::new("tilescan").about(about))
    }

    fn generate(&self, args: &ArgMatches, out: &Output) -> anyhow::Result<PathBuf> {
        let mut args = TileScanArgs::from_arg_matches(args)?;
        if out.smoke {
            (args.columns, args.rows, args.width, args.height) = (2, 2, 256, 192);
        }
        tilescan(&args, out)
    }
}

/// A pose in the plane, at focus.
fn pose([x, y]: [f64; 2]) -> StagePose {
    StagePose {
        x: um(x),
        y: um(y),
        ..StagePose::default()
    }
}

/// Writes the tile scan `args` describe: its dataset's folder.
pub fn tilescan(args: &TileScanArgs, out: &Output) -> anyhow::Result<PathBuf> {
    if !(0.0..1.0).contains(&args.overlap) || args.columns == 0 || args.rows == 0 {
        bail!("a scan needs a tile, and an overlap from 0 to less than 1");
    }
    if args.light.is_nan() || args.light <= 0.0 {
        bail!("a scan needs some light");
    }
    let world = match args.world.as_str() {
        "cells" => WorldInfo::Cells(CellsParams {
            seed: out.seed,
            ..CellsParams::default()
        }),
        "grains" => WorldInfo::Grains(GrainsParams {
            seed: out.seed,
            ..GrainsParams::default()
        }),
        other => bail!("the world is cells or grains, not {other}"),
    };
    let built = world.build()?;
    let mut camera = Camera::new((args.width, args.height), out.seed);
    camera.optics.flux *= args.light;
    let [width, height] = camera.field();
    let step = [width, height].map(|field| field * (1.0 - args.overlap));
    let (dataset, truth) = dataset(out, "tilescan", serde_json::to_value(args)?)?;

    // Raster order: each tile's nominal centre, and where the stage truly put it.
    let cells = (0..args.rows).flat_map(|row| (0..args.columns).map(move |column| (column, row)));
    let mut tiles = Vec::new();
    for (p, (column, row)) in (0u32..).zip(cells) {
        let nominal = [f64::from(column) * step[0], f64::from(row) * step[1]];
        let error = [0, 1].map(|axis| args.stage_error * gaussian(out.seed, p.into(), axis));
        let actual = [nominal[0] + error[0], nominal[1] + error[1]];
        let frame = camera.render(built.as_ref(), pose(actual), p.into())?;
        record(
            &dataset,
            &camera,
            (key(0, p), p.into()),
            pose(nominal),
            &frame,
        )?;
        tiles.push(json!({
            "frame": key(0, p).to_string(),
            "nominal": nominal,
            "true": actual,
            "error": error,
        }));
    }

    // The nominal tiles' extent.
    let region = RectUm {
        origin: Vec2Um::from_um(-width / 2.0, -height / 2.0),
        size: Vec2Um::from_um(
            step[0] * f64::from(args.columns - 1) + width,
            step[1] * f64::from(args.rows - 1) + height,
        ),
    };
    write_ground_truth(built.as_ref(), region, um(camera.pixel()), &truth, "world")?;
    let mosaic = if args.no_mosaic {
        serde_json::Value::Null
    } else {
        let path = mosaic(&dataset, &camera, built.as_ref(), region)?;
        json!({ "image": path, "origin": region.origin.to_um(), "pixel_size": camera.pixel() })
    };
    let [x0, y0] = region.origin.to_um();
    write_truth(
        &truth,
        "tilescan",
        &json!({
            "pixel_size": camera.pixel(),
            "tile": [args.width, args.height],
            "grid": { "columns": args.columns, "rows": args.rows, "step": step },
            "stage_error": args.stage_error,
            "tiles": tiles,
            "region": { "origin": [x0, y0], "size": region.size.to_um() },
            "mosaic": mosaic,
        }),
    )?;
    Ok(dataset.path().to_owned())
}

/// Writes the world over `region`, in focus, as the camera's pixels see it without vignetting or
/// noise, as the image `truth` with its pyramid: its path in the dataset.
fn mosaic(
    dataset: &Dataset,
    camera: &Camera,
    world: &dyn World,
    region: RectUm,
) -> anyhow::Result<String> {
    let pixel = camera.pixel();
    let [width, height] = region
        .size
        .to_um()
        .map(|size| (size / pixel).round() as u64);
    let [x0, y0] = region.origin.to_um();
    // A frame's level for the world's intensity: its electrons, in ADU, over the dark offset.
    let gain = camera.optics.flux * 10.0 / camera.noise.electrons_per_adu;
    let offset = camera.noise.dark_offset;
    let info = ImageInfo {
        name: "truth".to_owned(),
        sample: SampleType::U16,
        size: Size5 {
            t: 1,
            c: 1,
            z: 1,
            y: height,
            x: width,
        },
        levels: 1,
        chunk: CHUNK,
        pixel_size: [1.0, pixel, pixel],
        origin: [0.0, y0 + pixel / 2.0, x0 + pixel / 2.0],
        channels: vec![ChannelInfo {
            label: "world".to_owned(),
            colour: [255; 3],
            window: (offset, gain + offset),
        }],
    };
    let path = dataset.zarr_dir().join("truth.ome.zarr");
    let mut image = ZarrImage::create(&path, info)?;
    let (rows, columns) = image.info().chunks(0);
    (0..rows * columns).into_par_iter().try_for_each(|k| {
        let (row, column) = (k / columns, k % columns);
        let (x, y) = (column * CHUNK, row * CHUNK);
        let rect = RectPx::new(
            x as u32,
            y as u32,
            CHUNK.min(width - x) as u32,
            CHUNK.min(height - y) as u32,
        );
        let mut signal = OwnedImage::<f32>::new(rect.width as usize, rect.height as usize);
        let origin = Vec2Um::from_um(x0 + x as f64 * pixel, y0 + y as f64 * pixel);
        sample(world, &grid(origin, um(pixel)), &mut signal.view_mut());
        let chunk = OwnedImage::from_fn(rect.width as usize, rect.height as usize, |i, j| {
            let intensity = f64::from(*signal.view().get(i, j).expect("inside"));
            (intensity * gain + offset).round().clamp(0.0, 16383.0) as u16
        });
        image.write_chunk(0, (0, 0, 0), (row, column), chunk.view())
    })?;
    siq_pyramid::build(&mut image, siq_pyramid::TOP, &())?;
    Ok("zarr/truth.ome.zarr".to_owned())
}
