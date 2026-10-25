// REQ-DATA-001: the manifest's round trip, its version check, and its atomic write (ADR-0041).
use std::io::{self, Write};

use super::*;

fn manifest() -> Manifest {
    Manifest {
        schema: SCHEMA.to_owned(),
        schema_version: SCHEMA_VERSION,
        id: DatasetId::generate(),
        kind: DatasetKind::TileScan,
        name: "kidney".to_owned(),
        created: Utc::now(),
        user: "lucas".to_owned(),
        provenance: Provenance {
            software: "spectrumiq".to_owned(),
            version: "0.1.0".to_owned(),
            host: "lab-pc".to_owned(),
        },
        instrument: Some(
            Snapshot::of(&serde_json::json!({ "camera": { "width": 11648 } })).unwrap(),
        ),
        calibration: None,
        plan: None,
        history: Vec::new(),
    }
}

#[test]
fn a_manifest_reads_back_as_written_and_refuses_another_format_or_a_newer_version() {
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("manifest.json");
    let written = manifest();
    written.write(&path).unwrap();
    assert_eq!(Manifest::read(&path).unwrap(), written);

    let newer = Manifest {
        schema_version: SCHEMA_VERSION + 1,
        ..written.clone()
    };
    newer.write(&path).unwrap();
    assert!(matches!(
        Manifest::read(&path),
        Err(StorageError::NotADataset { .. })
    ));
    let other = Manifest {
        schema: "napari".to_owned(),
        ..written
    };
    other.write(&path).unwrap();
    assert!(matches!(
        Manifest::read(&path),
        Err(StorageError::NotADataset { .. })
    ));
}

#[test]
fn a_snapshot_s_hash_follows_its_content() {
    let a = Snapshot::of(&serde_json::json!({ "x": 1, "y": [2, 3] })).unwrap();
    let b = Snapshot::of(&serde_json::json!({ "x": 1, "y": [2, 3] })).unwrap();
    let c = Snapshot::of(&serde_json::json!({ "x": 1, "y": [2, 4] })).unwrap();
    assert_eq!(a.sha256, b.sha256);
    assert_ne!(a.sha256, c.sha256);
    assert_eq!(a.sha256.len(), 64);
    // The SHA-256 of `{}`, as `sha256sum` gives it.
    let empty = Snapshot::of(&serde_json::json!({})).unwrap();
    assert_eq!(
        empty.sha256,
        "44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a"
    );
}

#[test]
fn an_interrupted_write_leaves_the_previous_manifest_intact() {
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("manifest.json");
    let first = manifest();
    first.write(&path).unwrap();
    let before = std::fs::read(&path).unwrap();

    // Half a manifest is written, then the disk fails.
    let failed = write_atomically(&path, |out| {
        out.write_all(br#"{"schema": "spectrumiq.dat"#)?;
        out.flush()?;
        Err(io::Error::from(io::ErrorKind::StorageFull))
    });
    assert_eq!(failed.unwrap_err().kind(), io::ErrorKind::StorageFull);
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert_eq!(Manifest::read(&path).unwrap(), first);
    let left: Vec<_> = std::fs::read_dir(folder.path()).unwrap().collect();
    assert_eq!(left.len(), 1, "no temporary file is left");
}
