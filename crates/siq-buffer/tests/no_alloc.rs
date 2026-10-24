//! The frame path allocates nothing once warm (REQ-ACQ-002): acquire, fill, publish, clone, drop
//! and release, with metrics on. A counting global allocator watches this test's thread.
//!
//! It is the only test in its binary, so the allocator counts nothing but this loop.

#![allow(unsafe_code)] // a global allocator can only be written with `unsafe`

mod common;

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use common::meta;
use siq_buffer::{Backend, FramePool, PoolConfig};
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

// REQ-ACQ-002
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
}
