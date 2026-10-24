//! The frame path allocates nothing once warm (REQ-ACQ-002): acquire, fill, publish, clone, drop
//! and release, with metrics on; then the same through the fan-out. A counting global allocator
//! watches this test's thread.
//!
//! It is the only test in its binary, so the allocator counts nothing but this loop.

#![allow(unsafe_code)] // a global allocator can only be written with `unsafe`

mod common;

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::future::Future;
use std::num::NonZeroUsize;
use std::pin::pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::{Context, Waker};
use std::time::Duration;

use common::meta;
use siq_buffer::{Backend, Fanout, FramePool, Overflow, Policy, PoolConfig};
use siq_telemetry::Registry;
use siq_types::ManualClock;

static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);

thread_local! {
    /// Whether this thread's allocations are counted. Const-initialized, so reading it allocates
    /// nothing.
    static COUNTING: Cell<bool> = const { Cell::new(false) };
}

struct Counting;

fn count() {
    if COUNTING.with(Cell::get) {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
    }
}

// SAFETY: every call is forwarded to the system allocator unchanged.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        count();
        // SAFETY: forwarded with the caller's guarantees.
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        count();
        // SAFETY: forwarded with the caller's guarantees.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        count();
        // SAFETY: forwarded with the caller's guarantees.
        unsafe { System.realloc(ptr, layout, new_size) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: forwarded with the caller's guarantees.
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

// REQ-ACQ-002, REQ-ACQ-005
#[test]
fn the_steady_state_allocates_nothing() {
    let clock = Arc::new(ManualClock::new());
    let registry = Registry::new();
    let config = PoolConfig {
        name: "camera.pool".into(),
        slots: 4,
        slot_bytes: 64 * 64 * 2,
        backend: Backend::Heap,
    };
    let pool = FramePool::new(&config, clock.clone(), Some(&registry)).unwrap();
    let cycle = |n: u64| {
        let mut guard = pool.try_acquire_free().expect("a free slot");
        guard.image_mut::<u16>(64, 64).unwrap().fill(n as u16);
        let lease = guard.publish(meta(n, 64, 64)).unwrap();
        let (a, b) = (lease.clone(), lease.clone());
        clock.advance(Duration::from_millis(3));
        assert_eq!(a.image::<u16>().unwrap().get(5, 5), Some(&(n as u16)));
        drop((lease, a, b));
    };
    cycle(0); // warm-up

    // The counter must see an allocation, or a zero below would prove nothing.
    COUNTING.with(|counting| counting.set(true));
    drop(std::hint::black_box(vec![1u8]));
    COUNTING.with(|counting| counting.set(false));
    assert_eq!(
        ALLOCATIONS.swap(0, Ordering::Relaxed),
        1,
        "the counter works"
    );

    COUNTING.with(|counting| counting.set(true));
    for n in 1..=1000 {
        cycle(n);
    }
    COUNTING.with(|counting| counting.set(false));

    assert_eq!(ALLOCATIONS.load(Ordering::Relaxed), 0);
    assert_eq!(pool.stats().free, 4);

    // The fan-out: three mailboxes, read through `try_recv`, `recv_timeout` and `recv`.
    let fanout = Fanout::new(&pool);
    let mut preview = fanout.subscribe("preview", Policy::LatestOnly, |_| true);
    let overflow = Overflow::DropOldest;
    let capacity = NonZeroUsize::new(2).unwrap();
    let mut stats = fanout.subscribe("stats", Policy::Bounded { capacity, overflow }, |_| true);
    let mut writer = fanout.subscribe("writer", Policy::Lossless, |meta| {
        meta.seq.0.is_multiple_of(2)
    });
    let mut context = Context::from_waker(Waker::noop());
    let mut fan_out = |n: u64| {
        assert!(pin!(writer.recv()).poll(&mut context).is_pending());
        let mut guard = pool.try_acquire_free().expect("a free slot");
        guard.image_mut::<u16>(64, 64).unwrap().fill(n as u16);
        fanout.publish(&guard.publish(meta(n, 64, 64)).unwrap());
        let saved = pin!(writer.recv()).poll(&mut context);
        assert_eq!(saved.is_ready(), n.is_multiple_of(2));
        drop(saved);
        drop(preview.try_recv().expect("the newest frame"));
        if n.is_multiple_of(3) {
            drop(stats.recv_timeout(Duration::ZERO).expect("a queued frame"));
        }
    };
    fan_out(0); // warm-up

    COUNTING.with(|counting| counting.set(true));
    for n in 1..=1000 {
        fan_out(n);
    }
    COUNTING.with(|counting| counting.set(false));

    assert_eq!(ALLOCATIONS.load(Ordering::Relaxed), 0);
    assert!(stats.stats().dropped > 0);
    assert_eq!(writer.stats().taken, 501);
}
