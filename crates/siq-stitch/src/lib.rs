//! Post-acquisition stitching (REQ-STI-002): a scan's tiles placed by their stage poses, their
//! overlaps registered pairwise, the placement solved globally, and the mosaic composited out of
//! core into OME-Zarr.
//!
//! - **The tiles** ([`Mosaic`]) are placed by their poses through the calibration, on the camera's
//!   pixel grid: rotation, scale and shear are the calibration's, and the tiles differ by
//!   translations alone (ADR-0013).
//! - **The overlap graph** joins the tiles whose nominal rectangles overlap by a minimum each way.
//! - **Each pair** ([`pairs`]) is registered by phase correlation of the two tiles' nominal overlap,
//!   flat-corrected, its search reaching only as far as the stage's uncertainty allows, into an
//!   [`Edge`]: the measured offset and how sure it is (ADR-0061).
//! - **The pixels** come from a [`TileSource`]: a dataset's raw frames, mapped read-only, or images
//!   in memory.

mod tiles;

pub mod pairs;

use siq_storage::{FrameKey, StorageError};
use siq_types::ImageError;

pub use pairs::{Edge, PairOptions};
pub use tiles::{MappedTiles, Mosaic, Tile, TileSource};

/// Why a mosaic couldn't be stitched.
#[derive(Debug, thiserror::Error)]
pub enum StitchError {
    /// A frame has no stage pose to place it by.
    #[error("frame {0} has no stage pose")]
    NoPose(FrameKey),
    /// The frames aren't binned alike.
    #[error("the frames must share a binning")]
    Binnings,
    /// The source has no such tile.
    #[error("there is no tile {0}")]
    NoTile(usize),
    /// A tile's pixels aren't 16-bit words.
    #[error("tile {0}'s pixels aren't 16-bit")]
    NotWords(usize),
    /// The dataset couldn't be read.
    #[error(transparent)]
    Storage(#[from] StorageError),
    /// A view didn't fit its image.
    #[error(transparent)]
    Image(#[from] ImageError),
}
