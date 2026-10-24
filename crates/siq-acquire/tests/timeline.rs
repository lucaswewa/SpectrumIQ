//! The pose timeline: interpolation between readings, settling, carrying on past the last reading,
//! and what it doesn't know.

use proptest::prelude::*;
use siq_acquire::PoseTimeline;
use siq_devices::{AxisStatus, FocusStatus, MotionState, StageStatus, Xyz};
use siq_types::{Micrometers, PoseQuality, Timestamp};

fn stage(ms: u64, [x, y, z]: [f64; 3], moving: bool) -> StageStatus {
    AxisStatus {
        position: Xyz::from_um(x, y, z),
        state: if moving {
            MotionState::Moving
        } else {
            MotionState::Idle
        },
        homed: true,
        ts: Timestamp(ms * 1_000_000),
    }
}

fn focus(ms: u64, um: f64, moving: bool) -> FocusStatus {
    AxisStatus {
        position: Micrometers(um),
        state: if moving {
            MotionState::Moving
        } else {
            MotionState::Idle
        },
        homed: true,
        ts: Timestamp(ms * 1_000_000),
    }
}

fn at(ms: f64) -> Timestamp {
    Timestamp((ms * 1e6) as u64)
}

fn position() -> impl Strategy<Value = [f64; 3]> {
    [-5e4..5e4f64, -5e4..5e4f64, -1e3..1e3f64]
}

fn far(a: [f64; 3], b: [f64; 3]) -> f64 {
    (0..3).map(|i| (b[i] - a[i]).powi(2)).sum::<f64>().sqrt()
}

proptest! {
    #[test]
    fn between_readings_the_pose_is_interpolated_and_any_path_lies_within_its_uncertainty(
        from in position(),
        to in position(),
        moving in any::<(bool, bool)>(),
        fraction in 0.0..1.0f64,
        path in 0.0..=1.0f64,
    ) {
        let timeline = PoseTimeline::new(8);
        timeline.push_stage(&stage(1_000, from, moving.0));
        timeline.push_stage(&stage(1_200, to, moving.1));
        // The fraction of the way, at the nanosecond the timestamp holds.
        let t = at(1_000.0 + 200.0 * fraction);
        let fraction = (t.0 as f64 - 1e9) / 2e8;
        let pose = timeline.pose_at(t);
        let p = pose.pose.unwrap();
        let expected: [f64; 3] = std::array::from_fn(|i| from[i] + fraction * (to[i] - from[i]));
        prop_assert!(far([p.x.0, p.y.0, p.z_coarse.0], expected) < 1e-6);
        let PoseQuality::Interpolated { uncertainty } = pose.quality else {
            panic!("{:?} between readings that moved", pose.quality)
        };
        prop_assert!((uncertainty.0 - far(from, to)).abs() < 1e-6);
        // Whatever the stage did between the readings, along the way from one to the other, the
        // true position lies within the uncertainty.
        let truth: [f64; 3] = std::array::from_fn(|i| from[i] + path * (to[i] - from[i]));
        prop_assert!(far([p.x.0, p.y.0, p.z_coarse.0], truth) <= uncertainty.0 + 1e-6);
        prop_assert!(pose.moving);
    }

    #[test]
    fn a_pose_is_settled_only_between_still_readings_at_the_same_place(
        at_rest in position(),
        moving in any::<(bool, bool)>(),
        nudge in prop::sample::select(vec![0.0, 0.625, 1e-3]),
        fraction in 0.0..1.0f64,
    ) {
        let timeline = PoseTimeline::new(8);
        let moved = [at_rest[0] + nudge, at_rest[1], at_rest[2]];
        timeline.push_stage(&stage(1_000, at_rest, moving.0));
        timeline.push_stage(&stage(1_200, moved, moving.1));
        let pose = timeline.pose_at(at(1_000.0 + 200.0 * fraction));
        let still = !moving.0 && !moving.1 && nudge == 0.0;
        prop_assert_eq!(pose.quality == PoseQuality::Settled, still);
        prop_assert_eq!(pose.moving, !still);
    }

    #[test]
    fn past_the_last_reading_a_moving_stage_carries_on_at_its_speed(
        from in position(),
        to in position(),
        ahead in 0.0..500.0f64,
        last_moving in any::<bool>(),
    ) {
        let timeline = PoseTimeline::new(8);
        timeline.push_stage(&stage(1_000, from, true));
        timeline.push_stage(&stage(1_200, to, last_moving));
        let t = at(1_200.0 + ahead);
        let ahead = (t.0 as f64 - 1.2e9) / 1e6;
        let pose = timeline.pose_at(t);
        let p = pose.pose.unwrap();
        // At the speed between the two readings, or still if the last one was.
        let speed = if last_moving { 1.0 } else { 0.0 };
        let expected: [f64; 3] = std::array::from_fn(|i| to[i] + speed * (to[i] - from[i]) * ahead / 200.0);
        prop_assert!(far([p.x.0, p.y.0, p.z_coarse.0], expected) < 1e-6);
        let uncertainty = speed * far(from, to);
        let interpolated = PoseQuality::Interpolated { uncertainty: Micrometers(uncertainty) };
        match pose.quality {
            PoseQuality::Interpolated { uncertainty: u } => prop_assert!((u.0 - uncertainty).abs() < 1e-6),
            other => prop_assert_eq!(other, interpolated),
        }
        prop_assert!(!timeline.decided(t));
    }
}

#[test]
fn the_focus_joins_the_stage_and_the_worse_of_the_two_says_how_well_the_pose_is_known() {
    let timeline = PoseTimeline::new(8);
    assert_eq!(timeline.pose_at(at(1.0)).quality, PoseQuality::Unknown);
    assert!(
        timeline.decided(at(1.0)),
        "nothing is followed: nothing to wait for"
    );
    timeline.push_stage(&stage(100, [10.0, 20.0, 0.0], false));
    timeline.push_stage(&stage(300, [10.0, 20.0, 0.0], false));
    // Before the first reading, nothing is known.
    assert_eq!(timeline.pose_at(at(50.0)).quality, PoseQuality::Unknown);
    assert_eq!(timeline.pose_at(at(200.0)).quality, PoseQuality::Settled);

    // A focus drive followed but not yet read leaves the pose unknown.
    timeline.push_focus(&focus(400, 5.0, false));
    assert_eq!(timeline.pose_at(at(200.0)).quality, PoseQuality::Unknown);
    timeline.push_focus(&focus(100, 5.0, false));
    let pose = timeline.pose_at(at(200.0));
    assert_eq!(
        (pose.quality, pose.pose.unwrap().focus),
        (PoseQuality::Settled, Micrometers(5.0))
    );

    // The focus moving makes the pose interpolated, by its own uncertainty.
    timeline.push_focus(&focus(500, 9.0, true));
    timeline.push_stage(&stage(600, [10.0, 20.0, 0.0], false));
    let pose = timeline.pose_at(at(450.0));
    let uncertainty = Micrometers(4.0);
    assert_eq!(pose.quality, PoseQuality::Interpolated { uncertainty });
    assert_eq!(pose.pose.unwrap().focus, Micrometers(7.0));
    assert!(pose.moving && !timeline.still());
    assert!(timeline.decided(at(450.0)) && !timeline.decided(at(550.0)));
}

#[test]
fn the_timeline_keeps_its_newest_readings_in_order() {
    let timeline = PoseTimeline::new(4);
    for ms in [100, 200, 400, 300, 500, 600] {
        timeline.push_stage(&stage(ms, [ms as f64, 0.0, 0.0], true));
    }
    // 300 came late and went in its place; the oldest went first.
    let pose = timeline.pose_at(at(350.0)).pose.unwrap();
    assert_eq!(pose.x, Micrometers(350.0));
    assert_eq!(timeline.pose_at(at(150.0)).quality, PoseQuality::Unknown);
}
