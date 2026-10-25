//! Synthetic datasets with exact ground truth (ADR-0047), for algorithm work without devices: the
//! generators behind `siq-datagen`'s subcommands.
//!
//! - **A generator** is a subcommand: its arguments, and the dataset it writes. [`GENERATORS`]
//!   lists them; a phase adds its own, such as M12's SIM phase sets or M13's light fields, by
//!   adding it there.
//! - **A dataset** is of kind `synthetic`, and its manifest's plan is the generator's arguments:
//!   - its frames in `raw/`, rendered offline by `siq-sim`'s image formation, as the recorder
//!     writes them;
//!   - its images in `zarr/`, with their pyramids, for the viewer;
//!   - its ground truth in `truth/`: `world.json`, `siq-synth`'s (ADR-0024), and the generator's
//!     own `<generator>.json`.

use std::path::{Path, PathBuf};

use anyhow::Context as _;
use clap::{ArgMatches, Command};
use siq_devices::{CameraSettings, TriggerMode};
use siq_sim::optics::{OpticsModel, Scratch, SensorModel, Shot, digitize, form_image};
use siq_storage::zarr::{CHUNK, ChannelInfo, ImageInfo, SampleType, Size5, ZarrImage};
use siq_storage::{Dataset, DatasetKind, FrameKey, NewDataset, write_frame};
use siq_synth::World;
use siq_types::{
    Affine2, Binning, FrameFlags, FrameMeta, FrameSeq, ImageView, Micrometers, Millis, OwnedImage,
    PixelFormat, PoseQuality, Roi, StagePose, Timestamp,
};

pub mod beads;
pub mod tilescan;
pub mod zstack;

/// A kind of synthetic dataset: a subcommand of `siq-datagen`.
pub trait Generator: Sync {
    /// The subcommand, with its arguments.
    fn command(&self) -> Command;
    /// Writes the dataset that `args` describe under `out`: its folder.
    fn generate(&self, args: &ArgMatches, out: &Output) -> anyhow::Result<PathBuf>;
}

/// The generators, one per subcommand.
pub const GENERATORS: &[&dyn Generator] =
    &[&tilescan::TileScan, &zstack::ZStack, &beads::BeadStack];

/// Where, and how big, a generator writes.
#[derive(Debug, Clone)]
pub struct Output {
    /// The data root: the dataset goes in its `datasets/`.
    pub data_root: PathBuf,
    /// Small sizes, for CI.
    pub smoke: bool,
    /// The seed of the world, the stage's errors and the noise.
    pub seed: u64,
}

/// The camera that takes the frames: the simulated instrument's 3.76 µm sensor behind a 10×/0.30
/// objective, Mono14, 10 ms, with the simulator's optics and sensor models.
#[derive(Debug, Clone, Copy)]
pub struct Camera {
    /// The frame's width and height.
    pub sensor: (u32, u32),
    /// The sensor's pixel pitch, in µm.
    pub pitch: f64,
    /// The objective's magnification.
    pub magnification: f64,
    /// The objective and the light.
    pub optics: OpticsModel,
    /// The noise and the fixed pattern.
    pub noise: SensorModel,
}

impl Camera {
    /// A camera of `width` × `height` pixels, its sensor seeded with `seed`.
    pub fn new((width, height): (u32, u32), seed: u64) -> Self {
        Self {
            sensor: (width, height),
            pitch: 3.76,
            magnification: 10.0,
            optics: OpticsModel::default(),
            noise: SensorModel {
                seed,
                ..SensorModel::default()
            },
        }
    }

    /// A pixel's size on the sample, in µm.
    pub fn pixel(&self) -> f64 {
        self.pitch / self.magnification
    }

    /// The frame's width and height on the sample, in µm.
    pub fn field(&self) -> [f64; 2] {
        [self.sensor.0, self.sensor.1].map(|n| f64::from(n) * self.pixel())
    }

    /// A frame with the stage truly at `pose`.
    pub fn shot(&self, pose: StagePose) -> Shot {
        let scale = 1.0 / self.pixel();
        Shot {
            pose,
            calibration: Affine2::scale(scale, scale),
            sensor: self.sensor,
            settings: CameraSettings {
                roi: Roi::full(self.sensor.0, self.sensor.1),
                binning: Binning::NONE,
                pixel_format: PixelFormat::Mono14,
                exposure: Millis(10.0),
                gain: 1.0,
                trigger: TriggerMode::Software,
            },
        }
    }

    /// Frame `seq`'s metadata, with the pose the stage reported: `reported`.
    pub fn meta(&self, seq: u64, reported: StagePose) -> FrameMeta {
        let settings = self.shot(reported).settings;
        FrameMeta {
            seq: FrameSeq(seq),
            device_ts: None,
            host_ts_mid_exposure: Timestamp(seq * 100_000_000),
            exposure: settings.exposure,
            gain: settings.gain,
            roi: settings.roi,
            binning: settings.binning,
            pixel_format: settings.pixel_format,
            pose: Some(reported),
            pose_quality: PoseQuality::Settled,
            channel: None,
            objective: None,
            calibration: None,
            flags: FrameFlags {
                save: true,
                moving: false,
            },
        }
    }

    /// Frame `seq` of `world`, with the stage truly at `pose`.
    pub fn render(&self, world: &dyn World, pose: StagePose, seq: u64) -> anyhow::Result<Frame> {
        let mut frame = OwnedImage::new(self.sensor.0 as usize, self.sensor.1 as usize);
        let (shot, mut scratch) = (self.shot(pose), Scratch::default());
        let mut out = frame.view_mut();
        form_image(
            world,
            &self.optics,
            &self.noise,
            &shot,
            seq,
            &mut scratch,
            &mut out,
        )?;
        Ok(frame)
    }

    /// Frame `seq` of a noiseless `signal` (the world's intensity, from 0 to 1), with its noise and
    /// offsets, as the camera digitizes it.
    pub fn digitize(&self, signal: ImageView<'_, f32>, seq: u64) -> anyhow::Result<Frame> {
        let mut frame = OwnedImage::new(self.sensor.0 as usize, self.sensor.1 as usize);
        let shot = self.shot(StagePose::default());
        digitize(
            &signal,
            &self.optics,
            &self.noise,
            &shot,
            seq,
            &mut frame.view_mut(),
        )?;
        Ok(frame)
    }
}

/// A frame of 14-bit pixels.
pub type Frame = OwnedImage<u16>;

/// A new synthetic dataset named `name`, its plan the generator's `args`, with its `truth/`
/// folder: both.
pub fn dataset(
    out: &Output,
    name: &str,
    args: serde_json::Value,
) -> anyhow::Result<(Dataset, PathBuf)> {
    let mut new = NewDataset::new(DatasetKind::Synthetic, name);
    new.plan = Some(serde_json::json!({ "generator": name, "seed": out.seed, "args": args }));
    let dataset = Dataset::create(&out.data_root, new)?;
    let truth = dataset.path().join("truth");
    std::fs::create_dir_all(&truth).with_context(|| truth.display().to_string())?;
    Ok((dataset, truth))
}

/// Writes `value` as `<truth>/<name>.json`.
pub fn write_truth(truth: &Path, name: &str, value: &serde_json::Value) -> anyhow::Result<()> {
    let path = truth.join(format!("{name}.json"));
    let text = serde_json::to_string_pretty(value)?;
    std::fs::write(&path, text).with_context(|| path.display().to_string())
}

/// Frame `p` of a tile scan, or plane `z` of a stack, as the recorder keys it.
pub fn key(z: u32, p: u32) -> FrameKey {
    FrameKey { t: 0, c: 0, z, p }
}

/// Writes `frame` into `dataset`'s `raw/`, as it was taken at `reported`.
pub fn record(
    dataset: &Dataset,
    camera: &Camera,
    (key, seq): (FrameKey, u64),
    reported: StagePose,
    frame: &Frame,
) -> anyhow::Result<()> {
    let pixels: Vec<u16> = frame.view().rows().flatten().copied().collect();
    write_frame(dataset, key, camera.meta(seq, reported), &pixels)?;
    Ok(())
}

/// Writes `planes` as the image `zarr/<name>.ome.zarr`, a focus plane each, `dz` µm apart, its
/// first pixel's centre at `origin` (`z`, `y`, `x`), and builds its pyramid: its folder.
pub fn write_stack(
    dataset: &Dataset,
    camera: &Camera,
    (name, planes): (&str, &[Frame]),
    origin: [f64; 3],
    dz: f64,
) -> anyhow::Result<PathBuf> {
    let first = planes.first().context("a stack needs a plane")?;
    let (width, height) = (first.view().width() as u64, first.view().height() as u64);
    let path = dataset.zarr_dir().join(format!("{name}.ome.zarr"));
    let info = ImageInfo {
        name: name.to_owned(),
        sample: SampleType::U16,
        size: Size5 {
            t: 1,
            c: 1,
            z: planes.len() as u64,
            y: height,
            x: width,
        },
        levels: 1,
        chunk: CHUNK,
        pixel_size: [dz.max(1e-3), camera.pixel(), camera.pixel()],
        origin,
        channels: vec![ChannelInfo {
            label: name.to_owned(),
            colour: [255; 3],
            window: (0.0, 12_000.0),
        }],
    };
    let mut image = ZarrImage::create(&path, info)?;
    for (z, plane) in planes.iter().enumerate() {
        image.write_plane(0, (0, 0, z as u64), plane.view())?;
    }
    siq_pyramid::build(&mut image, siq_pyramid::TOP, &())?;
    Ok(path)
}

/// A standard normal number for `index` and `axis` of `seed`: always the same for the same three.
pub fn gaussian(seed: u64, index: u64, axis: u64) -> f64 {
    let mix = |mut z: u64| {
        // SplitMix64's finaliser.
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    };
    let unit = |salt: u64| {
        let bits = mix(seed ^ mix(index.wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ mix(axis ^ salt)));
        ((bits >> 11) as f64 + 0.5) / (1u64 << 53) as f64
    };
    // Box–Muller.
    (-2.0 * unit(1).ln()).sqrt() * (std::f64::consts::TAU * unit(2)).cos()
}

/// µm, as the stage takes them.
pub fn um(value: f64) -> Micrometers {
    Micrometers(value)
}
