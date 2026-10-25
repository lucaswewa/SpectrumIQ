//! A dataset's folder (architecture §8.1, ADR-0041):
//!
//! ```text
//! <data_root>/datasets/<yyyymmdd-hhmmss>_<slug>_<id8>/
//!   manifest.json   raw/   zarr/   exports/   qa/
//! ```

use std::fs;
use std::path::{Path, PathBuf};

use chrono::Utc;
use serde_json::Value;
use siq_types::DatasetId;

use crate::manifest::{DatasetKind, Manifest, Provenance, SCHEMA, SCHEMA_VERSION, Snapshot};
use crate::{StorageError, io_at};

/// The folder of a data root that holds its datasets.
pub const DATASETS: &str = "datasets";
/// A dataset's manifest, in its folder.
pub const MANIFEST: &str = "manifest.json";

/// The folders every dataset has.
const FOLDERS: [&str; 4] = ["raw", "zarr", "exports", "qa"];
/// The longest slug of a name, in a folder's name.
const SLUG_MAX: usize = 40;

/// What a new dataset is: the manifest's fields that its maker decides.
#[derive(Debug, Clone, PartialEq)]
pub struct NewDataset {
    /// What it will hold.
    pub kind: DatasetKind,
    /// Its name.
    pub name: String,
    /// Who makes it: the computer's user if not given.
    pub user: Option<String>,
    /// The instrument's configuration.
    pub instrument: Option<Snapshot>,
    /// The calibration in force.
    pub calibration: Option<Snapshot>,
    /// The plan it will follow.
    pub plan: Option<Value>,
}

impl NewDataset {
    /// A dataset of `kind` called `name`, with nothing else known.
    pub fn new(kind: DatasetKind, name: impl Into<String>) -> Self {
        Self {
            kind,
            name: name.into(),
            user: None,
            instrument: None,
            calibration: None,
            plan: None,
        }
    }
}

/// A dataset: its folder, and its manifest.
#[derive(Debug, Clone)]
pub struct Dataset {
    path: PathBuf,
    manifest: Manifest,
}

impl Dataset {
    /// Makes a new dataset under `<data_root>/datasets/`, with its folders and its manifest.
    pub fn create(data_root: &Path, new: NewDataset) -> Result<Self, StorageError> {
        let id = DatasetId::generate();
        let created = Utc::now();
        let folder = format!(
            "{}_{}_{}",
            created.format("%Y%m%d-%H%M%S"),
            slug(&new.name, new.kind),
            id.short()
        );
        let path = data_root.join(DATASETS).join(folder);
        for sub in FOLDERS {
            let folder = path.join(sub);
            fs::create_dir_all(&folder).map_err(io_at(&folder))?;
        }
        let manifest = Manifest {
            schema: SCHEMA.to_owned(),
            schema_version: SCHEMA_VERSION,
            id,
            kind: new.kind,
            name: new.name,
            created,
            user: new.user.unwrap_or_else(|| env_or("USERNAME", "local")),
            provenance: Provenance {
                software: "spectrumiq".to_owned(),
                version: env!("CARGO_PKG_VERSION").to_owned(),
                host: env_or("COMPUTERNAME", "unknown"),
            },
            instrument: new.instrument,
            calibration: new.calibration,
            plan: new.plan,
            history: Vec::new(),
        };
        manifest.write(&path.join(MANIFEST))?;
        Ok(Self { path, manifest })
    }

    /// The dataset in folder `path`.
    pub fn open(path: impl Into<PathBuf>) -> Result<Self, StorageError> {
        let path = path.into();
        let manifest = Manifest::read(&path.join(MANIFEST))?;
        Ok(Self { path, manifest })
    }

    /// Its folder.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Its manifest, as last written.
    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    /// The raw frames' folder.
    pub fn raw_dir(&self) -> PathBuf {
        self.path.join("raw")
    }

    /// The OME-Zarr folder.
    pub fn zarr_dir(&self) -> PathBuf {
        self.path.join("zarr")
    }

    /// The exports' folder: OME-TIFF, DZI.
    pub fn exports_dir(&self) -> PathBuf {
        self.path.join("exports")
    }

    /// The quality reports' folder.
    pub fn qa_dir(&self) -> PathBuf {
        self.path.join("qa")
    }

    /// Changes the manifest, and writes it atomically. If writing fails, it is left unchanged.
    pub fn update(&mut self, change: impl FnOnce(&mut Manifest)) -> Result<(), StorageError> {
        let mut next = self.manifest.clone();
        change(&mut next);
        next.write(&self.path.join(MANIFEST))?;
        self.manifest = next;
        Ok(())
    }
}

/// `name` for a folder's name: lowercase letters and digits, with a `-` for each run of anything
/// else, at most [`SLUG_MAX`] long. The kind's name if nothing is left.
fn slug(name: &str, kind: DatasetKind) -> String {
    let mut slug = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c.to_ascii_lowercase());
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
    }
    slug.truncate(SLUG_MAX);
    let slug = slug.trim_end_matches('-');
    if slug.is_empty() {
        kind.name().replace('_', "-")
    } else {
        slug.to_owned()
    }
}

fn env_or(name: &str, default: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| default.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_becomes_a_short_lowercase_slug() {
        assert_eq!(
            slug("Kidney  section #3", DatasetKind::Snap),
            "kidney-section-3"
        );
        assert_eq!(slug("  ¡DAPI/GFP!  ", DatasetKind::Snap), "dapi-gfp");
        assert_eq!(slug("µ", DatasetKind::TileScan), "tile-scan");
        assert_eq!(slug(&"a".repeat(60), DatasetKind::Snap).len(), SLUG_MAX);
    }
}
