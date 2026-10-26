//! A dataset's manifest (`manifest.json`): what the dataset is, what made it, and what has been
//! done to it since. Its format is versioned, and it is written atomically.

use std::fs::{self, File};
use std::io::{self, BufWriter, Write};
use std::path::Path;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use siq_types::DatasetId;

use crate::{StorageError, io_at, json_at};

/// The manifest's format: its `schema` field.
pub const SCHEMA: &str = "spectrumiq.dataset";
/// The format's version. A reader refuses a newer one; a newer reader reads this one.
pub const SCHEMA_VERSION: u32 = 1;

/// What a dataset holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DatasetKind {
    /// A frame per channel, snapped.
    Snap,
    /// Frames recorded as they came, without a plan.
    Recording,
    /// The tiles of a mosaic.
    TileScan,
    /// A focus stack.
    ZStack,
    /// What a plan of the workflow engine acquired, of any other shape.
    Sequence,
    /// Made by `siq-datagen`, with its ground truth.
    Synthetic,
}

impl DatasetKind {
    /// Its name, as the manifest writes it: `tile_scan`.
    pub fn name(self) -> &'static str {
        match self {
            Self::Snap => "snap",
            Self::Recording => "recording",
            Self::TileScan => "tile_scan",
            Self::ZStack => "z_stack",
            Self::Sequence => "sequence",
            Self::Synthetic => "synthetic",
        }
    }
}

/// A copy of something a dataset was made with, such as the instrument's configuration, with its
/// SHA-256: two datasets with the same hash were made with the same thing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    /// The hash of [`value`](Self::value)'s compact JSON, in lowercase hex.
    pub sha256: String,
    /// The copy.
    pub value: Value,
}

impl Snapshot {
    /// A snapshot of `value`. Its hash is of the compact JSON, in the order serde writes it.
    pub fn of(value: &impl Serialize) -> serde_json::Result<Self> {
        let value = serde_json::to_value(value)?;
        let digest = Sha256::digest(serde_json::to_vec(&value)?);
        let sha256 = digest.iter().map(|byte| format!("{byte:02x}")).collect();
        Ok(Self { sha256, value })
    }
}

/// What made a dataset: the program, its version, and the computer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Provenance {
    /// The program: `spectrumiq`, or `siq-datagen`.
    pub software: String,
    /// Its version.
    pub version: String,
    /// The computer's name.
    pub host: String,
}

/// How a step of processing went.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessingState {
    /// Waiting to run.
    Queued,
    /// Running.
    Running,
    /// Finished.
    Done,
    /// Ended with an error.
    Failed,
    /// Cancelled before it finished.
    Cancelled,
}

/// A step of processing on a dataset, such as a job (P4.3): what ran, on what, and how it went.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProcessingRecord {
    /// Its id, unique in the dataset.
    pub id: String,
    /// What ran: `pyramid`, `stitch`, ….
    pub kind: String,
    /// The version of what ran.
    pub version: String,
    /// Its parameters.
    #[serde(default)]
    pub params: Value,
    /// What it read, as paths in the dataset.
    #[serde(default)]
    pub inputs: Vec<String>,
    /// What it wrote, as paths in the dataset.
    #[serde(default)]
    pub outputs: Vec<String>,
    /// How it went.
    pub state: ProcessingState,
    /// How far it got, from 0 to 1.
    #[serde(default)]
    pub progress: f64,
    /// When it started.
    pub started: Option<DateTime<Utc>>,
    /// When it finished, failed or was cancelled.
    pub finished: Option<DateTime<Utc>>,
    /// Why it failed.
    pub error: Option<String>,
}

/// A dataset's manifest.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    /// [`SCHEMA`].
    pub schema: String,
    /// [`SCHEMA_VERSION`] when written.
    pub schema_version: u32,
    /// The dataset's id.
    pub id: DatasetId,
    /// What it holds.
    pub kind: DatasetKind,
    /// Its name, as the user gave it.
    pub name: String,
    /// When it was made.
    pub created: DateTime<Utc>,
    /// Who made it: the signed-in user from M9, the computer's user until then.
    pub user: String,
    /// What made it.
    pub provenance: Provenance,
    /// The instrument's configuration when it was made.
    pub instrument: Option<Snapshot>,
    /// The stage–camera calibration in force (M5).
    pub calibration: Option<Snapshot>,
    /// The acquisition plan it followed (M6).
    pub plan: Option<Value>,
    /// What has been done to it since, oldest first.
    #[serde(default)]
    pub history: Vec<ProcessingRecord>,
}

impl Manifest {
    /// Reads the manifest at `path`, refusing another format, or a newer version of this one.
    pub fn read(path: &Path) -> Result<Self, StorageError> {
        let text = fs::read(path).map_err(io_at(path))?;
        let manifest: Self = serde_json::from_slice(&text).map_err(json_at(path))?;
        if manifest.schema != SCHEMA {
            let why = format!("its schema is {:?}, not {SCHEMA:?}", manifest.schema);
            return Err(not_a_dataset(path, why));
        }
        if manifest.schema_version > SCHEMA_VERSION {
            let why = format!(
                "it is version {}, and this reads up to {SCHEMA_VERSION}",
                manifest.schema_version
            );
            return Err(not_a_dataset(path, why));
        }
        Ok(manifest)
    }

    /// Writes the manifest to `path` atomically: to a temporary file beside it, flushed to disk,
    /// then renamed over it. A failure leaves the previous manifest as it was.
    pub fn write(&self, path: &Path) -> Result<(), StorageError> {
        write_atomically(path, |out| {
            serde_json::to_writer_pretty(&mut *out, self)?;
            out.write_all(b"\n")
        })
        .map_err(io_at(path))
    }
}

fn not_a_dataset(path: &Path, why: String) -> StorageError {
    StorageError::NotADataset {
        path: path.to_owned(),
        why,
    }
}

/// Writes a file through `write` atomically, as [`Manifest::write`] does.
pub(crate) fn write_atomically(
    path: &Path,
    write: impl FnOnce(&mut BufWriter<File>) -> io::Result<()>,
) -> io::Result<()> {
    let temporary = path.with_extension("json.tmp");
    let written = File::create(&temporary).and_then(|file| {
        let mut out = BufWriter::new(file);
        write(&mut out)?;
        out.into_inner()
            .map_err(io::IntoInnerError::into_error)?
            .sync_all()
    });
    let renamed = written.and_then(|()| fs::rename(&temporary, path));
    if renamed.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    renamed
}

#[cfg(test)]
mod tests;
