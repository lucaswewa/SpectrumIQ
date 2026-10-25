//! Recording frames into a dataset and reading them back (ADR-0041): the round trip of pixels and
//! metadata, page-aligned files and mappings, a failed write, the free space kept, and a frame
//! written from memory.
//! REQ-DATA-001, REQ-DATA-002.

mod common;

use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use common::{pixel, pool, publish, recorder};
use siq_buffer::{FrameLease, PAGE};
use siq_storage::*;

fn dataset(root: &Path) -> Dataset {
    Dataset::create(root, NewDataset::new(DatasetKind::Recording, "recording")).unwrap()
}

#[test]
fn the_frames_marked_to_save_come_back_as_recorded() {
    let root = tempfile::tempdir().unwrap();
    let dataset = dataset(root.path());
    let (pool, fanout) = pool(4);
    let writer = DatasetWriter::start(&dataset, recorder(&fanout), WriterOptions::new(0)).unwrap();
    for (seq, save) in [(10, true), (11, false), (12, true)] {
        drop(publish(&pool, &fanout, seq, save));
    }
    let report = writer.finish();
    assert!(report.error.is_none(), "{:?}", report.error);
    assert_eq!(report.stats.frames, 2);
    assert_eq!(
        report.stats.bytes,
        2 * 2 * PAGE as u64,
        "6000 bytes is two pages"
    );

    let reader = DatasetReader::open(&dataset).unwrap();
    let keys: Vec<String> = reader
        .frames()
        .iter()
        .map(|f| f.sidecar.key.to_string())
        .collect();
    assert_eq!(keys, ["t0000_c02_z000_p00000", "t0000_c02_z000_p00001"]);
    for (frame, seq) in reader.frames().iter().zip([10, 12]) {
        assert_eq!(frame.sidecar.bytes, 6000);
        assert_eq!(
            std::fs::metadata(frame.path()).unwrap().len(),
            2 * PAGE as u64
        );
        let mapped = frame.map().unwrap();
        assert_eq!(mapped.meta().seq.0, seq);
        assert_eq!(
            mapped.as_bytes().as_ptr().addr() % PAGE,
            0,
            "mapped page-aligned"
        );
        assert!(mapped.image_u8().is_none(), "Mono14 is 16-bit");
        let image = mapped.image_u16().unwrap();
        assert_eq!((image.width(), image.height()), (1000, 3));
        let pixels: Vec<u16> = image.rows().flatten().copied().collect();
        assert!(
            pixels
                .iter()
                .enumerate()
                .all(|(i, &value)| value == pixel(seq, i))
        );
    }
    // Every slot went back to the pool.
    assert!(pool.take_all_free().len() == 4);
}

/// Writes frames directly, until the `fail_at`th, which it half-writes and fails as a full disk.
struct FillsUp {
    written: u32,
    fail_at: u32,
}

impl FrameWrite for FillsUp {
    fn write(&mut self, path: &Path, frame: &FrameLease) -> io::Result<u64> {
        self.written += 1;
        if self.written < self.fail_at {
            return DirectIo.write(path, frame);
        }
        std::fs::write(path, &frame.as_bytes()[..100])?;
        Err(io::Error::from(io::ErrorKind::StorageFull))
    }
}

#[test]
fn a_full_disk_stops_the_recording_keeping_what_was_written() {
    let root = tempfile::tempdir().unwrap();
    let dataset = dataset(root.path());
    let (pool, fanout) = pool(6);
    let mut options = WriterOptions::new(0);
    options.io = Box::new(FillsUp {
        written: 0,
        fail_at: 3,
    });
    let writer = DatasetWriter::start(&dataset, recorder(&fanout), options).unwrap();
    for seq in 0..4 {
        drop(publish(&pool, &fanout, seq, true));
    }
    while !writer.is_finished() {
        std::thread::yield_now();
    }
    let report = writer.finish();
    assert_eq!(report.stats.frames, 2);
    let Some(StorageError::Io { path, source }) = report.error else {
        panic!("{:?}", report.error);
    };
    assert_eq!(source.kind(), io::ErrorKind::StorageFull);
    assert!(path.ends_with("t0000_c02_z000_p00002.raw"));
    assert!(!path.exists(), "the partial frame is removed");
    assert_eq!(DatasetReader::open(&dataset).unwrap().frames().len(), 2);
    // The recorder let go of the frames it hadn't written, and takes no more: only this frame,
    // held here, keeps a slot.
    let held = publish(&pool, &fanout, 9, true);
    assert_eq!(pool.take_all_free().len(), 5);
    drop(held);
}

static FREE: AtomicU64 = AtomicU64::new(0);

fn fake_free_space(_: &Path) -> io::Result<u64> {
    Ok(FREE.load(Ordering::SeqCst))
}

#[test]
fn the_writer_keeps_the_free_space_it_is_told_to() {
    let root = tempfile::tempdir().unwrap();
    let dataset = dataset(root.path());
    let (pool, fanout) = pool(4);
    let options = || {
        let mut options = WriterOptions::new(1 << 20);
        options.free_space = fake_free_space;
        options
    };
    FREE.store(1 << 19, Ordering::SeqCst);
    let refused = DatasetWriter::start(&dataset, recorder(&fanout), options()).unwrap_err();
    assert!(matches!(
        refused,
        StorageError::LowDiskSpace {
            free: 524_288,
            needed: 1_048_576,
            ..
        }
    ));

    // Room for a frame, and then not: the second would take the disk below the space to keep.
    FREE.store((1 << 20) + 3 * PAGE as u64, Ordering::SeqCst);
    let writer = DatasetWriter::start(&dataset, recorder(&fanout), options()).unwrap();
    drop(publish(&pool, &fanout, 0, true));
    while writer.stats().frames == 0 {
        std::thread::yield_now();
    }
    FREE.store((1 << 20) + PAGE as u64, Ordering::SeqCst);
    drop(publish(&pool, &fanout, 1, true));
    let report = writer.finish();
    assert_eq!(report.stats.frames, 1);
    assert!(matches!(
        report.error,
        Some(StorageError::LowDiskSpace { .. })
    ));
}

#[test]
fn the_free_space_is_the_disk_s() {
    let root = tempfile::tempdir().unwrap();
    assert!(free_space(root.path()).unwrap() > 0);
    assert!(free_space(&root.path().join("not there")).is_err());
}

#[test]
fn a_frame_made_without_the_camera_reads_back_as_a_recorded_one() {
    let root = tempfile::tempdir().unwrap();
    let dataset = dataset(root.path());
    let (pool, fanout) = pool(1);
    // A recorded frame's metadata, as the generator gives its own.
    let meta = *publish(&pool, &fanout, 7, true).meta();
    let (width, height) = meta.image_size();
    let pixels: Vec<u16> = (0..width * height).map(|i| (i % 4093) as u16).collect();
    let key = FrameKey {
        t: 0,
        c: 0,
        z: 3,
        p: 0,
    };
    let written = write_frame(&dataset, key, meta, &pixels).unwrap();
    assert_eq!(
        written.path(),
        dataset.raw_dir().join("t0000_c00_z003_p00000.raw")
    );

    let reader = DatasetReader::open(&dataset).unwrap();
    assert_eq!(reader.frames(), [written]);
    let mapped = reader.frames()[0].map().unwrap();
    assert_eq!(*mapped.meta(), meta);
    let image = mapped.image_u16().unwrap();
    assert_eq!(image.rows().flatten().copied().collect::<Vec<_>>(), pixels);
}
