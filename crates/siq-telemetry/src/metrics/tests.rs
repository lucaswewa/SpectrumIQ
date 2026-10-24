use std::thread;

use super::*;

#[test]
fn concurrent_updates_are_all_counted() {
    let registry = Arc::new(Registry::new());
    let threads: Vec<_> = (0..8)
        .map(|t| {
            let registry = Arc::clone(&registry);
            thread::spawn(move || {
                // Each thread fetches the metrics by name: they are the same ones.
                let frames = registry.counter("camera.frames").unwrap();
                let latency = registry
                    .histogram("preview.latency_ms", &[10.0, 50.0])
                    .unwrap();
                let level = registry.gauge("pool.free_slots").unwrap();
                for i in 0..10_000u64 {
                    frames.increment();
                    latency.record((i % 100) as f64);
                    level.set(f64::from(t));
                }
            })
        })
        .collect();
    for thread in threads {
        thread.join().unwrap();
    }
    assert_eq!(registry.counter("camera.frames").unwrap().get(), 80_000);
    let latency = registry
        .histogram("preview.latency_ms", &[10.0, 50.0])
        .unwrap();
    assert_eq!(latency.count(), 80_000);
    // Per thread, the values 0..100 repeat 100 times: their sum is 4950 × 100.
    assert_eq!(latency.sum(), 8.0 * 4950.0 * 100.0);
    assert!((0.0..8.0).contains(&registry.gauge("pool.free_slots").unwrap().get()));
}

#[test]
fn histogram_buckets_include_their_upper_bound() {
    let registry = Registry::new();
    let h = registry.histogram("h", &[1.0, 2.0]).unwrap();
    for value in [0.5, 1.0, 1.5, 2.0, 2.5, f64::NAN] {
        h.record(value);
    }
    assert_eq!(
        h.buckets(),
        [
            Bucket {
                le: Some(1.0),
                count: 2
            },
            Bucket {
                le: Some(2.0),
                count: 2
            },
            Bucket { le: None, count: 1 },
        ]
    );
    assert_eq!((h.count(), h.sum()), (5, 7.5));
}

#[test]
fn names_kinds_and_bounds_are_checked() {
    let registry = Registry::new();
    for name in ["", "Camera.frames", "camera..frames", ".frames", "frames!"] {
        assert_eq!(
            registry.counter(name).unwrap_err(),
            MetricError::InvalidName(name.to_owned()),
            "{name:?}"
        );
    }
    registry.counter("stage.moves").unwrap();
    assert!(matches!(
        registry.gauge("stage.moves"),
        Err(MetricError::Conflict { existing, .. }) if existing == "counter"
    ));
    registry.histogram("stage.move_ms", &[10.0]).unwrap();
    assert!(matches!(
        registry.histogram("stage.move_ms", &[20.0]),
        Err(MetricError::Conflict { .. })
    ));
    for bounds in [&[][..], &[2.0, 1.0], &[1.0, 1.0], &[f64::INFINITY]] {
        assert!(
            matches!(
                registry.histogram("bad", bounds),
                Err(MetricError::InvalidBounds { .. })
            ),
            "{bounds:?}"
        );
    }
}

#[test]
fn the_snapshot_serializes_every_metric() {
    let registry = Registry::new();
    registry.counter("camera.frames_acquired").unwrap().add(42);
    registry.gauge("pool.free_slots").unwrap().set(10.0);
    let latency = registry
        .histogram("preview.latency_ms", &[10.0, 50.0])
        .unwrap();
    for value in [4.0, 12.0, 80.0] {
        latency.record(value);
    }
    insta::assert_json_snapshot!(registry.snapshot());
}
