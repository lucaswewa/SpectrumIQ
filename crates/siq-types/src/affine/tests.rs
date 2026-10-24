use proptest::prelude::*;

use super::*;
use crate::geometry::Vec2Um;

/// Transforms as a calibration or a mosaic placement gives them: scales from 0.01 to 100 (either
/// sign on the second axis), any rotation short of ±π, shear up to ±1, and translations up to
/// 10 mm.
fn params() -> impl Strategy<Value = AffineParams> {
    (
        0.01f64..100.0,
        prop_oneof![0.01f64..100.0, -100.0f64..-0.01],
        -3.1f64..3.1,
        -1.0f64..1.0,
        [-1e4f64..1e4, -1e4f64..1e4],
    )
        .prop_map(
            |(scale_x, scale_y, rotation, shear, translation)| AffineParams {
                scale_x,
                scale_y,
                rotation,
                shear,
                translation,
            },
        )
}

fn point() -> impl Strategy<Value = [f64; 2]> {
    [-1e4f64..1e4, -1e4f64..1e4]
}

fn close(a: f64, b: f64, relative: f64) -> bool {
    (a - b).abs() <= relative * (1.0 + a.abs().max(b.abs()))
}

/// The largest entry of a transform. Rounding errors scale with it.
fn size(t: &Affine2) -> f64 {
    t.rows()
        .concat()
        .iter()
        .fold(0.0, |max, v| max.max(v.abs()))
}

fn norm([x, y]: [f64; 2]) -> f64 {
    x.abs().max(y.abs())
}

fn within(a: [f64; 2], b: [f64; 2], tolerance: f64) -> bool {
    (a[0] - b[0]).abs() <= tolerance && (a[1] - b[1]).abs() <= tolerance
}

fn rows_within(a: &Affine2, b: &Affine2, tolerance: f64) -> bool {
    let (a, b) = (a.rows().concat(), b.rows().concat());
    a.iter().zip(&b).all(|(x, y)| (x - y).abs() <= tolerance)
}

proptest! {
    #[test]
    fn invert_undoes_apply(p in params(), x in point()) {
        let t = Affine2::from_params(&p);
        let inverse = t.invert().expect("invertible");
        let image = t.apply(x);
        let tolerance = 1e-12 * (1.0 + size(&inverse)) * (1.0 + norm(image));
        let back = inverse.apply(image);
        prop_assert!(within(back, x, tolerance), "{back:?} vs {x:?}");
    }

    #[test]
    fn compose_applies_the_inner_transform_first(a in params(), b in params(), x in point()) {
        let (a, b) = (Affine2::from_params(&a), Affine2::from_params(&b));
        let tolerance = 1e-12 * (1.0 + size(&a)) * (1.0 + size(&b)) * (1.0 + norm(x));
        prop_assert!(within(a.compose(&b).apply(x), a.apply(b.apply(x)), tolerance));
    }

    #[test]
    fn compose_is_associative(a in params(), b in params(), c in params()) {
        let (a, b, c) = (Affine2::from_params(&a), Affine2::from_params(&b), Affine2::from_params(&c));
        let tolerance = 1e-12 * (1.0 + size(&a)) * (1.0 + size(&b)) * (1.0 + size(&c));
        prop_assert!(rows_within(&a.compose(&b).compose(&c), &a.compose(&b.compose(&c)), tolerance));
    }

    #[test]
    fn decompose_recovers_the_parameters(p in params()) {
        let q = Affine2::from_params(&p).decompose().expect("not singular");
        prop_assert!(close(q.scale_x, p.scale_x, 1e-9), "{q:?} vs {p:?}");
        prop_assert!(close(q.scale_y, p.scale_y, 1e-9), "{q:?} vs {p:?}");
        prop_assert!(close(q.rotation, p.rotation, 1e-9), "{q:?} vs {p:?}");
        prop_assert!(close(q.shear, p.shear, 1e-9), "{q:?} vs {p:?}");
        prop_assert_eq!(q.translation, p.translation);
    }

    #[test]
    fn any_invertible_transform_is_rebuilt_from_its_parameters(
        rows in [[-10f64..10.0, -10f64..10.0, -1e3f64..1e3], [-10f64..10.0, -10f64..10.0, -1e3f64..1e3]]
    ) {
        let t = Affine2::new(rows);
        prop_assume!(t.determinant().abs() > 1e-3);
        let rebuilt = Affine2::from_params(&t.decompose().expect("not singular"));
        prop_assert!(rows_within(&rebuilt, &t, 1e-9 * (1.0 + size(&t))), "{rebuilt:?} vs {t:?}");
    }
}

#[test]
fn singular_transforms_have_no_inverse_or_parameters() {
    for rows in [
        [[1.0, 2.0, 5.0], [2.0, 4.0, 1.0]],
        [[0.0; 3]; 2],
        [[f64::NAN, 0.0, 0.0], [0.0, 1.0, 0.0]],
        [[1.0, 0.0, f64::INFINITY], [0.0, 1.0, 0.0]],
    ] {
        let t = Affine2::new(rows);
        assert_eq!(t.invert(), None, "{rows:?}");
        assert_eq!(t.decompose(), None, "{rows:?}");
    }
}

#[test]
fn simple_transforms() {
    assert_eq!(Affine2::IDENTITY.apply([3.0, -4.0]), [3.0, -4.0]);
    let t = Affine2::translation([1.0, 2.0]);
    assert_eq!(t.apply([3.0, 4.0]), [4.0, 6.0]);
    assert_eq!(t.apply_vector([3.0, 4.0]), [3.0, 4.0]);
    assert_eq!(t.linear(), Affine2::IDENTITY);
    assert_eq!(Affine2::scale(2.0, -1.0).determinant(), -2.0);
}

#[test]
fn affine_serializes_as_two_rows() {
    let t = Affine2::new([[1.0, 2.0, 3.0], [4.0, 5.0, 6.0]]);
    let json = serde_json::to_string(&t).unwrap();
    assert_eq!(json, "[[1.0,2.0,3.0],[4.0,5.0,6.0]]");
    assert_eq!(serde_json::from_str::<Affine2>(&json).unwrap(), t);
}

/// A 1000 × 800 sensor at 2 px/µm, slightly rotated, with the image's y axis flipped against the
/// stage's, read out as a 600 × 600 region at (100, 50) with 2 × 2 binning.
fn frame_mapping(stage: Vec2Um) -> (StageToPixel, Affine2) {
    let calibration = Affine2::from_params(&AffineParams {
        scale_x: 2.0,
        scale_y: -2.0,
        rotation: 0.01,
        shear: 0.0,
        // Ignored: only the linear part of a calibration is used.
        translation: [5.0, 5.0],
    });
    let roi = Roi {
        offset_x: 100,
        offset_y: 50,
        width: 600,
        height: 600,
    };
    let binning = Binning::new(2, 2).expect("non-zero");
    let mapping = StageToPixel::for_frame(
        &calibration,
        stage,
        PointPx::new(500.0, 400.0),
        roi,
        binning,
    );
    (mapping, calibration)
}

fn close_px(a: PointPx, b: PointPx) -> bool {
    close(a.x, b.x, 1e-12) && close(a.y, b.y, 1e-12)
}

// REQ-ACQ-004
#[test]
fn the_stage_position_maps_to_the_sensor_centre() {
    let stage = Vec2Um::from_um(1000.0, 2000.0);
    let (mapping, _) = frame_mapping(stage);
    // ((500 − 100) / 2, (400 − 50) / 2)
    assert!(close_px(mapping.apply(stage), PointPx::new(200.0, 175.0)));
}

// REQ-ACQ-004
#[test]
fn a_sample_point_maps_to_its_pixel_and_back() {
    let stage = Vec2Um::from_um(1000.0, 2000.0);
    let (mapping, calibration) = frame_mapping(stage);
    let sample = stage + Vec2Um::from_um(10.0, -4.0);
    let [dx, dy] = calibration.apply_vector([10.0, -4.0]);
    let expected = PointPx::new((500.0 + dx - 100.0) / 2.0, (400.0 + dy - 50.0) / 2.0);
    assert!(close_px(mapping.apply(sample), expected));
    let back = mapping
        .inverse()
        .expect("invertible")
        .apply(mapping.apply(sample));
    assert!((back - sample).length().get() < 1e-9);
    let again = mapping.inverse().unwrap().inverse().unwrap();
    assert!(close_px(again.apply(sample), mapping.apply(sample)));
}

// REQ-ACQ-004
#[test]
fn moving_the_stage_moves_the_sample_the_other_way_in_the_image() {
    let stage = Vec2Um::from_um(1000.0, 2000.0);
    let step = Vec2Um::from_um(3.0, 1.0);
    let (before, calibration) = frame_mapping(stage);
    let (after, _) = frame_mapping(stage + step);
    let sample = Vec2Um::from_um(1005.0, 1990.0);
    let [dx, dy] = calibration.apply_vector(step.to_um());
    let shift = after.apply(sample) - before.apply(sample);
    assert!(close_px(shift, PointPx::new(-dx / 2.0, -dy / 2.0)));
}
