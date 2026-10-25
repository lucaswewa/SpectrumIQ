//! The jobs the service runs (ADR-0043 to ADR-0046): a dataset's pyramid, built through the
//! scheduler, then its DZI and OME-TIFF exports, each recorded. REQ-DATA-003, REQ-DATA-004,
//! REQ-DATA-005.

use std::time::Duration;

use siq_jobs::kinds::{DziExportJob, OmeTiffExportJob, PyramidJob};
use siq_jobs::*;
use siq_pyramid::tiles::{DziGeometry, TileFormat};
use siq_storage::ometiff::TiffCompression;
use siq_storage::zarr::{ChannelInfo, ImageInfo, SampleType, Size5, ZarrImage};
use siq_storage::{Dataset, DatasetKind, NewDataset, ProcessingState};
use siq_types::{OwnedImage, RectPx};

#[test]
fn a_dataset_s_pyramid_is_built_as_a_job_and_recorded() {
    let root = tempfile::tempdir().unwrap();
    let dataset = Dataset::create(root.path(), NewDataset::new(DatasetKind::Snap, "snap")).unwrap();
    let path = dataset.zarr_dir().join("snap.ome.zarr");
    let info = ImageInfo {
        name: "snap".to_owned(),
        sample: SampleType::U16,
        size: Size5 {
            t: 1,
            c: 1,
            z: 1,
            y: 700,
            x: 1000,
        },
        levels: 1,
        chunk: 256,
        pixel_size: [1.0, 0.376, 0.376],
        origin: [0.0; 3],
        channels: vec![ChannelInfo {
            label: "white".to_owned(),
            colour: [255; 3],
            window: (0.0, 16383.0),
        }],
    };
    let image = ZarrImage::create(&path, info).unwrap();
    for (row, col) in (0..3).flat_map(|row| (0..4).map(move |col| (row, col))) {
        let (w, h) = (256.min(1000 - col * 256), 256.min(700 - row * 256));
        let chunk = OwnedImage::from_fn(w as usize, h as usize, |_, _| 1000u16);
        image
            .write_chunk(0, (0, 0, 0), (row, col), chunk.view())
            .unwrap();
    }

    let job = PyramidJob::new(&path);
    assert!(job.estimate().ram > 0);
    let jobs = Scheduler::new(SchedulerConfig::new(14_000_000_000, 0), None).unwrap();
    let id = jobs.submit(
        Box::new(job),
        Priority::Interactive,
        Some(dataset.path().to_owned()),
    );
    let info = jobs.wait(id, Duration::from_secs(30)).unwrap();
    assert_eq!(
        (info.state, info.progress),
        (JobState::Done, 1.0),
        "{info:?}"
    );

    // 1000 → 500 → 250: three levels, the top within 256.
    let image = ZarrImage::open(&path).unwrap();
    assert_eq!(image.info().levels, 3);
    let top = image
        .read_region::<u16>(2, (0, 0, 0), RectPx::new(0, 0, 250, 175))
        .unwrap();
    assert!(top.pixels.iter().all(|&p| p == 1000));
    let reopened = Dataset::open(dataset.path()).unwrap();
    let record = &reopened.manifest().history[0];
    assert_eq!(
        (record.kind.as_str(), record.state),
        ("pyramid", ProcessingState::Done)
    );
    assert_eq!(record.outputs, ["zarr/snap.ome.zarr"]);
    assert_eq!(record.params["top"], 256);

    // Then its DZI export: every level's tiles, and the descriptor.
    let out = dataset.exports_dir().join("dzi");
    let export = DziExportJob {
        image: path.clone(),
        plane: (0, 0, 0),
        lut: None,
        format: TileFormat::Jpeg,
        out: out.clone(),
        name: "snap".to_owned(),
    };
    let id = jobs.submit(
        Box::new(export),
        Priority::Background,
        Some(dataset.path().to_owned()),
    );
    assert_eq!(
        jobs.wait(id, Duration::from_secs(30)).unwrap().state,
        JobState::Done
    );
    let geometry = DziGeometry {
        width: 1000,
        height: 700,
    };
    let mut written = 0;
    for level in 0..=geometry.max_level() {
        let (cols, rows) = geometry.tiles(level);
        for (col, row) in (0..rows).flat_map(|row| (0..cols).map(move |col| (col, row))) {
            let tile =
                std::fs::read(out.join(format!("snap_files/{level}/{col}_{row}.jpeg"))).unwrap();
            assert_eq!(&tile[..2], [0xFF, 0xD8]);
            written += 1;
        }
    }
    assert_eq!(
        written,
        4 * 3 + 2 * 2 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1,
        "levels 10 to 0"
    );
    let descriptor = std::fs::read_to_string(out.join("snap.dzi")).unwrap();
    assert!(descriptor.contains(r#"<Size Width="1000" Height="700"/>"#));
    let reopened = Dataset::open(dataset.path()).unwrap();
    assert_eq!(
        reopened.manifest().history[1].outputs,
        ["exports/dzi/snap.dzi"]
    );

    // And its OME-TIFF export: level 0, its two other levels, and the dataset in the OME-XML.
    let export = OmeTiffExportJob {
        image: path.clone(),
        out: dataset.exports_dir().join("snap.ome.tif"),
        compression: TiffCompression::Deflate,
    };
    assert!(export.estimate().ram > 0);
    let id = jobs.submit(
        Box::new(export),
        Priority::Background,
        Some(dataset.path().to_owned()),
    );
    assert_eq!(
        jobs.wait(id, Duration::from_secs(30)).unwrap().state,
        JobState::Done
    );
    let file = std::fs::File::open(dataset.exports_dir().join("snap.ome.tif")).unwrap();
    let mut decoder = tiff::decoder::Decoder::new(file).unwrap();
    assert_eq!(decoder.dimensions().unwrap(), (1000, 700));
    let tiff::decoder::DecodingResult::U16(pixels) = decoder.read_image().unwrap() else {
        panic!("not 16-bit");
    };
    assert!(pixels.iter().all(|&p| p == 1000));
    let xml = decoder.get_tag_ascii_string(tiff::tags::Tag::ImageDescription);
    assert!(xml.unwrap().contains(&dataset.manifest().id.to_string()));
    let subifds = decoder.get_tag_u64_vec(tiff::tags::Tag::SubIfd).unwrap();
    assert_eq!(subifds.len(), 2);
    let reopened = Dataset::open(dataset.path()).unwrap();
    let record = &reopened.manifest().history[2];
    assert_eq!(
        (record.kind.as_str(), record.outputs.as_slice()),
        (
            "export_ome_tiff",
            ["exports/snap.ome.tif".to_owned()].as_slice()
        )
    );
}
