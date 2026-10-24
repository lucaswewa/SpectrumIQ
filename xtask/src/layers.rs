//! The crate layering of architecture §3.1, checked against `cargo metadata`.
//!
//! - A crate may depend only on crates in its own layer or below.
//! - Processing (L3) never depends on Devices (L2), so every algorithm also runs offline.
//! - Only `siq-server` depends on teta-wot (ADR-0003).
//! - Apps and tooling may depend on anything. Nothing depends on them.
//!
//! Normal and build dependencies are checked. Dev-dependencies are exempt, since tests and
//! examples may use anything, such as simulators to make test data.

#[cfg(test)]
mod tests;

use std::fmt;

use crate::metadata::Metadata;

/// A layer of architecture §3.1, lowest first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Layer {
    /// L0: types, configuration and telemetry.
    Foundation,
    /// L1: synchronous core libraries, without devices or tokio.
    Core,
    /// L2: device traits, drivers and simulators.
    Devices,
    /// L3: pure processing algorithms.
    Processing,
    /// L4: async engines.
    Engines,
    /// L5: the service.
    Service,
    /// Binaries.
    App,
    /// Development tooling, outside the architecture.
    Tooling,
}

impl Layer {
    /// Whether a crate in this layer may depend on a crate in `other`.
    pub fn may_depend_on(self, other: Layer) -> bool {
        use Layer::*;
        match (self, other) {
            (App | Tooling, _) => true,
            (_, App | Tooling) => false,
            (Processing, Devices) => false,
            _ => other <= self,
        }
    }
}

impl fmt::Display for Layer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Layer::Foundation => "L0 Foundation",
            Layer::Core => "L1 Core",
            Layer::Devices => "L2 Devices",
            Layer::Processing => "L3 Processing",
            Layer::Engines => "L4 Engines",
            Layer::Service => "L5 Service",
            Layer::App => "App",
            Layer::Tooling => "Tooling",
        })
    }
}

/// Every crate of the architecture, with its layer.
///
/// A new crate is added here, and to architecture §3.1, in the phase that creates it.
pub const TABLE: &[(&str, Layer)] = &[
    ("siq-types", Layer::Foundation),
    ("siq-config", Layer::Foundation),
    ("siq-telemetry", Layer::Foundation),
    ("siq-buffer", Layer::Core),
    ("siq-imgproc", Layer::Core),
    ("siq-gpu", Layer::Core),
    ("siq-synth", Layer::Core),
    ("siq-storage", Layer::Core),
    ("siq-register", Layer::Core),
    ("siq-mfenc", Layer::Core),
    ("siq-shm", Layer::Core),
    ("siq-devices", Layer::Devices),
    ("siq-serial", Layer::Devices),
    ("siq-grbl", Layer::Devices),
    ("siq-ascii", Layer::Devices),
    ("siq-euresys-sys", Layer::Devices),
    ("siq-euresys", Layer::Devices),
    ("siq-sim", Layer::Devices),
    ("siq-pyramid", Layer::Processing),
    ("siq-calib", Layer::Processing),
    ("siq-stitch-live", Layer::Processing),
    ("siq-stitch", Layer::Processing),
    ("siq-dff", Layer::Processing),
    ("siq-deconv", Layer::Processing),
    ("siq-section", Layer::Processing),
    ("siq-lightfield", Layer::Processing),
    ("siq-acquire", Layer::Engines),
    ("siq-preview", Layer::Engines),
    ("siq-jobs", Layer::Engines),
    ("siq-workflow", Layer::Engines),
    ("siq-auth", Layer::Engines),
    ("siq-server", Layer::Service),
    ("spectrumiq-server", Layer::App),
    ("spectrumiq-desktop", Layer::App),
    ("spectrumiq-viewer3d", Layer::App),
    ("siq-datagen", Layer::App),
    ("xtask", Layer::Tooling),
];

/// The one crate that may depend on teta-wot.
pub const TETA_WOT_USER: &str = "siq-server";

/// The layer of a crate in [`TABLE`].
pub fn layer_of(name: &str) -> Option<Layer> {
    TABLE
        .iter()
        .find(|(crate_name, _)| *crate_name == name)
        .map(|(_, layer)| *layer)
}

/// A broken layering rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Violation {
    /// A workspace crate that [`TABLE`] doesn't list.
    Unlisted(String),
    /// A dependency the layers don't allow.
    Dependency {
        /// The crate that depends.
        from: String,
        /// Its layer.
        from_layer: Layer,
        /// The crate it depends on.
        to: String,
        /// That crate's layer.
        to_layer: Layer,
    },
    /// A dependency on teta-wot from a crate other than [`TETA_WOT_USER`].
    TetaWot(String),
}

impl fmt::Display for Violation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Violation::Unlisted(name) => write!(
                f,
                "`{name}` has no layer: add it to xtask/src/layers.rs and architecture §3.1"
            ),
            Violation::Dependency {
                from,
                from_layer,
                to,
                to_layer,
            } => write!(
                f,
                "`{from}` ({from_layer}) may not depend on `{to}` ({to_layer})"
            ),
            Violation::TetaWot(name) => write!(
                f,
                "`{name}` depends on teta-wot, which only `{TETA_WOT_USER}` may (ADR-0003)"
            ),
        }
    }
}

/// Every broken rule in the workspace's dependencies.
pub fn check(metadata: &Metadata) -> Vec<Violation> {
    let mut violations = Vec::new();
    for package in &metadata.packages {
        let Some(from_layer) = layer_of(&package.name) else {
            violations.push(Violation::Unlisted(package.name.clone()));
            continue;
        };
        for dependency in package.dependencies.iter().filter(|d| !d.is_dev()) {
            let name = &dependency.name;
            if (name == "teta-wot" || name.starts_with("teta-wot-"))
                && package.name != TETA_WOT_USER
            {
                violations.push(Violation::TetaWot(package.name.clone()));
            }
            if let Some(to_layer) = layer_of(name)
                && !from_layer.may_depend_on(to_layer)
            {
                violations.push(Violation::Dependency {
                    from: package.name.clone(),
                    from_layer,
                    to: name.clone(),
                    to_layer,
                });
            }
        }
    }
    violations
}
