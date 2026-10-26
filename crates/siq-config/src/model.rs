//! The configuration's sections, with their defaults.
//!
//! Every section denies unknown keys, so a typo is an error rather than a silently ignored line.
//! Device sections are tagged by `kind`; each kind has its own keys.

use std::net::{IpAddr, Ipv4Addr};
use std::path::PathBuf;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Serialize};
use siq_telemetry::LogConfig;
use siq_types::{ChannelId, Hertz, Micrometers, Millis, ObjectiveId, PixelFormat};

/// The modality unit's imaging modes, from `siq-types`, where the device traits use them too.
pub use siq_types::ModalityMode;

/// A SpectrumIQ instrument: its devices, optics and channels, and how the service runs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InstrumentConfig {
    /// Where the service listens and keeps its data.
    #[serde(default)]
    pub server: ServerConfig,
    /// The memory budget (architecture §5).
    #[serde(default)]
    pub memory: MemoryConfig,
    /// Where logs go.
    #[serde(default)]
    pub log: LogConfig,
    /// The camera.
    pub camera: CameraConfig,
    /// The XYZ stage.
    pub stage: StageConfig,
    /// The focus drive: the fine z axis.
    pub focus: FocusConfig,
    /// The objectives. There is at least one.
    pub objectives: Vec<ObjectiveConfig>,
    /// The channels. There is at least one.
    pub channels: Vec<ChannelConfig>,
    /// The illumination, filter wheel, objective turret and modality unit, where fitted.
    #[serde(default)]
    pub peripherals: PeripheralsConfig,
    /// The simulation, for the devices of kind `sim`.
    #[serde(default)]
    pub sim: SimConfig,
}

/// Where the service listens and keeps its data. Relative paths are relative to the
/// configuration file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct ServerConfig {
    /// The address to listen on: `127.0.0.1` for this computer only, `0.0.0.0` for the LAN.
    pub bind: IpAddr,
    /// The TCP port.
    pub port: u16,
    /// The folder of the datasets.
    pub data_root: PathBuf,
    /// Scratch space for raw frames, on a fast NVMe drive.
    pub scratch: PathBuf,
    /// The settings the service keeps: the calibrations, versioned (ADR-0049).
    pub settings: PathBuf,
    /// The disk space to keep free where datasets are recorded, in GB: recording won't start
    /// below it, and stops before a frame would go below it (ADR-0041).
    pub min_free_gb: f64,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
            port: 5000,
            data_root: "data".into(),
            scratch: "scratch".into(),
            settings: "settings".into(),
            min_free_gb: 20.0,
        }
    }
}

/// The memory budget, for the 32 GB / 8 GB workstation by default (ADR-0005).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct MemoryConfig {
    /// Frame-pool slots, each one full frame, allocated and locked at start-up.
    pub pool_slots: u16,
    /// RAM that processing jobs may use together, in GB.
    pub job_ram_gb: f64,
    /// GPU memory that processing jobs may use together, in GB.
    pub job_vram_gb: f64,
    /// The deep-zoom tile cache, in MB.
    pub tile_cache_mb: u32,
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            pool_slots: 12,
            job_ram_gb: 14.0,
            job_vram_gb: 4.0,
            tile_cache_mb: 1024,
        }
    }
}

/// The camera.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CameraConfig {
    /// The simulated camera.
    Sim(SimCameraConfig),
    /// A CoaXPress camera on a Euresys Coaxlink grabber.
    Euresys(EuresysCameraConfig),
}

/// The simulated camera. The defaults are a 100 MP, 14-bit sensor at 9 fps.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct SimCameraConfig {
    /// Sensor columns.
    pub width: u32,
    /// Sensor rows.
    pub height: u32,
    /// The pixel format delivered.
    pub pixel_format: PixelFormat,
    /// The full-frame rate.
    pub frame_rate: Hertz,
    /// The sensor's pixel pitch, in µm.
    pub pixel_size: Micrometers,
}

impl Default for SimCameraConfig {
    fn default() -> Self {
        Self {
            width: 11_648,
            height: 8_742,
            pixel_format: PixelFormat::Mono14,
            frame_rate: Hertz(9.0),
            pixel_size: Micrometers(3.76),
        }
    }
}

/// A camera on a Euresys Coaxlink grabber, through eGrabber.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct EuresysCameraConfig {
    /// The grabber's interface index.
    pub interface: u32,
    /// The device index on that interface.
    pub device: u32,
    /// The camera's feature-name map (P10.2), if it differs from GenICam SFNC.
    pub features: Option<PathBuf>,
}

/// The XYZ stage.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StageConfig {
    /// The GRBL driver, talking to the GRBL emulator.
    Sim(SimStageConfig),
    /// A GRBL or grblHAL controller on a serial port.
    Grbl(GrblStageConfig),
}

/// The simulated stage.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct SimStageConfig {
    /// How often the driver asks for the position.
    pub poll_hz: Hertz,
    /// How the driver moves the stage.
    pub motion: GrblMotion,
}

impl Default for SimStageConfig {
    fn default() -> Self {
        Self {
            poll_hz: poll_rate(),
            motion: GrblMotion::default(),
        }
    }
}

/// A GRBL or grblHAL controller.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GrblStageConfig {
    /// The serial port, such as `"COM5"`.
    pub port: String,
    /// The baud rate.
    #[serde(default = "grbl_baud")]
    pub baud: u32,
    /// How often the driver asks for the position. GRBL suggests at most 5 Hz.
    #[serde(default = "poll_rate")]
    pub poll_hz: Hertz,
    /// How the driver moves the stage.
    #[serde(default)]
    pub motion: GrblMotion,
}

/// How the GRBL driver moves the stage (P1.9).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct GrblMotion {
    /// Unlock with `$X` when the controller starts locked, rather than wait for homing.
    pub unlock: bool,
    /// The direction each axis, x, y and z, arrives from in a backlash-consistent move: 1 or -1.
    pub approach: [i8; 3],
    /// How far a backlash-consistent move overshoots a target it would otherwise reach the wrong
    /// way.
    pub overshoot: Micrometers,
}

impl Default for GrblMotion {
    fn default() -> Self {
        Self {
            unlock: true,
            approach: [1, 1, 1],
            overshoot: Micrometers(50.0),
        }
    }
}

fn grbl_baud() -> u32 {
    115_200
}

fn poll_rate() -> Hertz {
    Hertz(5.0)
}

/// The focus drive.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FocusConfig {
    /// The ASCII driver, talking to an emulator made from the same dialect.
    Sim(SimFocusConfig),
    /// A focus drive with an ASCII protocol on a serial port.
    Ascii(AsciiFocusConfig),
}

/// The simulated focus drive.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct SimFocusConfig {
    /// The dialect to emulate; the example dialect if none.
    pub dialect: Option<PathBuf>,
}

/// A focus drive with an ASCII protocol.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AsciiFocusConfig {
    /// The serial port, such as `"COM6"`.
    pub port: String,
    /// The baud rate.
    #[serde(default = "ascii_baud")]
    pub baud: u32,
    /// The dialect file that describes its protocol (P1.10a).
    pub dialect: PathBuf,
}

fn ascii_baud() -> u32 {
    9_600
}

/// An objective.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ObjectiveConfig {
    /// Its id, unique among the objectives. It stays the same when the list is reordered.
    pub id: ObjectiveId,
    /// Its name, unique among the objectives, such as `"10x/0.30"`.
    pub name: String,
    /// The magnification.
    pub magnification: f64,
    /// The numerical aperture.
    pub numerical_aperture: f64,
    /// The immersion medium.
    #[serde(default)]
    pub immersion: Immersion,
    /// Its position on the objective turret, from 0, if it is on one.
    #[serde(default)]
    pub turret_position: Option<u8>,
    /// How far focus moves when switching to it from the reference objective.
    #[serde(default)]
    pub parfocal_offset: Micrometers,
}

/// An objective's immersion medium.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Immersion {
    /// Dry.
    #[default]
    Air,
    /// Water.
    Water,
    /// Oil.
    Oil,
    /// Glycerol.
    Glycerol,
}

impl Immersion {
    /// The medium's refractive index.
    pub const fn refractive_index(self) -> f64 {
        match self {
            Immersion::Air => 1.0,
            Immersion::Water => 1.333,
            Immersion::Oil => 1.518,
            Immersion::Glycerol => 1.473,
        }
    }
}

/// A channel: the illumination, filter and exposure an image is taken with.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChannelConfig {
    /// Its id, unique among the channels. It stays the same when the list is reordered.
    pub id: ChannelId,
    /// Its name, unique among the channels, such as `"DAPI"`.
    pub name: String,
    /// The light source and intensity, if the instrument has illumination.
    #[serde(default)]
    pub illumination: Option<ChannelIllumination>,
    /// The filter, by name, if the instrument has a filter wheel.
    #[serde(default)]
    pub filter: Option<String>,
    /// The exposure time.
    pub exposure: Millis,
    /// The colour the channel is shown in.
    #[serde(default)]
    pub colour: Colour,
}

/// A channel's light source, and its intensity.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChannelIllumination {
    /// The source, by name, from `peripherals.illumination.sources`.
    pub source: String,
    /// The intensity, from 0 to 1.
    pub intensity: f64,
}

/// A display colour, written `"#rrggbb"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Colour(pub [u8; 3]);

impl Default for Colour {
    fn default() -> Self {
        Self([255; 3])
    }
}

impl TryFrom<String> for Colour {
    type Error = String;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        let hex = text
            .strip_prefix('#')
            .filter(|hex| hex.len() == 6 && hex.bytes().all(|b| b.is_ascii_hexdigit()));
        let hex = hex.ok_or_else(|| format!("{text:?} is not a colour: write \"#rrggbb\""))?;
        let byte = |at: usize| u8::from_str_radix(&hex[at..at + 2], 16).expect("hex digits");
        Ok(Self([byte(0), byte(2), byte(4)]))
    }
}

impl From<Colour> for String {
    fn from(Colour([r, g, b]): Colour) -> Self {
        format!("#{r:02x}{g:02x}{b:02x}")
    }
}

impl JsonSchema for Colour {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "Colour".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "description": "A display colour, written \"#rrggbb\".",
            "type": "string",
            "pattern": "^#[0-9a-fA-F]{6}$"
        })
    }
}

/// The peripherals, each optional: absent means not fitted.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct PeripheralsConfig {
    /// The light sources.
    pub illumination: Option<IlluminationConfig>,
    /// The filter wheel.
    pub filter_wheel: Option<FilterWheelConfig>,
    /// The objective turret.
    pub objective_turret: Option<TurretConfig>,
    /// The light-field, confocal or structured-illumination unit.
    pub modality: Option<ModalityConfig>,
}

/// How a peripheral is driven. Only simulators exist until the hardware is chosen (P10.5).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PeripheralKind {
    /// Simulated.
    #[default]
    Sim,
}

/// The light sources.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IlluminationConfig {
    /// How it is driven.
    #[serde(default)]
    pub kind: PeripheralKind,
    /// The sources' names, such as `["white", "470nm"]`.
    pub sources: Vec<String>,
}

/// The filter wheel.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FilterWheelConfig {
    /// How it is driven.
    #[serde(default)]
    pub kind: PeripheralKind,
    /// The filters' names, by position from 0. Use `"empty"` for an empty position.
    pub filters: Vec<String>,
}

/// The objective turret.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TurretConfig {
    /// How it is driven.
    #[serde(default)]
    pub kind: PeripheralKind,
    /// The number of positions.
    pub positions: u8,
}

/// The modality unit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ModalityConfig {
    /// How it is driven.
    #[serde(default)]
    pub kind: PeripheralKind,
    /// The modes it offers.
    pub modes: Vec<ModalityMode>,
}

/// The simulation (ADR-0012).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct SimConfig {
    /// The seed of every random choice, so runs repeat.
    pub seed: u64,
    /// The full sensor, or a smaller one for fast UI work.
    pub profile: SimProfile,
    /// What the simulated camera looks at.
    pub world: WorldConfig,
    /// The errors injected into the simulated stage.
    pub errors: SimErrors,
    /// How the simulated camera sits on the microscope: what the stage–camera calibration finds.
    pub mount: SimMount,
}

/// How the simulated camera sits: the image turned, scaled and sheared against the stage, as a
/// real mount and real optics leave it. The stage–camera calibration measures it (P5.2).
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct SimMount {
    /// The camera's rotation, in degrees.
    pub rotation: f64,
    /// The magnification's relative error: 0.001 is 0.1 % more pixels per µm.
    pub scale_error: f64,
    /// How far the image's second axis leans along its first.
    pub shear: f64,
}

/// How much the simulated camera renders.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SimProfile {
    /// The configured sensor.
    #[default]
    Full,
    /// A quarter of the sensor's pixels, for quick work on the UI.
    Fast,
}

/// The simulated specimen (P1.4).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum WorldConfig {
    /// Cells, for transmitted light and fluorescence.
    #[default]
    Cells,
    /// Grains, for reflected light.
    Grains,
    /// A relief of known height, for depth from focus.
    HeightField,
    /// Fluorescent beads in a volume, for deconvolution and light field.
    Beads,
    /// A real image, laid out at a pixel size.
    Image {
        /// The image file (16-bit PNG or TIFF).
        path: PathBuf,
        /// The size of one image pixel on the sample.
        pixel_size: Micrometers,
    },
}

/// The errors injected into the simulated stage, so stitching has something to correct.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct SimErrors {
    /// Lost motion when an axis reverses.
    pub backlash: Micrometers,
    /// Relative scale error of the axes.
    pub scale_error: f64,
    /// Random positioning error (standard deviation).
    pub position_noise: Micrometers,
    /// How long the controller takes to reply.
    pub reply_latency: Millis,
}

impl Default for SimErrors {
    fn default() -> Self {
        Self {
            backlash: Micrometers(2.0),
            scale_error: 0.0005,
            position_noise: Micrometers(0.5),
            reply_latency: Millis(5.0),
        }
    }
}
