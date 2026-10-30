//! The stage–camera fit (ADR-0049): a known affine recovered from noisy moves, degenerate moves and
//! poor fits refused, the backlash, and the registry's versions. REQ-CAL-001, REQ-CAL-003.

use siq_calib::*;
use siq_types::{Affine2, AffineParams, ObjectiveId};

/// A camera turned 0.4°, with 0.2 % more pixels per µm and a shear of 0.001, at 0.376 µm a pixel.
fn mount() -> Affine2 {
    Affine2::from_params(&AffineParams {
        scale_x: 1.002 / 0.376,
        scale_y: 1.002 / 0.376,
        rotation: 0.4f64.to_radians(),
        shear: 0.001,
        translation: [0.0; 2],
    })
}

/// A noise value in ±`amplitude` for `i`.
fn noise(i: usize, amplitude: f64) -> f64 {
    let z = (i as u64 + 1).wrapping_mul(0x9e37_79b9_7f4a_7c15);
    let unit = ((z ^ (z >> 29)) >> 11) as f64 / (1u64 << 53) as f64;
    (unit * 2.0 - 1.0) * amplitude
}

/// The moves of a star, out to `reach` µm, with the shifts `l` gives them and `jitter` px of noise.
fn moves(l: &Affine2, reach: f64, jitter: f64) -> Vec<Move> {
    let directions = [
        [1.0, 0.0],
        [0.0, 1.0],
        [-1.0, 0.0],
        [0.0, -1.0],
        [1.0, 1.0],
        [-1.0, 1.0],
        [-1.0, -1.0],
        [1.0, -1.0],
    ];
    let mut out = Vec::new();
    for (k, [dx, dy]) in directions.iter().chain(&directions).enumerate() {
        let r = if k < 8 { reach } else { reach / 2.0 };
        let stage = [dx * r, dy * r];
        let [sx, sy] = l.apply_vector(stage);
        out.push(Move {
            stage,
            shift: [-sx + noise(2 * k, jitter), -sy + noise(2 * k + 1, jitter)],
        });
    }
    out
}

#[test]
fn a_known_affine_is_recovered_from_noisy_moves() {
    let truth = mount();
    let fit = fit(&moves(&truth, 250.0, 0.2)).unwrap();
    let (found, expected) = (fit.affine.decompose().unwrap(), truth.decompose().unwrap());
    let scale = found.scale_x / expected.scale_x - 1.0;
    let rotation = (found.rotation - expected.rotation).to_degrees();
    assert!(scale.abs() < 2e-4, "scale off by {scale:e}");
    assert!(rotation.abs() < 0.01, "rotation off by {rotation}°");
    assert!((found.shear - expected.shear).abs() < 5e-4);
    // 0.2 px at 0.376 µm a pixel, on the stage.
    assert!(
        fit.residual_rms() < 0.1 && fit.residual_max() < 0.15,
        "{fit:?}"
    );
    assert!((fit.pixel_size() - 0.376 / 1.002).abs() < 1e-4);
    assert_eq!(fit.check(&Thresholds::default()), Ok(()));

    let record = CalibrationRecord::new(ObjectiveId(1), &fit, Some([2.0, 2.0]), "stage_camera");
    assert!((record.rotation_deg - 0.4).abs() < 0.01);
    assert_eq!(record.moves, 16);
    assert_eq!(record.affine().rows(), fit.affine.rows());
}

#[test]
fn moves_that_dont_span_the_plane_are_refused() {
    let truth = mount();
    let along_x: Vec<Move> = moves(&truth, 250.0, 0.0)
        .into_iter()
        .filter(|m| m.stage[1] == 0.0)
        .collect();
    assert!(matches!(fit(&along_x), Err(CalibError::Degenerate(_))));
    assert!(matches!(
        fit(&moves(&truth, 250.0, 0.0)[..2]),
        Err(CalibError::Degenerate(_))
    ));
    let still = vec![
        Move {
            stage: [0.0; 2],
            shift: [0.0; 2]
        };
        4
    ];
    assert!(matches!(fit(&still), Err(CalibError::Degenerate(_))));
}

#[test]
fn an_offset_common_to_the_moves_is_no_residual() {
    // The reference frame taken 1 µm off: every shift carries it.
    let truth = mount();
    let [ox, oy] = truth.apply_vector([1.0, -1.0]);
    let offset: Vec<Move> = moves(&truth, 250.0, 0.0)
        .into_iter()
        .map(|m| Move {
            shift: [m.shift[0] + ox, m.shift[1] + oy],
            ..m
        })
        .collect();
    let fit = fit(&offset).unwrap();
    assert!(fit.residual_max() < 1e-9, "{fit:?}");
    let [[a, b, _], [c, d, _]] = fit.affine.rows();
    let [[ta, tb, _], [tc, td, _]] = truth.rows();
    assert!(
        [a - ta, b - tb, c - tc, d - td]
            .iter()
            .all(|e| e.abs() < 1e-9)
    );
}

#[test]
fn a_poor_fit_is_refused() {
    // ±10 px, ±3.8 µm on the stage: a stage that doesn't repeat.
    let fit = fit(&moves(&mount(), 250.0, 10.0)).unwrap();
    assert!(matches!(
        fit.check(&Thresholds::default()),
        Err(CalibError::Rejected(_))
    ));
    let few = siq_calib::fit(&moves(&mount(), 250.0, 0.0)[..4]).unwrap();
    let refused = few.check(&Thresholds::default()).unwrap_err();
    assert!(refused.to_string().contains("4 moves"), "{refused}");
}

#[test]
fn the_backlash_comes_from_arriving_from_either_side() {
    let truth = mount();
    let fit = fit(&moves(&truth, 250.0, 0.0)).unwrap();
    // Arriving moving toward −x and −y leaves the stage 2 µm further on each axis.
    let [sx, sy] = truth.apply_vector([2.0, 2.0]);
    let found = backlash(&fit, [-sx, -sy]);
    assert!(
        (found[0] - 2.0).abs() < 1e-6 && (found[1] - 2.0).abs() < 1e-6,
        "{found:?}"
    );
}

// P10.3: the repeatability is the arrivals' standard deviation on the stage, the first's included:
// arrivals at 0, 2 and 4 µm along x, and 0, 1 and −1 along y.
#[test]
fn the_repeatability_is_the_spread_of_the_arrivals_from_one_side() {
    let fit = Fit {
        affine: mount(),
        residuals: Vec::new(),
    };
    let shifts = [[2.0, 1.0], [4.0, -1.0]].map(|at| fit.affine.apply_vector(at));
    let [x, y] = repeatability(&fit, &shifts).unwrap();
    assert!((x - 2.0).abs() < 1e-9 && (y - 1.0).abs() < 1e-9, "{x}, {y}");
    assert_eq!(repeatability(&fit, &[]), None, "one arrival has no spread");
}

#[test]
fn the_registry_keeps_every_version_and_the_newest_is_in_force() {
    let settings = tempfile::tempdir().unwrap();
    let registry = CalibrationRegistry::open(settings.path()).unwrap();
    assert_eq!(registry.current(ObjectiveId(1)), None);
    let fit = fit(&moves(&mount(), 250.0, 0.1)).unwrap();
    let first = CalibrationRecord::new(ObjectiveId(1), &fit, None, "stage_camera");
    let other = CalibrationRecord::new(ObjectiveId(2), &fit, None, "stage_camera");
    let path = registry.save(&first, &[]).unwrap();
    registry.save(&other, &[]).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(5));
    let second = CalibrationRecord::new(ObjectiveId(1), &fit, Some([1.9, 2.1]), "stage_camera");
    registry.save(&second, &[]).unwrap();
    assert!(path.starts_with(settings.path().join("calibrations").join("objective-1")));

    assert_eq!(registry.current(ObjectiveId(1)), Some(second.clone()));
    assert_eq!(
        registry.history(ObjectiveId(1)),
        [second.clone(), first.clone()]
    );
    assert_eq!(registry.get(first.id), Some(first.clone()));

    // Read again from disk, a stray file left out.
    std::fs::write(path.with_file_name("broken.json"), "{").unwrap();
    let again = CalibrationRegistry::open(settings.path()).unwrap();
    assert_eq!(again.history(ObjectiveId(1)), [second, first]);
    assert_eq!(again.current(ObjectiveId(2)), Some(other));
}
