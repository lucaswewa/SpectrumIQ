//! `beads`: a focus stack through a volume of fluorescent beads, for deconvolution and 3D work.
//! Each bead is drawn through a Gaussian PSF as wide as its own size and the optics' blur at its
//! defocus, added in quadrature; it keeps its light, so it peaks at its brightness in focus and
//! dims as it spreads. The camera's noise follows. As for surfaces, the defocus is the focus plus
//! the bead's height. The ground truth is every bead: position, size and brightness
//! (`truth/world.json`).

use std::path::PathBuf;

use clap::{ArgMatches, Args, Command, FromArgMatches};
use rayon::prelude::*;
use serde::Serialize;
use serde_json::json;
use siq_synth::{Bead, BeadsParams, WorldInfo, write_ground_truth};
use siq_types::{OwnedImage, RectUm, StagePose, Vec2Um};

use crate::zstack::focus_positions;
use crate::{Camera, Generator, Output, dataset, key, record, um, write_stack, write_truth};

/// A bead stack's parameters.
#[derive(Debug, Clone, Args, Serialize)]
pub struct BeadsArgs {
    /// The frame's width, in pixels.
    #[arg(long, default_value_t = 1024)]
    pub width: u32,
    /// The frame's height, in pixels.
    #[arg(long, default_value_t = 1024)]
    pub height: u32,
    /// The focus planes, centred on focus 0.
    #[arg(long, default_value_t = 41)]
    pub planes: u32,
    /// Their spacing, in µm.
    #[arg(long, default_value_t = 1.0)]
    pub step: f64,
    /// The grid the beads are placed on, at most one per cube, in µm.
    #[arg(long, default_value_t = 15.0)]
    pub spacing: f64,
    /// The share of the grid's cubes with a bead, from 0 to 1.
    #[arg(long, default_value_t = 0.5)]
    pub density: f64,
}

/// The `beads` generator.
pub struct BeadStack;

impl Generator for BeadStack {
    fn command(&self) -> Command {
        let about = "A focus stack through a bead volume, and every bead's true position";
        BeadsArgs::augment_args(Command::new("beads").about(about))
    }

    fn generate(&self, args: &ArgMatches, out: &Output) -> anyhow::Result<PathBuf> {
        let mut args = BeadsArgs::from_arg_matches(args)?;
        if out.smoke {
            (args.width, args.height, args.planes, args.step) = (128, 128, 11, 4.0);
        }
        beads(&args, out)
    }
}

/// The noiseless signal at `focus` of `beads`, on `camera`'s frame centred on the stage's origin.
fn signal(camera: &Camera, beads: &[Bead], focus: f64) -> OwnedImage<f32> {
    let (width, height) = (camera.sensor.0 as usize, camera.sensor.1 as usize);
    let pixel = camera.pixel();
    let mut out = OwnedImage::new(width, height);
    let mut view = out.view_mut();
    let sharpest =
        |bead: &Bead, defocus: f64| bead.sigma.0.hypot(camera.optics.blur(um(defocus)).0);
    for bead in beads {
        let sigma = sharpest(bead, focus + bead.z.0);
        let peak = f64::from(bead.brightness) * (sharpest(bead, 0.0) / sigma).powi(2);
        // The bead's centre, in pixels, and how far it reaches.
        let (cx, cy) = (
            bead.x.0 / pixel + width as f64 / 2.0 - 0.5,
            bead.y.0 / pixel + height as f64 / 2.0 - 0.5,
        );
        let reach = 4.0 * sigma / pixel;
        let span = |centre: f64, size: usize| {
            ((centre - reach).floor().max(0.0) as usize)
                ..((centre + reach).ceil().max(0.0) as usize).min(size)
        };
        let (s2, columns) = (2.0 * (sigma / pixel).powi(2), span(cx, width));
        for y in span(cy, height) {
            let row = view.row_mut(y);
            for x in columns.clone() {
                let r2 = (x as f64 - cx).powi(2) + (y as f64 - cy).powi(2);
                row[x] += (peak * (-r2 / s2).exp()) as f32;
            }
        }
    }
    out
}

/// Writes the bead stack `args` describe: its dataset's folder.
pub fn beads(args: &BeadsArgs, out: &Output) -> anyhow::Result<PathBuf> {
    let focus = focus_positions(args.planes, args.step);
    let depth = focus.last().copied().unwrap_or(0.0);
    let world = WorldInfo::Beads(BeadsParams {
        seed: out.seed,
        spacing: um(args.spacing),
        density: args.density,
        z_min: um(-depth),
        z_max: um(depth),
        ..BeadsParams::default()
    });
    let built = world.build()?;
    let camera = Camera::new((args.width, args.height), out.seed);
    let (dataset, truth) = dataset(out, "beads", serde_json::to_value(args)?)?;
    let [width, height] = camera.field();
    let region = RectUm {
        origin: Vec2Um::from_um(-width / 2.0, -height / 2.0),
        size: Vec2Um::from_um(width, height),
    };
    let beads =
        write_ground_truth(built.as_ref(), region, um(camera.pixel()), &truth, "world")?.beads;
    let frames = focus
        .par_iter()
        .enumerate()
        .map(|(z, &f)| camera.digitize(signal(&camera, &beads, f).view(), z as u64))
        .collect::<anyhow::Result<Vec<_>>>()?;
    let mut planes = Vec::new();
    for ((z, frame), &f) in (0u32..).zip(&frames).zip(&focus) {
        let pose = StagePose {
            focus: um(f),
            ..StagePose::default()
        };
        record(&dataset, &camera, (key(z, 0), z.into()), pose, frame)?;
        planes.push(json!({ "frame": key(z, 0).to_string(), "focus": f }));
    }
    let half = camera.pixel() / 2.0;
    let origin = [focus[0], half - height / 2.0, half - width / 2.0];
    write_stack(&dataset, &camera, ("stack", &frames), origin, args.step)?;
    write_truth(
        &truth,
        "beads",
        &json!({
            "pixel_size": camera.pixel(),
            "tile": [args.width, args.height],
            "planes": planes,
            "beads": beads.len(),
            "psf": {
                "model": "gaussian",
                "sigma": "hypot(bead sigma, optics blur at focus + bead z)",
                "numerical_aperture": camera.optics.numerical_aperture,
                "wavelength_nm": camera.optics.wavelength.0,
            },
            "sharp_where": "bead z = -focus",
            "image": "zarr/stack.ome.zarr",
        }),
    )?;
    Ok(dataset.path().to_owned())
}
