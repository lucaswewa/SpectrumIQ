//! The procedural worlds: determinism, seams, statistics, sampling, and their descriptions.

use proptest::prelude::*;
use siq_config::model::WorldConfig;
use siq_synth::*;
use siq_types::{Micrometers, OwnedImage, RectUm, Vec2Um};

/// One of each procedural world, with `seed`.
fn worlds(seed: u64) -> Vec<Box<dyn World>> {
    vec![
        Box::new(Cells::new(CellsParams {
            seed,
            ..CellsParams::default()
        })),
        Box::new(Grains::new(GrainsParams {
            seed,
            ..GrainsParams::default()
        })),
        Box::new(HeightField::new(HeightFieldParams {
            seed,
            ..HeightFieldParams::default()
        })),
        Box::new(Beads::new(BeadsParams {
            seed,
            ..BeadsParams::default()
        })),
    ]
}

/// `width` × `height` pixels of `scale` µm from `origin`, row after row.
fn render(world: &dyn World, origin: Vec2Um, scale: f64, width: usize, height: usize) -> Vec<f32> {
    let mut image = OwnedImage::new(width, height);
    sample(
        world,
        &grid(origin, Micrometers(scale)),
        &mut image.view_mut(),
    );
    image.view().rows().flatten().copied().collect()
}

/// World `kind` of [`worlds`], or, for 4, an image world: 400 × 300 µm of noise at 1 µm per
/// pixel.
fn world(kind: usize, seed: u64) -> Box<dyn World> {
    if kind < 4 {
        return worlds(seed).swap_remove(kind);
    }
    let noise =
        |x: usize, y: usize| ((x * 7919 + y * 104_729 + seed as usize) % 65_536) as f32 / 65_535.0;
    let params = ImageParams {
        path: "noise".into(),
        pixel_size: Micrometers(1.0),
    };
    Box::new(ImageWorld::from_image(
        params,
        OwnedImage::from_fn(400, 300, noise),
    ))
}

fn mean_and_deviation(values: &[f32]) -> (f64, f64) {
    let n = values.len() as f64;
    let mean = values.iter().map(|&v| f64::from(v)).sum::<f64>() / n;
    let variance = values
        .iter()
        .map(|&v| (f64::from(v) - mean).powi(2))
        .sum::<f64>()
        / n;
    (mean, variance.sqrt())
}

// REQ-SIM-004
#[test]
fn every_world_is_the_same_for_a_seed_and_different_for_another() {
    let origin = Vec2Um::from_um(-300.0, 120.0);
    for ((a, again), other) in worlds(7).iter().zip(worlds(7)).zip(worlds(8)) {
        let first = render(a.as_ref(), origin, 1.0, 200, 150);
        assert_eq!(
            first,
            render(again.as_ref(), origin, 1.0, 200, 150),
            "{:?}",
            a.info()
        );
        assert_ne!(
            first,
            render(other.as_ref(), origin, 1.0, 200, 150),
            "{:?}",
            a.info()
        );
    }
}

proptest! {
    // REQ-SIM-004
    #[test]
    fn regions_sampled_apart_agree_wherever_they_meet(
        kind in 0..5usize,
        seed in 0..1000u64,
        scale_exponent in -2..=2i32,
        (x, y) in (-300..300i32, -300..300i32),
        (dx, dy) in (0..24usize, 0..24usize),
    ) {
        // Power-of-two pixels on a grid of whole pixels, so both regions compute each shared
        // point exactly alike.
        let scale = 2f64.powi(scale_exponent);
        let world = world(kind, seed);
        let a = Vec2Um::from_um(f64::from(x) * scale, f64::from(y) * scale);
        let b = a + Vec2Um::from_um(dx as f64 * scale, dy as f64 * scale);
        let (first, second) = (render(world.as_ref(), a, scale, 24, 24), render(world.as_ref(), b, scale, 24, 24));
        for row in 0..24 - dy {
            for column in 0..24 - dx {
                prop_assert_eq!(first[(row + dy) * 24 + column + dx], second[row * 24 + column]);
            }
        }
    }
}

// REQ-SIM-004
#[test]
fn every_world_stays_within_its_expected_statistics() {
    let origin = Vec2Um::from_um(0.0, 0.0);
    let [cells, grains, height, beads] = <[Box<dyn World>; 4]>::try_from(worlds(3)).ok().unwrap();
    for world in [&cells, &grains, &height, &beads] {
        let values = render(world.as_ref(), origin, 2.0, 500, 500);
        assert!(
            values.iter().all(|v| (0.0..=1.0).contains(v)),
            "{:?}",
            world.info()
        );
    }

    // Cells cover about density × mean ellipse area / spacing², 0.6 × π × 12² / 40² ≈ 0.17.
    let values = render(cells.as_ref(), origin, 1.0, 1000, 1000);
    let covered = values.iter().filter(|&&v| v < 0.75).count() as f64 / values.len() as f64;
    assert!((0.10..0.25).contains(&covered), "cells cover {covered}");

    // Boundaries 1.5 µm wide around 60 µm grains are a few percent of the area.
    let values = render(grains.as_ref(), origin, 0.5, 1000, 1000);
    let boundary = values.iter().filter(|&&v| v < 0.2).count() as f64 / values.len() as f64;
    let (_, spread) = mean_and_deviation(&values);
    assert!(
        (0.02..0.10).contains(&boundary),
        "boundaries cover {boundary}"
    );
    assert!(spread > 0.1, "grains differ: {spread}");

    // The relief stays within its amplitude, and has relief.
    let heights: Vec<f32> = (0..2000)
        .map(|n| Vec2Um::from_um(f64::from(n % 50) * 40.0, f64::from(n / 50) * 40.0))
        .map(|at| height.surface_z(at).unwrap().0 as f32)
        .collect();
    let (_, relief) = mean_and_deviation(&heights);
    assert!(
        heights.iter().all(|z| z.abs() <= 40.0) && relief > 5.0,
        "relief {relief}"
    );

    // Half the 15 µm cubes of a 40 µm slab hold a bead: 400 columns × 40 / 15 × 0.5 ≈ 533.
    let region = RectUm {
        origin,
        size: Vec2Um::from_um(300.0, 300.0),
    };
    let list = beads.beads(region);
    assert!((430..640).contains(&list.len()), "{} beads", list.len());
    for bead in &list {
        assert!((-20.0..=20.0).contains(&bead.z.0) && (0.5..=1.0).contains(&bead.brightness));
        let at = Vec2Um::new(bead.x, bead.y);
        let peak = beads.density(at, bead.z).unwrap();
        assert!((peak - bead.brightness).abs() < 1e-3, "{peak} at {bead:?}");
        assert!(beads.density(at, bead.z + Micrometers(1.0)).unwrap() < 1e-3);
    }
}

// REQ-SIM-004
#[test]
fn a_coarse_footprint_averages_detail_away() {
    let world = HeightField::new(HeightFieldParams::default());
    let points: Vec<Vec2Um> = (0..400)
        .map(|n| Vec2Um::from_um(f64::from(n) * 1.3, 7.0))
        .collect();
    let sampled = |footprint: f64| {
        let values: Vec<f32> = points
            .iter()
            .map(|&at| world.intensity(at, Micrometers(footprint)))
            .collect();
        mean_and_deviation(&values)
    };
    let ((fine_mean, fine), (coarse_mean, coarse)) = (sampled(0.1), sampled(20.0));
    assert!(coarse < fine / 100.0, "{fine} → {coarse}");
    assert!(
        (coarse_mean - fine_mean).abs() < 0.1,
        "the mean stays: {fine_mean} → {coarse_mean}"
    );

    // A cell's edge is a ramp a footprint wide, not a step.
    let cells = Cells::new(CellsParams::default());
    let row = |footprint: f64| -> Vec<f32> {
        (0..4000)
            .map(|n| {
                cells.intensity(
                    Vec2Um::from_um(f64::from(n) * 0.05, 20.0),
                    Micrometers(footprint),
                )
            })
            .collect()
    };
    let steepest = |values: &[f32]| {
        values
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0, f32::max)
    };
    assert!(steepest(&row(2.0)) < steepest(&row(0.0)) / 4.0);
}

// REQ-SIM-004
#[test]
fn a_world_description_round_trips_and_builds_the_same_world() {
    let origin = Vec2Um::from_um(10.0, -20.0);
    for world in worlds(11) {
        let info = world.info();
        let json = serde_json::to_string(&info).unwrap();
        let back: WorldInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(back, info);
        let rebuilt = back.build().unwrap();
        assert_eq!(
            render(rebuilt.as_ref(), origin, 1.0, 64, 64),
            render(world.as_ref(), origin, 1.0, 64, 64)
        );
    }
    let partial: WorldInfo = serde_json::from_str(r#"{"kind": "cells", "seed": 5}"#).unwrap();
    assert_eq!(
        partial,
        WorldInfo::Cells(CellsParams {
            seed: 5,
            ..CellsParams::default()
        })
    );
    assert_eq!(WorldInfo::from_config(&WorldConfig::Cells, 5), partial);
    let beads = WorldInfo::from_config(&WorldConfig::Beads, 9);
    assert_eq!(
        beads,
        WorldInfo::Beads(BeadsParams {
            seed: 9,
            ..BeadsParams::default()
        })
    );
}

// REQ-SIM-004
#[test]
fn ground_truth_is_written_beside_a_dataset_and_reads_back() {
    let dir = tempfile::tempdir().unwrap();
    let region = RectUm {
        origin: Vec2Um::from_um(-64.0, 32.0),
        size: Vec2Um::from_um(64.0, 48.0),
    };
    let relief = HeightField::new(HeightFieldParams {
        seed: 4,
        ..HeightFieldParams::default()
    });
    let truth = write_ground_truth(&relief, region, Micrometers(2.0), dir.path(), "tile").unwrap();
    let json = std::fs::read_to_string(dir.path().join("tile.json")).unwrap();
    assert_eq!(serde_json::from_str::<GroundTruth>(&json).unwrap(), truth);
    let map = truth.height_map.as_ref().unwrap();
    assert_eq!(
        (map.width, map.height, map.file.as_str()),
        (32, 24, "tile.height.f32")
    );
    let heights = map.read(dir.path()).unwrap();
    assert_eq!(heights.len(), 32 * 24);
    let at = Vec2Um::from_um(-64.0 + 5.5 * 2.0, 32.0 + 7.5 * 2.0);
    assert_eq!(heights[7 * 32 + 5], relief.surface_z(at).unwrap().0 as f32);

    let beads = Beads::new(BeadsParams {
        seed: 4,
        ..BeadsParams::default()
    });
    let truth = write_ground_truth(&beads, region, Micrometers(2.0), dir.path(), "beads").unwrap();
    assert!(truth.height_map.is_none() && !truth.beads.is_empty());
    let origin = region.origin.to_um();
    assert!(
        truth
            .beads
            .iter()
            .all(|b| (origin[0]..origin[0] + 64.0).contains(&b.x.0))
    );
    assert!(!dir.path().join("beads.height.f32").exists());
}
