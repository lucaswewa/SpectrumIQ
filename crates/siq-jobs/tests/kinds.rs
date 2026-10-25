//! The jobs the service runs (ADR-0043, ADR-0044): a dataset's pyramid, built through the
//! scheduler, and recorded. REQ-DATA-003, REQ-DATA-005.

use std::time::Duration;

use siq_jobs::kinds::PyramidJob;
use siq_jobs::*;
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
}
