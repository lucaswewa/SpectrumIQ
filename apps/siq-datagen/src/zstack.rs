//! `zstack`: a focus stack over a height field: one field of view at evenly spaced focus
//! positions, each frame sharp where the surface is in focus. The ground truth is the surface's
//! height under every pixel (`truth/world.height.f32`). As the simulator has it, the defocus is the
//! focus plus the height, so the frame at focus `f` is sharp where the height is `-f`.

use std::path::PathBuf;

use clap::{ArgMatches, Args, Command, FromArgMatches};
use serde::Serialize;
use serde_json::json;
use siq_synth::{HeightFieldParams, WorldInfo, write_ground_truth};
use siq_types::{RectUm, StagePose, Vec2Um};

use crate::{Camera, Generator, Output, dataset, key, record, um, write_stack, write_truth};

/// A z-stack's parameters.
#[derive(Debug, Clone, Args, Serialize)]
pub struct ZStackArgs {
    /// The frame's width, in pixels.
    #[arg(long, default_value_t = 2048)]
    pub width: u32,
    /// The frame's height, in pixels.
    #[arg(long, default_value_t = 1536)]
    pub height: u32,
    /// The focus planes, centred on focus 0.
    #[arg(long, default_value_t = 41)]
    pub planes: u32,
    /// Their spacing, in µm.
    #[arg(long, default_value_t = 2.0)]
    pub step: f64,
    /// The relief: its height runs from minus to plus this, in µm.
    #[arg(long, default_value_t = 30.0)]
    pub amplitude: f64,
}

/// The `zstack` generator.
pub struct ZStack;

impl Generator for ZStack {
    fn command(&self) -> Command {
        let about = "A focus stack over a height field, and the true height of every pixel";
        ZStackArgs::augment_args(Command::new("zstack").about(about))
    }

    fn generate(&self, args: &ArgMatches, out: &Output) -> anyhow::Result<PathBuf> {
        let mut args = ZStackArgs::from_arg_matches(args)?;
        if out.smoke {
            (
                args.width,
                args.height,
                args.planes,
                args.step,
                args.amplitude,
            ) = (128, 96, 9, 5.0, 15.0);
        }
        zstack(&args, out)
    }
}

/// The focus of each of `planes` planes `step` apart, centred on 0.
pub fn focus_positions(planes: u32, step: f64) -> Vec<f64> {
    let middle = f64::from(planes.saturating_sub(1)) / 2.0;
    (0..planes)
        .map(|k| (f64::from(k) - middle) * step)
        .collect()
}

/// Writes the z-stack `args` describe: its dataset's folder.
pub fn zstack(args: &ZStackArgs, out: &Output) -> anyhow::Result<PathBuf> {
    let world = WorldInfo::HeightField(HeightFieldParams {
        seed: out.seed,
        amplitude: um(args.amplitude),
        ..HeightFieldParams::default()
    });
    let built = world.build()?;
    let camera = Camera::new((args.width, args.height), out.seed);
    let (dataset, truth) = dataset(out, "zstack", serde_json::to_value(args)?)?;
    let focus = focus_positions(args.planes, args.step);
    let mut frames = Vec::new();
    let mut planes = Vec::new();
    for (z, &f) in (0u32..).zip(&focus) {
        let pose = StagePose {
            focus: um(f),
            ..StagePose::default()
        };
        let frame = camera.render(built.as_ref(), pose, z.into())?;
        record(&dataset, &camera, (key(z, 0), z.into()), pose, &frame)?;
        planes.push(json!({ "frame": key(z, 0).to_string(), "focus": f }));
        frames.push(frame);
    }
    let [width, height] = camera.field();
    let region = RectUm {
        origin: Vec2Um::from_um(-width / 2.0, -height / 2.0),
        size: Vec2Um::from_um(width, height),
    };
    write_ground_truth(built.as_ref(), region, um(camera.pixel()), &truth, "world")?;
    let half = camera.pixel() / 2.0;
    let origin = [focus[0], half - height / 2.0, half - width / 2.0];
    write_stack(&dataset, &camera, ("stack", &frames), origin, args.step)?;
    write_truth(
        &truth,
        "zstack",
        &json!({
            "pixel_size": camera.pixel(),
            "tile": [args.width, args.height],
            "planes": planes,
            "height_map": "world.height.f32",
            "sharp_where": "height = -focus",
            "image": "zarr/stack.ome.zarr",
        }),
    )?;
    Ok(dataset.path().to_owned())
}
