use std::sync::Arc;
use std::thread;

use super::*;

#[test]
fn timestamps_saturate() {
    assert_eq!(
        Timestamp(9).saturating_since(Timestamp(5)),
        Duration::from_nanos(4)
    );
    assert_eq!(Timestamp(5).saturating_since(Timestamp(9)), Duration::ZERO);
    assert_eq!(
        Timestamp(u64::MAX - 1) + Duration::from_secs(1),
        Timestamp(u64::MAX)
    );
    assert_eq!(Timestamp(1) - Duration::from_nanos(5), Timestamp(0));
    assert_eq!(
        Timestamp(1) + Duration::from_millis(2),
        Timestamp(2_000_001)
    );
}

#[test]
fn a_manual_clock_moves_only_when_told_and_never_back() {
    let clock = ManualClock::new();
    assert_eq!(clock.now(), Timestamp(0));
    clock.advance(Duration::from_millis(5));
    assert_eq!(clock.now(), Timestamp(5_000_000));
    clock.set(Timestamp(1));
    assert_eq!(clock.now(), Timestamp(5_000_000));
    clock.set(Timestamp(7_000_000));
    assert_eq!(clock.now(), Timestamp(7_000_000));
    assert_eq!(
        clock.utc(Timestamp(1_500_000_000)).to_rfc3339(),
        "2026-01-01T00:00:01.500+00:00"
    );
}

#[test]
fn a_manual_clock_moved_by_one_thread_is_seen_by_another() {
    let clock = Arc::new(ManualClock::new());
    let mover = {
        let clock = Arc::clone(&clock);
        thread::spawn(move || clock.advance(Duration::from_nanos(10)))
    };
    mover.join().unwrap();
    assert_eq!(clock.now(), Timestamp(10));
}

#[test]
fn the_system_clock_is_monotonic_and_maps_to_utc() {
    let clock = SystemClock::default();
    let (a, b) = (clock.now(), clock.now());
    assert!(b >= a);
    let skew = clock.now_utc() - Utc::now();
    assert!(skew.abs() < TimeDelta::seconds(1), "{skew}");
    let before = clock.now();
    thread::sleep(Duration::from_millis(5));
    assert!(clock.now().saturating_since(before) >= Duration::from_millis(5));
}

#[test]
fn clocks_are_shared_as_trait_objects() {
    let clock: Arc<dyn Clock> = Arc::new(ManualClock::default());
    assert_eq!(clock.now(), Timestamp(0));
    assert_eq!(clock.now_utc(), clock.utc(Timestamp(0)));
}
