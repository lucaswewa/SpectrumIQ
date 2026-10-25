//! The synthetic datasets (ADR-0047), at smoke sizes: each loads as a dataset, and its frames agree
//! with its ground truth. A tile matches the ideal mosaic where the truth puts it, not where the
//! stage said; a z-stack is sharpest where the true surface is; each bead is brightest in the plane
//! at its depth. And the CLI writes one. REQ-SIM-004, REQ-STI-004.

use std::path::Path;

use serde_json::Value;
use siq_datagen::{GENERATORS, Generator, Output};
use siq_storage::zarr::ZarrImage;
use siq_storage::{Catalog, Dataset, DatasetKind, DatasetReader};
use siq_synth::GroundTruth;
use siq_types::RectPx;

/// Runs generator `name`, at smoke sizes, into `root`: the dataset.
fn generate(root: &Path, name: &str) -> Dataset {
    let generator: &dyn Generator = *GENERATORS
        .iter()
        .find(|generator| generator.command().get_name() == name)
        .unwrap();
    let args = generator.command().try_get_matches_from([name]).unwrap();
    let out = Output {
        data_root: root.to_owned(),
        smoke: true,
        seed: 7,
    };
    Dataset::open(generator.generate(&args, &out).unwrap()).unwrap()
}

fn truth(dataset: &Dataset, name: &str) -> Value {
    let text = std::fs::read_to_string(dataset.path().join("truth").join(name)).unwrap();
    serde_json::from_str(&text).unwrap()
}

/// The frames, each as its pixels.
fn frames(dataset: &Dataset) -> Vec<Vec<u16>> {
    let reader = DatasetReader::open(dataset).unwrap();
    let frames = reader.frames().iter();
    frames
        .map(|frame| {
            let mapped = frame.map().unwrap();
            mapped
                .image_u16()
                .unwrap()
                .rows()
                .flatten()
                .copied()
                .collect()
        })
        .collect()
}

/// The normalized cross-correlation of two equally long series.
fn correlation(a: &[f64], b: &[f64]) -> f64 {
    let mean = |v: &[f64]| v.iter().sum::<f64>() / v.len() as f64;
    let (ma, mb) = (mean(a), mean(b));
    let (mut ab, mut aa, mut bb) = (0.0, 0.0, 0.0);
    for (x, y) in a.iter().zip(b) {
        (ab, aa, bb) = (
            ab + (x - ma) * (y - mb),
            aa + (x - ma).powi(2),
            bb + (y - mb).powi(2),
        );
    }
    ab / (aa * bb).sqrt()
}

#[test]
fn a_tile_scan_s_tiles_lie_where_the_truth_puts_them() {
    let root = tempfile::tempdir().unwrap();
    let dataset = generate(root.path(), "tilescan");
    let manifest = dataset.manifest();
    assert_eq!(manifest.kind, DatasetKind::Synthetic);
    assert_eq!(manifest.plan.as_ref().unwrap()["generator"], "tilescan");
    let truth = truth(&dataset, "tilescan.json");
    let world: GroundTruth = serde_json::from_value(
        serde_json::from_str(
            &std::fs::read_to_string(dataset.path().join("truth/world.json")).unwrap(),
        )
        .unwrap(),
    )
    .unwrap();
    assert!(matches!(world.world, siq_synth::WorldInfo::Cells(_)));

    let pixel = truth["pixel_size"].as_f64().unwrap();
    let mosaic = ZarrImage::open(&dataset.path().join("zarr/truth.ome.zarr")).unwrap();
    assert!(mosaic.info().levels >= 1);
    let reader = DatasetReader::open(&dataset).unwrap();
    let tiles = truth["tiles"].as_array().unwrap();
    assert_eq!((reader.frames().len(), tiles.len()), (4, 4));
    for ((frame, pixels), tile) in reader.frames().iter().zip(frames(&dataset)).zip(tiles) {
        let at = |name: &str| [0, 1].map(|axis| tile[name][axis].as_f64().unwrap());
        let (nominal, actual, error) = (at("nominal"), at("true"), at("error"));
        // The frame says where the stage was asked to be; the truth, where it was.
        let pose = frame.sidecar.meta.pose.unwrap();
        assert_eq!([pose.x.0, pose.y.0], nominal);
        for axis in 0..2 {
            assert!((actual[axis] - nominal[axis] - error[axis]).abs() < 1e-9);
            assert!(error[axis].abs() < 5.0 * 2.0, "within five sigmas");
        }
        // The frame's middle against the mosaic, where the truth puts it and where the stage said.
        let (width, height) = (256usize, 192usize);
        let middle: Vec<f64> = (48..144)
            .flat_map(|y| (64..192).map(move |x| (x, y)))
            .map(|(x, y)| f64::from(pixels[y * width + x]))
            .collect();
        let patch = |[x, y]: [f64; 2]| -> Vec<f64> {
            // The frame's middle starts 64 and 48 pixels in: past any error of the stage's.
            let (x, y) = (
                (x / pixel).round() as i64 + 64,
                (y / pixel).round() as i64 + 48,
            );
            let rect = RectPx::new(x as u32, y as u32, 128, 96);
            let region = mosaic.read_region::<u16>(0, (0, 0, 0), rect).unwrap();
            region.pixels.iter().map(|&p| f64::from(p)).collect()
        };
        let (there, said) = (
            correlation(&middle, &patch(actual)),
            correlation(&middle, &patch(nominal)),
        );
        assert!(
            there > 0.9 && there > said,
            "{there} where it is, {said} where it was said"
        );
        assert_eq!(pixels.len(), width * height);
    }
}

#[test]
fn a_z_stack_is_sharpest_where_the_true_surface_is() {
    let root = tempfile::tempdir().unwrap();
    let dataset = generate(root.path(), "zstack");
    let truth = truth(&dataset, "zstack.json");
    let world = dataset.path().join("truth");
    let ground: GroundTruth =
        serde_json::from_str(&std::fs::read_to_string(world.join("world.json")).unwrap()).unwrap();
    let heights = ground.height_map.unwrap().read(&world).unwrap();
    let focus: Vec<f64> = truth["planes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|plane| plane["focus"].as_f64().unwrap())
        .collect();
    let planes = frames(&dataset);
    assert_eq!(planes.len(), focus.len());
    let stack = ZarrImage::open(&dataset.path().join("zarr/stack.ome.zarr")).unwrap();
    assert_eq!(stack.info().size.z, focus.len() as u64);
    let written = stack
        .read_region::<u16>(0, (0, 0, 4), RectPx::new(0, 0, 128, 96))
        .unwrap();
    assert_eq!(written.pixels, planes[4], "the stack's plane is its frame");

    // In each 16-pixel block, the sharpest plane is the one nearest the surface's focus.
    let (width, block) = (128usize, 16usize);
    let (mut agree, mut blocks) = (0, 0);
    for (bx, by) in (0..96 / block).flat_map(|by| (0..width / block).map(move |bx| (bx, by))) {
        let pixels = |plane: &[u16]| {
            (by * block..(by + 1) * block)
                .flat_map(|y| (bx * block..(bx + 1) * block - 1).map(move |x| (x, y)))
                .map(|(x, y)| {
                    (f64::from(plane[y * width + x + 1]) - f64::from(plane[y * width + x])).powi(2)
                })
                .sum::<f64>()
        };
        let sharpest = (0..planes.len())
            .max_by(|&a, &b| pixels(&planes[a]).total_cmp(&pixels(&planes[b])))
            .unwrap();
        let heights = &heights;
        let height: f64 = (by * block..(by + 1) * block)
            .flat_map(|y| {
                (bx * block..(bx + 1) * block).map(move |x| f64::from(heights[y * width + x]))
            })
            .sum::<f64>()
            / (block * block) as f64;
        let nearest = (0..focus.len())
            .min_by(|&a, &b| {
                (focus[a] + height)
                    .abs()
                    .total_cmp(&(focus[b] + height).abs())
            })
            .unwrap();
        blocks += 1;
        agree += usize::from(sharpest.abs_diff(nearest) <= 1);
    }
    assert!(agree * 10 >= blocks * 8, "{agree} of {blocks} blocks");
}

#[test]
fn each_bead_is_brightest_in_the_plane_at_its_depth() {
    let root = tempfile::tempdir().unwrap();
    let dataset = generate(root.path(), "beads");
    let truth = truth(&dataset, "beads.json");
    let ground: GroundTruth = serde_json::from_str(
        &std::fs::read_to_string(dataset.path().join("truth/world.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(truth["beads"], ground.beads.len());
    let pixel = truth["pixel_size"].as_f64().unwrap();
    let focus: Vec<f64> = truth["planes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|plane| plane["focus"].as_f64().unwrap())
        .collect();
    let planes = frames(&dataset);
    let (mut agree, mut seen) = (0, 0);
    for bead in &ground.beads {
        let (x, y) = (bead.x.0 / pixel + 64.0, bead.y.0 / pixel + 64.0);
        if !(4.0..124.0).contains(&x) || !(4.0..124.0).contains(&y) {
            continue;
        }
        let at = y as usize * 128 + x as usize;
        let brightest = (0..planes.len()).max_by_key(|&k| planes[k][at]).unwrap();
        let nearest = (0..focus.len())
            .min_by(|&a, &b| {
                (focus[a] + bead.z.0)
                    .abs()
                    .total_cmp(&(focus[b] + bead.z.0).abs())
            })
            .unwrap();
        seen += 1;
        agree += usize::from(brightest.abs_diff(nearest) <= 1);
    }
    assert!(seen > 10, "{seen} beads in the frame");
    assert!(agree * 10 >= seen * 9, "{agree} of {seen} beads");
}

#[test]
fn the_cli_writes_a_dataset_that_the_catalog_finds() {
    let root = tempfile::tempdir().unwrap();
    let ran = std::process::Command::new(env!("CARGO_BIN_EXE_siq-datagen"))
        .args(["beads", "--smoke", "--seed", "3", "--out"])
        .arg(root.path())
        .output()
        .unwrap();
    assert!(
        ran.status.success(),
        "{}",
        String::from_utf8_lossy(&ran.stderr)
    );
    assert!(String::from_utf8_lossy(&ran.stdout).starts_with("beads: "));
    let catalog = Catalog::scan(root.path()).unwrap();
    let datasets = catalog.list();
    assert_eq!(datasets.len(), 1);
    assert_eq!(
        (datasets[0].kind, datasets[0].frames),
        (DatasetKind::Synthetic, 11)
    );
    let wrong = std::process::Command::new(env!("CARGO_BIN_EXE_siq-datagen"))
        .args(["tilescan", "--smoke", "--world", "stars", "--out"])
        .arg(root.path())
        .output()
        .unwrap();
    assert!(!wrong.status.success());
}
