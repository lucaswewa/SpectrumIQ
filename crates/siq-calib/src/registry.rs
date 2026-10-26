//! The calibrations, versioned (ADR-0049): one JSON file per calibration, never changed or
//! removed, `<settings>/<kind>/<key>/<created>_<id8>.json`, with any data it holds beside it as
//! `<created>_<id8>.<name>`. A key's newest is the one in force; the others stay, so that frames
//! taken under them can be read again.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{PoisonError, RwLock};

use chrono::{DateTime, Utc};
use serde::Serialize;
use serde::de::DeserializeOwned;
use siq_types::CalibrationId;

use crate::{CalibError, CalibrationRecord};

/// A kind of calibration the registry keeps.
pub trait Versioned: Clone + Serialize + DeserializeOwned {
    /// What a calibration is in force for, such as its objective.
    type Key: PartialEq;
    /// The kind's folder in the settings folder.
    const FOLDER: &'static str;

    /// Its id.
    fn id(&self) -> CalibrationId;
    /// When it was made.
    fn created(&self) -> DateTime<Utc>;
    /// What it is in force for.
    fn key(&self) -> Self::Key;
    /// Its key's folder, in the kind's.
    fn key_folder(&self) -> String;
}

/// Every calibration of one kind in a settings folder, read when opened and kept as each is saved.
#[derive(Debug)]
pub struct Registry<R> {
    dir: PathBuf,
    records: RwLock<Vec<R>>,
}

/// The stage–camera calibrations.
pub type CalibrationRegistry = Registry<CalibrationRecord>;

impl Versioned for CalibrationRecord {
    type Key = siq_types::ObjectiveId;
    const FOLDER: &'static str = "calibrations";

    fn id(&self) -> CalibrationId {
        self.id
    }

    fn created(&self) -> DateTime<Utc> {
        self.created
    }

    fn key(&self) -> Self::Key {
        self.objective
    }

    fn key_folder(&self) -> String {
        format!("objective-{}", self.objective)
    }
}

pub(crate) fn storage(path: &Path, error: impl std::fmt::Display) -> CalibError {
    CalibError::Storage(format!("{}: {error}", path.display()))
}

impl<R: Versioned> Registry<R> {
    /// The calibrations under `settings`. A file that isn't one is left out, with a warning.
    pub fn open(settings: &Path) -> Result<Self, CalibError> {
        let dir = settings.join(R::FOLDER);
        let mut records: Vec<R> = Vec::new();
        let folders = match fs::read_dir(&dir) {
            Ok(folders) => folders
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .collect(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(error) => return Err(storage(&dir, error)),
        };
        for folder in folders {
            let files = fs::read_dir(&folder).map_err(|error| storage(&folder, error))?;
            for path in files.filter_map(Result::ok).map(|entry| entry.path()) {
                if path.extension().is_none_or(|extension| extension != "json") {
                    continue;
                }
                let read = fs::read_to_string(&path).map_err(|error| storage(&path, error));
                match read.and_then(|text| {
                    serde_json::from_str::<R>(&text).map_err(|e| storage(&path, e))
                }) {
                    Ok(record) => records.push(record),
                    Err(error) => tracing::warn!(%error, "a calibration was left out"),
                }
            }
        }
        records.sort_by_key(R::created);
        Ok(Self {
            dir,
            records: RwLock::new(records),
        })
    }

    /// Where `record`'s file is, or with `name`, the data beside it.
    pub fn path(&self, record: &R, name: &str) -> PathBuf {
        let stem = format!(
            "{}_{}",
            record.created().format("%Y%m%d-%H%M%S"),
            record.id().short()
        );
        let folder = self.dir.join(record.key_folder());
        folder.join(format!("{stem}.{name}"))
    }

    /// Keeps `record`, written whole before it counts: it is in force for its key from now. Its
    /// data, `(name, bytes)` each, is written first.
    pub fn save(&self, record: &R, data: &[(&str, &[u8])]) -> Result<PathBuf, CalibError> {
        let path = self.path(record, "json");
        let folder = path.parent().unwrap_or(&self.dir);
        fs::create_dir_all(folder).map_err(|error| storage(folder, error))?;
        let text = serde_json::to_string_pretty(record).map_err(|error| storage(&path, error))?;
        let files = data
            .iter()
            .map(|&(name, bytes)| (self.path(record, name), bytes));
        for (target, bytes) in files.chain([(path.clone(), text.as_bytes())]) {
            let partial = target.with_extension("part");
            fs::write(&partial, bytes).map_err(|error| storage(&partial, error))?;
            fs::rename(&partial, &target).map_err(|error| storage(&target, error))?;
        }
        let mut records = self.records.write().unwrap_or_else(PoisonError::into_inner);
        records.push(record.clone());
        records.sort_by_key(R::created);
        Ok(path)
    }

    /// The calibration in force for `key`: its newest.
    pub fn current(&self, key: R::Key) -> Option<R> {
        let records = self.records.read().unwrap_or_else(PoisonError::into_inner);
        records
            .iter()
            .rev()
            .find(|record| record.key() == key)
            .cloned()
    }

    /// Every calibration of `key`, newest first.
    pub fn history(&self, key: R::Key) -> Vec<R> {
        let records = self.records.read().unwrap_or_else(PoisonError::into_inner);
        let mine = records.iter().rev().filter(|record| record.key() == key);
        mine.cloned().collect()
    }

    /// The calibration `id`, in force or not.
    pub fn get(&self, id: CalibrationId) -> Option<R> {
        let records = self.records.read().unwrap_or_else(PoisonError::into_inner);
        records.iter().find(|record| record.id() == id).cloned()
    }
}
