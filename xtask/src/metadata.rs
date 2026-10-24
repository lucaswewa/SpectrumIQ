//! The parts of `cargo metadata` that the tasks read.

use std::path::Path;

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::process;

/// The workspace's packages, from `cargo metadata --no-deps`.
#[derive(Debug, Deserialize)]
pub struct Metadata {
    /// Every workspace member.
    pub packages: Vec<Package>,
}

/// A workspace member.
#[derive(Debug, Deserialize)]
pub struct Package {
    /// The package's name.
    pub name: String,
    /// What it depends on, as its manifest declares.
    #[serde(default)]
    pub dependencies: Vec<Dependency>,
    /// Its library, binaries, examples, tests and benches.
    #[serde(default)]
    pub targets: Vec<Target>,
}

/// A declared dependency.
#[derive(Debug, Deserialize)]
pub struct Dependency {
    /// The package depended on, whatever it is renamed to.
    pub name: String,
    /// `None` for a normal dependency, `"dev"` or `"build"` otherwise.
    #[serde(default)]
    pub kind: Option<String>,
}

impl Dependency {
    /// Whether it's a dev-dependency, which only tests, examples and benches use.
    pub fn is_dev(&self) -> bool {
        self.kind.as_deref() == Some("dev")
    }
}

/// A build target of a package.
#[derive(Debug, Deserialize)]
pub struct Target {
    /// The target's name.
    pub name: String,
    /// Its kinds, such as `lib`, `bin` or `example`.
    #[serde(default)]
    pub kind: Vec<String>,
    /// The features it needs to build.
    #[serde(default, rename = "required-features")]
    pub required_features: Vec<String>,
}

impl Metadata {
    /// Reads the metadata of the workspace at `repo`.
    pub fn load(repo: &Path) -> Result<Self> {
        let json = process::output(
            repo,
            "cargo",
            ["metadata", "--format-version", "1", "--no-deps"],
        )?;
        serde_json::from_str(&json).context("could not read `cargo metadata`'s output")
    }

    /// Every example of every package.
    pub fn examples(&self) -> impl Iterator<Item = (&Package, &Target)> {
        self.packages.iter().flat_map(|package| {
            package
                .targets
                .iter()
                .filter(|target| target.kind.iter().any(|kind| kind == "example"))
                .map(move |target| (package, target))
        })
    }
}
