//! The calibrations, versioned (ADR-0049): one JSON file per calibration, never changed or
//! removed, `<settings>/calibrations/objective-<id>/<created>_<id8>.json`. An objective's newest
//! is the one in force; the others stay, so that frames taken under them can be read again.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{PoisonError, RwLock};

use siq_types::{CalibrationId, ObjectiveId};

use crate::{CalibError, CalibrationRecord};

/// Every calibration of a settings folder, read when opened and kept as each is saved.
#[derive(Debug)]
pub struct CalibrationRegistry {
    dir: PathBuf,
    records: RwLock<Vec<CalibrationRecord>>,
}

fn storage(path: &Path, error: impl std::fmt::Display) -> CalibError {
    CalibError::Storage(format!("{}: {error}", path.display()))
}

impl CalibrationRegistry {
    /// The calibrations under `settings`. A file that isn't one is left out, with a warning.
    pub fn open(settings: &Path) -> Result<Self, CalibError> {
        let dir = settings.join("calibrations");
        let mut records = Vec::new();
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
                    serde_json::from_str::<CalibrationRecord>(&text).map_err(|e| storage(&path, e))
                }) {
                    Ok(record) => records.push(record),
                    Err(error) => tracing::warn!(%error, "a calibration was left out"),
                }
            }
        }
        records.sort_by_key(|record| record.created);
        Ok(Self {
            dir,
            records: RwLock::new(records),
        })
    }

    /// Keeps `record`, written whole before it counts: it is in force for its objective from now.
    pub fn save(&self, record: &CalibrationRecord) -> Result<PathBuf, CalibError> {
        let folder = self.dir.join(format!("objective-{}", record.objective));
        fs::create_dir_all(&folder).map_err(|error| storage(&folder, error))?;
        let name = format!(
            "{}_{}.json",
            record.created.format("%Y%m%d-%H%M%S"),
            record.id.short()
        );
        let path = folder.join(name);
        let partial = path.with_extension("json.part");
        let text = serde_json::to_string_pretty(record).map_err(|error| storage(&path, error))?;
        fs::write(&partial, text).map_err(|error| storage(&partial, error))?;
        fs::rename(&partial, &path).map_err(|error| storage(&path, error))?;
        let mut records = self.records.write().unwrap_or_else(PoisonError::into_inner);
        records.push(record.clone());
        records.sort_by_key(|record| record.created);
        Ok(path)
    }

    /// The calibration in force for `objective`: its newest.
    pub fn current(&self, objective: ObjectiveId) -> Option<CalibrationRecord> {
        let records = self.records.read().unwrap_or_else(PoisonError::into_inner);
        records
            .iter()
            .rev()
            .find(|record| record.objective == objective)
            .cloned()
    }

    /// Every calibration of `objective`, newest first.
    pub fn history(&self, objective: ObjectiveId) -> Vec<CalibrationRecord> {
        let records = self.records.read().unwrap_or_else(PoisonError::into_inner);
        let mine = records
            .iter()
            .rev()
            .filter(|record| record.objective == objective);
        mine.cloned().collect()
    }

    /// The calibration `id`, in force or not.
    pub fn get(&self, id: CalibrationId) -> Option<CalibrationRecord> {
        let records = self.records.read().unwrap_or_else(PoisonError::into_inner);
        records.iter().find(|record| record.id == id).cloned()
    }
}
