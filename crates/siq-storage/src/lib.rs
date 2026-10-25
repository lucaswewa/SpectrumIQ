//! Datasets on disk (ADR-0041): the folder layout of architecture §8.1 with its manifest, the raw
//! frame recorder, the frames read back, and the catalog of a data root.
//!
//! - A [`Dataset`] is a folder under `<data_root>/datasets/`, described by its [`Manifest`]:
//!   versioned, with snapshots of what made it and its processing history, written atomically.
//! - A [`DatasetWriter`] records every frame a lossless subscriber gets, the frames marked to
//!   save, into the dataset's `raw/` folder: unbuffered and write-through, straight from the frame
//!   pool's page-aligned slot. It won't start, or go on, with too little free space.
//! - A [`DatasetReader`] lists the frames recorded, and maps each one as an `ImageView`.
//! - A [`Catalog`] indexes the datasets of a data root.
//! - [`zarr`] reads and writes OME-Zarr images (ADR-0042): the master format of everything
//!   derived from the frames.
//! - [`ometiff`] exports an OME-Zarr image as OME-TIFF (ADR-0046), for Fiji, QuPath and other
//!   tools.
//!
//! The unsafe code is in two small modules, listed in the README: `disk` (the free space, from
//! Windows) and `raw::map` (a recorded frame mapped read-only).

mod catalog;
mod dataset;
mod disk;
mod manifest;
pub mod ometiff;
mod raw;
pub mod zarr;

use std::io;
use std::path::{Path, PathBuf};

pub use catalog::{Catalog, DatasetSummary};
pub use dataset::{DATASETS, Dataset, MANIFEST, NewDataset};
pub use disk::free_space;
pub use manifest::{
    DatasetKind, Manifest, ProcessingRecord, ProcessingState, Provenance, SCHEMA, SCHEMA_VERSION,
    Snapshot,
};
pub use raw::{
    DatasetReader, DatasetWriter, DirectIo, FrameKey, FrameWrite, MappedFrame, RawFrame, Sidecar,
    WriterOptions, WriterReport, WriterStats,
};

/// Where long work on images reports its progress, and learns whether to stop: a pyramid's build,
/// or an export.
pub trait Control: Sync {
    /// The share done, from 0 to 1.
    fn progress(&self, done: f64);
    /// Whether to stop.
    fn cancelled(&self) -> bool;
}

/// Nobody to report to, and no stopping.
impl Control for () {
    fn progress(&self, _: f64) {}

    fn cancelled(&self) -> bool {
        false
    }
}

/// Why storage failed.
#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    /// A file or folder couldn't be read or written.
    #[error("{}: {source}", path.display())]
    Io {
        /// The file or folder.
        path: PathBuf,
        /// What went wrong.
        source: io::Error,
    },
    /// A JSON file doesn't hold what it should.
    #[error("{}: {source}", path.display())]
    Json {
        /// The file.
        path: PathBuf,
        /// What is wrong with it.
        source: serde_json::Error,
    },
    /// The disk has too little free space to start, or to write the next frame.
    #[error("{} has {free} bytes free, and {needed} are needed", path.display())]
    LowDiskSpace {
        /// The folder whose disk it is.
        path: PathBuf,
        /// The bytes free.
        free: u64,
        /// The bytes needed: the next frame and the space to keep free.
        needed: u64,
    },
    /// An OME-Zarr image couldn't be made, read or written.
    #[error("{}: {why}", path.display())]
    Zarr {
        /// The image's folder.
        path: PathBuf,
        /// What went wrong.
        why: String,
    },
    /// A TIFF file couldn't be written.
    #[error("{}: {why}", path.display())]
    Tiff {
        /// The file.
        path: PathBuf,
        /// What went wrong.
        why: String,
    },
    /// The work was asked to stop, and what it wrote is removed.
    #[error("cancelled")]
    Cancelled,
    /// A folder isn't a dataset this version can read.
    #[error("{} isn't a dataset this version reads: {why}", path.display())]
    NotADataset {
        /// The folder, or its manifest.
        path: PathBuf,
        /// Why.
        why: String,
    },
}

/// An I/O error at `path`, as a [`StorageError`].
fn io_at(path: &Path) -> impl FnOnce(io::Error) -> StorageError + '_ {
    move |source| StorageError::Io {
        path: path.to_owned(),
        source,
    }
}

/// A JSON error in `path`, as a [`StorageError`].
fn json_at(path: &Path) -> impl FnOnce(serde_json::Error) -> StorageError + '_ {
    move |source| StorageError::Json {
        path: path.to_owned(),
        source,
    }
}
