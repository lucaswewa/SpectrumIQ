//! The catalog of a data root: its datasets, indexed in memory from their manifests. It is scanned
//! once, and told about each dataset as it is made or written to.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{PoisonError, RwLock};

use chrono::{DateTime, Utc};
use siq_types::DatasetId;

use crate::dataset::{DATASETS, Dataset};
use crate::manifest::DatasetKind;
use crate::{StorageError, io_at};

/// What the catalog knows of a dataset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatasetSummary {
    /// Its id.
    pub id: DatasetId,
    /// What it holds.
    pub kind: DatasetKind,
    /// Its name.
    pub name: String,
    /// When it was made.
    pub created: DateTime<Utc>,
    /// Who made it.
    pub user: String,
    /// Its folder.
    pub path: PathBuf,
    /// The raw frames recorded in it.
    pub frames: usize,
}

/// The datasets of a data root.
#[derive(Debug)]
pub struct Catalog {
    data_root: PathBuf,
    index: RwLock<BTreeMap<DatasetId, DatasetSummary>>,
}

impl Catalog {
    /// The catalog of `data_root`, scanned now. Without a `datasets` folder, it is empty. A folder
    /// that isn't a readable dataset is left out, with a warning.
    pub fn scan(data_root: impl Into<PathBuf>) -> Result<Self, StorageError> {
        let data_root = data_root.into();
        let index = RwLock::new(scan(&data_root)?);
        Ok(Self { data_root, index })
    }

    /// The data root.
    pub fn data_root(&self) -> &Path {
        &self.data_root
    }

    /// Scans the data root again.
    pub fn refresh(&self) -> Result<(), StorageError> {
        let index = scan(&self.data_root)?;
        *self.index.write().unwrap_or_else(PoisonError::into_inner) = index;
        Ok(())
    }

    /// Reads the dataset in folder `path` again, as it is now: once made, or written to.
    pub fn record(&self, path: &Path) -> Result<(), StorageError> {
        let summary = summarize(path)?;
        let mut index = self.index.write().unwrap_or_else(PoisonError::into_inner);
        index.insert(summary.id, summary);
        Ok(())
    }

    /// Every dataset, newest first.
    pub fn list(&self) -> Vec<DatasetSummary> {
        let index = self.index.read().unwrap_or_else(PoisonError::into_inner);
        // Ids are UUID v7, ordered by when they were made.
        index.values().rev().cloned().collect()
    }

    /// The dataset `id`.
    pub fn get(&self, id: DatasetId) -> Option<DatasetSummary> {
        let index = self.index.read().unwrap_or_else(PoisonError::into_inner);
        index.get(&id).cloned()
    }

    /// Deletes dataset `id`: its folder and everything in it, then its entry. False if the catalog
    /// doesn't know it. A folder that is only partly deleted stays in the catalog.
    pub fn delete(&self, id: DatasetId) -> Result<bool, StorageError> {
        let Some(summary) = self.get(id) else {
            return Ok(false);
        };
        fs::remove_dir_all(&summary.path).map_err(io_at(&summary.path))?;
        let mut index = self.index.write().unwrap_or_else(PoisonError::into_inner);
        index.remove(&id);
        Ok(true)
    }
}

fn scan(data_root: &Path) -> Result<BTreeMap<DatasetId, DatasetSummary>, StorageError> {
    let root = data_root.join(DATASETS);
    let entries = match fs::read_dir(&root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(error) => return Err(io_at(&root)(error)),
    };
    let mut index = BTreeMap::new();
    for entry in entries {
        let path = entry.map_err(io_at(&root))?.path();
        if !path.is_dir() {
            continue;
        }
        match summarize(&path) {
            Ok(summary) => {
                index.insert(summary.id, summary);
            }
            Err(error) => tracing::warn!(%error, "a folder of the data root was left out"),
        }
    }
    Ok(index)
}

fn summarize(path: &Path) -> Result<DatasetSummary, StorageError> {
    let dataset = Dataset::open(path)?;
    let raw = dataset.raw_dir();
    let frames = match fs::read_dir(&raw) {
        Ok(entries) => entries
            .filter_map(Result::ok)
            .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
            .count(),
        Err(error) if error.kind() == io::ErrorKind::NotFound => 0,
        Err(error) => return Err(io_at(&raw)(error)),
    };
    let manifest = dataset.manifest();
    Ok(DatasetSummary {
        id: manifest.id,
        kind: manifest.kind,
        name: manifest.name.clone(),
        created: manifest.created,
        user: manifest.user.clone(),
        path: path.to_owned(),
        frames,
    })
}
