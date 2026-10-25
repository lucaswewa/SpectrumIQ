//! A dataset's folder and manifest (ADR-0041), and the catalog of a data root.
//! REQ-DATA-001, REQ-DATA-006.

mod common;

use std::fs;

use common::{pool, publish, recorder};
use siq_storage::*;

#[test]
fn a_dataset_has_its_folders_and_a_manifest_with_its_provenance() {
    let root = tempfile::tempdir().unwrap();
    let mut new = NewDataset::new(DatasetKind::TileScan, "Kidney section #3");
    new.user = Some("lucas".to_owned());
    new.instrument = Some(Snapshot::of(&serde_json::json!({ "camera": "sim" })).unwrap());
    let mut dataset = Dataset::create(root.path(), new).unwrap();

    let folder = dataset
        .path()
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned();
    let (stamp, rest) = folder.split_at(15);
    assert!(stamp.chars().enumerate().all(|(i, c)| if i == 8 {
        c == '-'
    } else {
        c.is_ascii_digit()
    }));
    assert_eq!(
        rest,
        format!("_kidney-section-3_{}", dataset.manifest().id.short())
    );
    assert_eq!(dataset.path().parent().unwrap(), root.path().join(DATASETS));
    for sub in ["raw", "zarr", "exports", "qa"] {
        assert!(dataset.path().join(sub).is_dir(), "{sub}");
    }

    let manifest = dataset.manifest().clone();
    assert_eq!(
        (manifest.schema.as_str(), manifest.schema_version),
        (SCHEMA, SCHEMA_VERSION)
    );
    assert_eq!(
        (manifest.kind, manifest.name.as_str()),
        (DatasetKind::TileScan, "Kidney section #3")
    );
    assert_eq!(manifest.user, "lucas");
    assert_eq!(manifest.provenance.software, "spectrumiq");
    assert_eq!(manifest.instrument.as_ref().unwrap().value["camera"], "sim");
    assert_eq!(Dataset::open(dataset.path()).unwrap().manifest(), &manifest);

    // Processing is recorded in the manifest, and kept.
    dataset
        .update(|manifest| {
            manifest.history.push(ProcessingRecord {
                id: "1".to_owned(),
                kind: "pyramid".to_owned(),
                version: "0.1.0".to_owned(),
                params: serde_json::json!({ "levels": 6 }),
                inputs: vec!["zarr/mosaic.ome.zarr/0".to_owned()],
                outputs: vec![],
                state: ProcessingState::Done,
                started: None,
                finished: None,
                error: None,
            })
        })
        .unwrap();
    let reopened = Dataset::open(dataset.path()).unwrap();
    assert_eq!(reopened.manifest().history[0].kind, "pyramid");
    assert_eq!(reopened.manifest(), dataset.manifest());
}

#[test]
fn the_catalog_lists_a_data_root_s_datasets_newest_first() {
    let root = tempfile::tempdir().unwrap();
    assert!(
        Catalog::scan(root.path()).unwrap().list().is_empty(),
        "no datasets folder yet"
    );

    let first = Dataset::create(root.path(), NewDataset::new(DatasetKind::Snap, "first")).unwrap();
    let second = Dataset::create(
        root.path(),
        NewDataset::new(DatasetKind::Recording, "second"),
    )
    .unwrap();
    // A folder that isn't a dataset, and one with a broken manifest, are left out.
    fs::create_dir_all(root.path().join(DATASETS).join("stray")).unwrap();
    let broken = root.path().join(DATASETS).join("broken");
    fs::create_dir_all(&broken).unwrap();
    fs::write(broken.join(MANIFEST), "{ not json").unwrap();

    let catalog = std::sync::Arc::new(Catalog::scan(root.path()).unwrap());
    let names: Vec<String> = catalog.list().into_iter().map(|d| d.name).collect();
    assert_eq!(names, ["second", "first"]);
    assert_eq!(catalog.get(first.manifest().id).unwrap().frames, 0);

    // Recording into a dataset updates its entry when the writer finishes.
    let (pool, fanout) = pool(4);
    let mut options = WriterOptions::new(0);
    options.catalog = Some(catalog.clone());
    let writer = DatasetWriter::start(&second, recorder(&fanout), options).unwrap();
    drop(publish(&pool, &fanout, 1, true));
    drop(publish(&pool, &fanout, 2, true));
    assert!(writer.finish().error.is_none());
    let entry = catalog.get(second.manifest().id).unwrap();
    assert_eq!(
        (entry.frames, entry.kind, entry.path.as_path()),
        (2, DatasetKind::Recording, second.path())
    );

    // A dataset made since the scan appears on refresh.
    let third =
        Dataset::create(root.path(), NewDataset::new(DatasetKind::ZStack, "third")).unwrap();
    assert!(catalog.get(third.manifest().id).is_none());
    catalog.refresh().unwrap();
    assert_eq!(catalog.list()[0].name, "third");
}
