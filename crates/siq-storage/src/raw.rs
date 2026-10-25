//! Raw frames (ADR-0041). A dataset's `raw/` folder holds each frame as two files, named by its
//! [`FrameKey`]:
//!
//! - `t0000_c00_z000_p00012.raw`: its pixels, little-endian and row-major, padded with whatever
//!   the pool's slot held to whole pages, as direct I/O writes them;
//! - `t0000_c00_z000_p00012.json`: its [`Sidecar`], written after the pixels, atomically. A frame
//!   without a sidecar wasn't finished.

mod map;

use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::os::windows::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use siq_buffer::{FrameLease, RecvError, Subscriber};
use siq_types::{FrameMeta, ImageView};
use windows_sys::Win32::Storage::FileSystem::{FILE_FLAG_NO_BUFFERING, FILE_FLAG_WRITE_THROUGH};

use crate::dataset::Dataset;
use crate::manifest::write_atomically;
use crate::{Catalog, StorageError, io_at, json_at};

/// How often a waiting writer checks whether it should stop.
const POLL: Duration = Duration::from_millis(50);

/// Where a frame belongs in a dataset: its time point, channel, focus plane and stage position.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct FrameKey {
    /// The time point.
    pub t: u32,
    /// The channel's id.
    pub c: u16,
    /// The focus plane.
    pub z: u32,
    /// The stage position: a tile, or the frame's number in a recording.
    pub p: u32,
}

impl fmt::Display for FrameKey {
    /// The files' name: `t0000_c00_z000_p00012`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Self { t, c, z, p } = self;
        write!(f, "t{t:04}_c{c:02}_z{z:03}_p{p:05}")
    }
}

/// A raw frame's sidecar: its key, its pixels' length in the raw file, and its metadata.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Sidecar {
    /// Where it belongs.
    pub key: FrameKey,
    /// The pixels' bytes, from the file's start; the padding follows them.
    pub bytes: u64,
    /// The frame's metadata.
    pub meta: FrameMeta,
}

/// How a [`DatasetWriter`] puts a frame's pixels on disk.
pub trait FrameWrite: Send {
    /// Writes `frame`'s pixels to `path`, which doesn't exist yet, and returns the bytes written.
    fn write(&mut self, path: &Path, frame: &FrameLease) -> io::Result<u64>;
}

/// Unbuffered, write-through I/O, straight from the frame's page-aligned slot: no copy, and
/// nothing left in the system's file cache.
#[derive(Debug, Clone, Copy, Default)]
pub struct DirectIo;

impl FrameWrite for DirectIo {
    fn write(&mut self, path: &Path, frame: &FrameLease) -> io::Result<u64> {
        let bytes = frame.as_padded_bytes();
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .custom_flags(FILE_FLAG_NO_BUFFERING | FILE_FLAG_WRITE_THROUGH)
            .open(path)?;
        file.write_all(bytes)?;
        Ok(bytes.len() as u64)
    }
}

/// How a [`DatasetWriter`] records.
pub struct WriterOptions {
    /// The bytes to leave free on the disk: the writer won't start below it, and stops before a
    /// frame would take the disk below it.
    pub min_free: u64,
    /// Each frame's key: by default, the order it came in, on its channel (`c`, or 0).
    pub key: Box<dyn FnMut(&FrameMeta) -> FrameKey + Send>,
    /// How the pixels are written: [`DirectIo`] by default.
    pub io: Box<dyn FrameWrite>,
    /// The free space on a folder's disk: [`free_space`](crate::free_space) by default.
    pub free_space: fn(&Path) -> io::Result<u64>,
    /// The catalog to record the dataset in when the writer finishes.
    pub catalog: Option<Arc<Catalog>>,
}

impl WriterOptions {
    /// The defaults, keeping `min_free` bytes free.
    pub fn new(min_free: u64) -> Self {
        let mut next = 0;
        Self {
            min_free,
            key: Box::new(move |meta| {
                let p = next;
                next += 1;
                FrameKey {
                    t: 0,
                    c: meta.channel.map_or(0, |channel| channel.0),
                    z: 0,
                    p,
                }
            }),
            io: Box::new(DirectIo),
            free_space: crate::free_space,
            catalog: None,
        }
    }
}

/// What a writer has recorded.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WriterStats {
    /// Frames written.
    pub frames: u64,
    /// Bytes written to raw files, padding included.
    pub bytes: u64,
    /// The time spent writing them.
    pub busy: Duration,
}

/// How a writer ended: what it wrote, and the error it stopped on, if any.
#[derive(Debug)]
pub struct WriterReport {
    /// What it wrote.
    pub stats: WriterStats,
    /// Why it stopped early.
    pub error: Option<StorageError>,
}

/// Records frames into a dataset's `raw/` folder, on a thread of its own (architecture §4.3).
pub struct DatasetWriter {
    stop: Arc<AtomicBool>,
    stats: Arc<Mutex<WriterStats>>,
    thread: Option<JoinHandle<Option<StorageError>>>,
}

impl DatasetWriter {
    /// Starts recording every frame `frames` gets into `dataset`: a lossless subscriber, whose
    /// filter takes the frames marked to save. Refuses to start with less than
    /// [`min_free`](WriterOptions::min_free) free.
    pub fn start(
        dataset: &Dataset,
        frames: Subscriber,
        options: WriterOptions,
    ) -> Result<Self, StorageError> {
        let dir = dataset.raw_dir();
        let free = (options.free_space)(&dir).map_err(io_at(&dir))?;
        if free < options.min_free {
            return Err(StorageError::LowDiskSpace {
                path: dir,
                free,
                needed: options.min_free,
            });
        }
        let stop = Arc::new(AtomicBool::new(false));
        let stats = Arc::new(Mutex::new(WriterStats::default()));
        let recorder = Recorder {
            dataset: dataset.path().to_owned(),
            dir,
            options,
            stats: Arc::clone(&stats),
        };
        let thread = thread::Builder::new()
            .name("dataset-writer".to_owned())
            .spawn({
                let stop = Arc::clone(&stop);
                move || recorder.run(frames, &stop)
            })
            .map_err(io_at(dataset.path()))?;
        Ok(Self {
            stop,
            stats,
            thread: Some(thread),
        })
    }

    /// What it has written so far.
    pub fn stats(&self) -> WriterStats {
        *self.stats.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Whether it has stopped, on an error or because the frames ended.
    pub fn is_finished(&self) -> bool {
        self.thread.as_ref().is_none_or(JoinHandle::is_finished)
    }

    /// Stops it: the frames already waiting are written, then it ends.
    pub fn finish(mut self) -> WriterReport {
        let error = self.join();
        WriterReport {
            stats: self.stats(),
            error,
        }
    }

    fn join(&mut self) -> Option<StorageError> {
        self.stop.store(true, Ordering::Release);
        let thread = self.thread.take()?;
        thread.join().unwrap_or_else(|_| {
            Some(StorageError::Io {
                path: PathBuf::from("dataset-writer"),
                source: io::Error::other("the writer thread panicked"),
            })
        })
    }
}

impl Drop for DatasetWriter {
    fn drop(&mut self) {
        let _ = self.join();
    }
}

impl fmt::Debug for DatasetWriter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DatasetWriter")
            .field("stats", &self.stats())
            .finish_non_exhaustive()
    }
}

/// The writer thread's state.
struct Recorder {
    dataset: PathBuf,
    dir: PathBuf,
    options: WriterOptions,
    stats: Arc<Mutex<WriterStats>>,
}

impl Recorder {
    /// Writes frames until told to stop and none are left, or the frames end, or a write fails.
    /// Returning drops the subscriber, so nothing waits on the recorder any more.
    fn run(mut self, mut frames: Subscriber, stop: &AtomicBool) -> Option<StorageError> {
        let error = loop {
            let frame = if stop.load(Ordering::Acquire) {
                frames.try_recv()
            } else {
                match frames.recv_timeout(POLL) {
                    Ok(frame) => Some(frame),
                    Err(RecvError::Timeout) => continue,
                    Err(RecvError::Closed) => None,
                }
            };
            let Some(frame) = frame else { break None };
            if let Err(error) = self.write(&frame) {
                tracing::error!(dataset = %self.dataset.display(), %error, "the recording stopped");
                break Some(error);
            }
        };
        if let Some(catalog) = &self.options.catalog
            && let Err(error) = catalog.record(&self.dataset)
        {
            tracing::warn!(%error, "the catalog wasn't updated");
        }
        error
    }

    fn write(&mut self, frame: &FrameLease) -> Result<(), StorageError> {
        let padded = frame.as_padded_bytes().len() as u64;
        let free = (self.options.free_space)(&self.dir).map_err(io_at(&self.dir))?;
        let needed = padded.saturating_add(self.options.min_free);
        if free < needed {
            return Err(StorageError::LowDiskSpace {
                path: self.dir.clone(),
                free,
                needed,
            });
        }
        let key = (self.options.key)(frame.meta());
        let raw = self.dir.join(format!("{key}.raw"));
        let started = Instant::now();
        let written = self.options.io.write(&raw, frame).map_err(|source| {
            // A partial frame isn't kept: without its sidecar, it would never be read.
            let _ = fs::remove_file(&raw);
            StorageError::Io {
                path: raw.clone(),
                source,
            }
        })?;
        let sidecar = Sidecar {
            key,
            bytes: frame.as_bytes().len() as u64,
            meta: *frame.meta(),
        };
        let json = self.dir.join(format!("{key}.json"));
        write_atomically(&json, |out| {
            serde_json::to_writer_pretty(out, &sidecar).map_err(io::Error::from)
        })
        .map_err(io_at(&json))?;
        let mut stats = self.stats.lock().unwrap_or_else(PoisonError::into_inner);
        stats.frames += 1;
        stats.bytes += written;
        stats.busy += started.elapsed();
        Ok(())
    }
}

/// Writes a frame of 16-bit pixels, made without the camera (such as by `siq-datagen`), into
/// `dataset`'s `raw/` folder as the recorder would: `<key>.raw`, little-endian, row after row,
/// then its sidecar.
pub fn write_frame(
    dataset: &Dataset,
    key: FrameKey,
    meta: FrameMeta,
    pixels: &[u16],
) -> Result<RawFrame, StorageError> {
    let dir = dataset.raw_dir();
    let raw = dir.join(format!("{key}.raw"));
    let bytes: Vec<u8> = pixels
        .iter()
        .flat_map(|pixel| pixel.to_le_bytes())
        .collect();
    fs::write(&raw, &bytes).map_err(io_at(&raw))?;
    let sidecar = Sidecar {
        key,
        bytes: bytes.len() as u64,
        meta,
    };
    let json = dir.join(format!("{key}.json"));
    write_atomically(&json, |out| {
        serde_json::to_writer_pretty(out, &sidecar).map_err(io::Error::from)
    })
    .map_err(io_at(&json))?;
    Ok(RawFrame { sidecar, path: raw })
}

/// A recorded frame: its sidecar, and its raw file.
#[derive(Debug, Clone, PartialEq)]
pub struct RawFrame {
    /// Its sidecar.
    pub sidecar: Sidecar,
    path: PathBuf,
}

impl RawFrame {
    /// Its raw file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Maps its raw file, read-only. While mapped, no program can change or delete the file.
    pub fn map(&self) -> Result<MappedFrame, StorageError> {
        let (file, map) = map::read_only(&self.path).map_err(io_at(&self.path))?;
        let bytes = usize::try_from(self.sidecar.bytes).unwrap_or(usize::MAX);
        if bytes > map.len() {
            return Err(StorageError::NotADataset {
                path: self.path.clone(),
                why: format!("it holds {} bytes, not {bytes}", map.len()),
            });
        }
        Ok(MappedFrame {
            _file: file,
            map,
            bytes,
            meta: self.sidecar.meta,
        })
    }
}

/// A recorded frame, mapped.
pub struct MappedFrame {
    // Held open, sharing reading only, for as long as the mapping lives.
    _file: fs::File,
    map: memmap2::Mmap,
    bytes: usize,
    meta: FrameMeta,
}

impl MappedFrame {
    /// The frame's metadata.
    pub fn meta(&self) -> &FrameMeta {
        &self.meta
    }

    /// The pixels' bytes, without the padding.
    pub fn as_bytes(&self) -> &[u8] {
        &self.map[..self.bytes]
    }

    /// The image of a frame of 16-bit words, such as Mono14; `None` for 8-bit pixels.
    pub fn image_u16(&self) -> Option<ImageView<'_, u16>> {
        if self.meta.pixel_format.bytes_per_pixel() != 2 {
            return None;
        }
        self.view(map::as_u16(self.as_bytes())?)
    }

    /// The image of a frame of 8-bit pixels; `None` for wider ones.
    pub fn image_u8(&self) -> Option<ImageView<'_, u8>> {
        if self.meta.pixel_format.bytes_per_pixel() != 1 {
            return None;
        }
        self.view(self.as_bytes())
    }

    fn view<'a, T>(&self, pixels: &'a [T]) -> Option<ImageView<'a, T>> {
        let (width, height) = self.meta.image_size();
        let width = width as usize;
        ImageView::new(pixels, width, height as usize, width).ok()
    }
}

impl fmt::Debug for MappedFrame {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MappedFrame")
            .field("bytes", &self.bytes)
            .field("meta", &self.meta)
            .finish_non_exhaustive()
    }
}

/// The frames recorded in a dataset, in key order.
#[derive(Debug, Clone, PartialEq)]
pub struct DatasetReader {
    frames: Vec<RawFrame>,
}

impl DatasetReader {
    /// Lists `dataset`'s recorded frames: those with a sidecar.
    pub fn open(dataset: &Dataset) -> Result<Self, StorageError> {
        let dir = dataset.raw_dir();
        let mut frames = Vec::new();
        for entry in fs::read_dir(&dir).map_err(io_at(&dir))? {
            let path = entry.map_err(io_at(&dir))?.path();
            if path.extension().is_none_or(|extension| extension != "json") {
                continue;
            }
            let text = fs::read(&path).map_err(io_at(&path))?;
            let sidecar: Sidecar = serde_json::from_slice(&text).map_err(json_at(&path))?;
            frames.push(RawFrame {
                sidecar,
                path: path.with_extension("raw"),
            });
        }
        frames.sort_by_key(|frame| frame.sidecar.key);
        Ok(Self { frames })
    }

    /// The frames, in key order: by time point, channel, plane, then position.
    pub fn frames(&self) -> &[RawFrame] {
        &self.frames
    }
}
