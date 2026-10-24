use super::*;

#[test]
fn vectors_add_subtract_and_scale() {
    let a = Vec2Um::from_um(3.0, 4.0);
    let b = Vec2Um::from_um(1.0, -1.0);
    assert_eq!(a + b, Vec2Um::from_um(4.0, 3.0));
    assert_eq!(a - b, Vec2Um::from_um(2.0, 5.0));
    assert_eq!(-a, Vec2Um::from_um(-3.0, -4.0));
    assert_eq!(a * 2.0, Vec2Um::from_um(6.0, 8.0));
    assert_eq!(a.length(), Micrometers(5.0));
    assert_eq!(a.to_um(), [3.0, 4.0]);
    assert_eq!(Vec2Um::ZERO, Vec2Um::default());
}

#[test]
fn pixel_centres_are_at_half_pixels() {
    assert_eq!(PointPx::pixel_centre(0, 0), PointPx::new(0.5, 0.5));
    assert_eq!(PointPx::pixel_centre(3, 7), PointPx::new(3.5, 7.5));
    let d = PointPx::new(4.0, 6.0) - PointPx::new(1.0, 2.0);
    assert_eq!(d + PointPx::new(1.0, 1.0), PointPx::new(4.0, 5.0));
}

#[test]
fn a_um_rectangle_contains_its_minimum_edges_only() {
    let rect = RectUm {
        origin: Vec2Um::from_um(-10.0, 0.0),
        size: Vec2Um::from_um(20.0, 10.0),
    };
    assert_eq!(rect.centre(), Vec2Um::from_um(0.0, 5.0));
    assert!(rect.contains(Vec2Um::from_um(-10.0, 0.0)));
    assert!(rect.contains(Vec2Um::from_um(9.9, 9.9)));
    assert!(!rect.contains(Vec2Um::from_um(10.0, 5.0)));
    assert!(!rect.contains(Vec2Um::from_um(0.0, 10.0)));
}

#[test]
fn pixel_rectangles_intersect() {
    let a = RectPx::new(0, 0, 10, 10);
    assert_eq!(
        a.intersection(&RectPx::new(5, 5, 10, 10)),
        Some(RectPx::new(5, 5, 5, 5))
    );
    assert_eq!(
        a.intersection(&RectPx::new(2, 3, 4, 4)),
        Some(RectPx::new(2, 3, 4, 4))
    );
    // Touching edges share no pixel.
    assert_eq!(a.intersection(&RectPx::new(10, 0, 5, 5)), None);
    assert_eq!(a.intersection(&RectPx::new(20, 20, 1, 1)), None);
    assert!(RectPx::new(1, 1, 0, 3).is_empty());
    let edge = RectPx::new(u32::MAX - 1, 0, 2, 1);
    assert_eq!(edge.right(), u64::from(u32::MAX) + 1);
}

#[test]
fn a_pose_projects_to_the_xy_plane() {
    let pose = StagePose {
        x: Micrometers(1.0),
        y: Micrometers(2.0),
        z_coarse: Micrometers(3.0),
        focus: Micrometers(4.0),
    };
    assert_eq!(pose.xy(), Vec2Um::from_um(1.0, 2.0));
}
