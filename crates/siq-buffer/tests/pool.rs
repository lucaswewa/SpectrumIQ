//! The pool's lifecycle through its public API.

mod common;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::{heap_pool, meta};
use siq_buffer::*;
use siq_telemetry::{MetricValue, Registry};
use siq_types::{ManualClock, PixelFormat};

// REQ-ACQ-002
#[test]
fn a_slot_goes_from_free_to_filling_to_leased_and_back() {
    let pool = heap_pool(2, 4096);
    let mut guard = pool.try_acquire_free().expect("a free slot");
    assert_eq!((pool.stats().free, pool.stats().filling), (1, 1));
    guard.image_mut::<u16>(32, 32).unwrap().fill(7);
    let lease = guard.publish(meta(1, 32, 32)).unwrap();
    assert_eq!((pool.stats().filling, pool.stats().leased), (0, 1));

    let image = lease.image::<u16>().expect("Mono16 pixels");
    assert!(image.rows().all(|row| row.iter().all(|&p| p == 7)));
    let (a, b) = (lease.clone(), lease.clone());
    assert_eq!(pool.stats().leased, 1, "clones share the slot");
    assert_eq!((a.slot(), b.meta().seq), (lease.slot(), lease.meta().seq));
    drop((a, lease));
    assert_eq!(pool.stats().leased, 1, "one lease is left");
    drop(b);
    let stats = pool.stats();
    assert_eq!((stats.free, stats.leased, stats.high_water), (2, 0, 1));
}

/// Keeps released slots, as a grabber driver does to requeue them.
#[derive(Default)]
struct Requeue(Mutex<Vec<FillGuard>>);

impl ReleaseSink for Requeue {
    fn release(&self, slot: FillGuard) {
        self.0.lock().unwrap().push(slot);
    }
}

#[test]
fn the_last_lease_hands_the_slot_to_the_release_sink() {
    let pool = heap_pool(2, 4096);
    let sink = Arc::new(Requeue::default());
    pool.set_release_sink(sink.clone());
    let lease = pool
        .try_acquire_free()
        .unwrap()
        .publish(meta(1, 8, 8))
        .unwrap();
    let slot = lease.slot();
    let clone = lease.clone();
    drop(lease);
    assert!(sink.0.lock().unwrap().is_empty(), "a lease is still held");
    drop(clone);
    let released = std::mem::take(&mut *sink.0.lock().unwrap());
    assert_eq!(
        released.iter().map(FillGuard::slot).collect::<Vec<_>>(),
        [slot]
    );
    assert_eq!(
        pool.stats().filling,
        1,
        "the sink holds it, as a queued buffer"
    );

    pool.reset_release_sink();
    let lease = released
        .into_iter()
        .next()
        .unwrap()
        .publish(meta(2, 8, 8))
        .unwrap();
    drop(lease);
    assert_eq!(pool.stats().free, 2, "back to the free list");
}

#[test]
fn a_frame_too_large_for_its_slot_is_refused_with_its_guard() {
    let pool = heap_pool(1, 4096);
    let guard = pool.try_acquire_free().unwrap();
    let error = guard.publish(meta(1, 64, 64)).unwrap_err();
    assert_eq!((error.needed, error.capacity), (Some(8192), 4096));
    assert_eq!(
        error.to_string(),
        "the image needs 8192 bytes, the slot has 4096"
    );
    let error = error
        .guard
        .publish(meta(2, u32::MAX, u32::MAX))
        .unwrap_err();
    assert_eq!(error.needed, None, "the size overflows");
    assert!(error.guard.publish(meta(3, 32, 32)).is_ok());
}

#[test]
fn views_follow_the_pixel_format() {
    let pool = heap_pool(1, 4096);
    let mut guard = pool.try_acquire_free().unwrap();
    guard.bytes_mut()[..4].copy_from_slice(&[1, 2, 3, 4]);
    let mut mono8 = meta(1, 4, 1);
    mono8.pixel_format = PixelFormat::Mono8;
    let lease = guard.publish(mono8).unwrap();
    assert!(lease.image::<u16>().is_none(), "Mono8 isn't u16");
    assert_eq!(lease.image::<u8>().unwrap().row(0), [1, 2, 3, 4]);
    assert_eq!(lease.as_bytes(), [1, 2, 3, 4]);
}

#[test]
fn slots_are_whole_pages_and_page_aligned() {
    let pool = heap_pool(3, 5000);
    assert_eq!((pool.slots(), pool.slot_bytes()), (3, 2 * PAGE));
    let mut guards = pool.take_all_free();
    assert_eq!(guards.len(), 3);
    assert!(pool.try_acquire_free().is_none());
    for guard in &mut guards {
        assert_eq!(guard.as_mut_ptr().addr() % PAGE, 0);
        assert_eq!(guard.len(), 2 * PAGE);
        assert_eq!(guard.pixels_mut::<u16>().len(), PAGE);
        assert!(guard.bytes_mut().iter().all(|&b| b == 0), "zero-filled");
    }
}

#[test]
fn impossible_sizes_are_refused() {
    let clock = Arc::new(ManualClock::new());
    for (slots, slot_bytes) in [(0, 4096), (2, 0), (2, usize::MAX)] {
        let config = PoolConfig {
            name: "p".into(),
            slots,
            slot_bytes,
            backend: Backend::Heap,
        };
        let made = FramePool::new(&config, clock.clone(), None);
        assert!(
            matches!(made, Err(PoolError::InvalidSize)),
            "{slots} × {slot_bytes}"
        );
    }
}

#[test]
fn metrics_report_free_slots_the_high_water_mark_and_lease_time() {
    let clock = Arc::new(ManualClock::new());
    let registry = Registry::new();
    let config = PoolConfig {
        name: "camera.pool".into(),
        slots: 3,
        slot_bytes: 4096,
        backend: Backend::Heap,
    };
    let pool = FramePool::new(&config, clock.clone(), Some(&registry)).unwrap();
    let held = pool.try_acquire_free().unwrap();
    let lease = pool
        .try_acquire_free()
        .unwrap()
        .publish(meta(1, 8, 8))
        .unwrap();
    clock.advance(Duration::from_millis(25));
    drop((lease, held));
    let snapshot = registry.snapshot();
    assert_eq!(
        snapshot["camera.pool.free_slots"],
        MetricValue::Gauge { value: 3.0 }
    );
    assert_eq!(
        snapshot["camera.pool.in_use_high_water"],
        MetricValue::Gauge { value: 2.0 }
    );
    let MetricValue::Histogram { count, sum, .. } = &snapshot["camera.pool.lease_ms"] else {
        panic!("a histogram")
    };
    assert_eq!((*count, *sum), (1, 25.0));
}

#[test]
fn leases_and_guards_outlive_the_pool_handle() {
    let pool = heap_pool(2, 4096);
    let mut guard = pool.try_acquire_free().unwrap();
    guard.bytes_mut()[0] = 42;
    let lease = guard.publish(meta(1, 1, 1)).unwrap();
    let other = pool.try_acquire_free().unwrap();
    drop(pool);
    assert_eq!(lease.as_bytes(), [42, 0]);
    drop((lease, other));
}
