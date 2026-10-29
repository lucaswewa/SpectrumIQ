//! A camera's feature names (ADR-0070). Where they differ from GenICam SFNC's, a TOML file for the
//! camera model says what to set, such as `config/cameras/adimec-d103a12c.toml`.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Deserialize;
use siq_devices::TriggerMode;
use siq_types::{Cfa, PixelFormat};

use crate::grabber::Module;

/// A feature, and the value it is set to. A feature is the camera's, or another module's with its
/// name first: `device:CameraControlMethod` is the grabber's camera controller.
pub type Setting = [String; 2];

/// How a camera's features are named and set. The default is GenICam SFNC's.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FeatureMap {
    /// The camera's `DeviceModelName`: the map is refused for another model. `None` for any.
    pub model: Option<String>,
    /// A colour sensor's filter array at its top left.
    pub cfa: Option<Cfa>,
    /// The sensor's pixel pitch, in µm: GenICam doesn't give it.
    pub pixel_size_um: f64,
    /// The camera's `PixelFormat` entries, with the formats they deliver.
    pub pixel_formats: BTreeMap<String, PixelFormat>,
    /// The exposure's feature, in µs.
    pub exposure: String,
    /// The gain's feature.
    pub gain: String,
    /// Whether the gain is in dB, as SFNC's is; otherwise it is a factor.
    pub gain_db: bool,
    /// The feature that sets the free-running frame period, in µs, if the camera has one.
    pub frame_period: Option<String>,
    /// What each trigger mode sets, in order.
    pub trigger: Triggers,
    /// The command that triggers one frame.
    pub trigger_command: String,
    /// The module of that command: the camera's, or the grabber's for its camera controller.
    pub trigger_module: Module,
    /// The readout's fixed part, in ms.
    pub readout_overhead_ms: f64,
    /// The readout's time a row, in µs.
    pub line_time_us: f64,
}

/// The features each trigger mode sets, in order.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Triggers {
    /// Free-running: live view.
    pub free_run: Vec<Setting>,
    /// A frame for each software trigger.
    pub software: Vec<Setting>,
    /// A frame for each signal on the trigger input.
    pub external: Vec<Setting>,
}

fn settings(pairs: &[(&str, &str)]) -> Vec<Setting> {
    let setting = |(name, value): &(&str, &str)| [(*name).to_owned(), (*value).to_owned()];
    pairs.iter().map(setting).collect()
}

impl Default for Triggers {
    fn default() -> Self {
        Self {
            free_run: settings(&[("TriggerMode", "Off")]),
            software: settings(&[("TriggerMode", "On"), ("TriggerSource", "Software")]),
            external: settings(&[("TriggerMode", "On"), ("TriggerSource", "Line0")]),
        }
    }
}

impl Default for FeatureMap {
    fn default() -> Self {
        let formats = [
            PixelFormat::Mono8,
            PixelFormat::Mono10,
            PixelFormat::Mono12,
            PixelFormat::Mono14,
            PixelFormat::Mono16,
        ];
        Self {
            model: None,
            cfa: None,
            pixel_size_um: 0.0,
            pixel_formats: formats.map(|f| (format!("{f:?}"), f)).into(),
            exposure: "ExposureTime".into(),
            gain: "Gain".into(),
            gain_db: true,
            frame_period: None,
            trigger: Triggers::default(),
            trigger_command: "TriggerSoftware".into(),
            trigger_module: Module::Remote,
            readout_overhead_ms: 0.0,
            line_time_us: 0.0,
        }
    }
}

/// Why a feature map couldn't be read.
#[derive(Debug, thiserror::Error)]
pub enum MapError {
    /// The file couldn't be read.
    #[error("reading {path}: {source}")]
    Read {
        /// The file.
        path: String,
        /// Why.
        source: std::io::Error,
    },
    /// The file isn't a feature map.
    #[error("{path}: {source}")]
    Parse {
        /// The file.
        path: String,
        /// Why.
        source: toml::de::Error,
    },
}

/// A [`Setting`]'s module, and its feature's name.
pub fn module_of(feature: &str) -> (Module, &str) {
    let modules = [
        ("interface:", Module::Interface),
        ("device:", Module::Device),
        ("stream:", Module::Stream),
        ("remote:", Module::Remote),
    ];
    let prefixed = modules
        .into_iter()
        .find_map(|(prefix, module)| feature.strip_prefix(prefix).map(|name| (module, name)));
    prefixed.unwrap_or((Module::Remote, feature))
}

impl FeatureMap {
    /// Reads the map in `path`.
    pub fn load(path: &Path) -> Result<Self, MapError> {
        let shown = path.display().to_string();
        let text = std::fs::read_to_string(path).map_err(|source| MapError::Read {
            path: shown.clone(),
            source,
        })?;
        toml::from_str(&text).map_err(|source| MapError::Parse {
            path: shown,
            source,
        })
    }

    /// What `mode` sets.
    pub fn trigger(&self, mode: TriggerMode) -> &[Setting] {
        match mode {
            TriggerMode::FreeRun => &self.trigger.free_run,
            TriggerMode::Software => &self.trigger.software,
            TriggerMode::External => &self.trigger.external,
        }
    }

    /// The camera's `PixelFormat` entry for `format`.
    pub fn entry(&self, format: PixelFormat) -> Option<&str> {
        (self.pixel_formats.iter())
            .find(|(_, delivered)| **delivered == format)
            .map(|(entry, _)| entry.as_str())
    }
}
