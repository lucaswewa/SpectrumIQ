//! A locked pool on the real system: `VirtualAlloc`, `VirtualLock`, and the working set raised for
//! it and restored after. The only test in its binary, since the working set is process-wide.

mod common;

use std::sync::Arc;

use common::meta;
use siq_buffer::{Backend, FramePool, PAGE, PoolConfig, working_set};
use siq_types::ManualClock;

// REQ-ACQ-002, REQ-NFR-002
#[test]
fn locked_slots_are_aligned_and_the_working_set_is_raised_then_restored() {
    let (min_before, _) = working_set().unwrap();
    let config = PoolConfig {
        name: "locked".into(),
        slots: 4,
        slot_bytes: 1 << 20,
        backend: Backend::WindowsLocked,
    };
    let pool = FramePool::new(&config, Arc::new(ManualClock::new()), None).unwrap();
    let (min, max) = working_set().unwrap();
    assert!(
        min >= min_before + 4 * (1 << 20),
        "raised from {min_before} to {min}"
    );
    assert!(max >= min);

    let mut guard = pool.try_acquire_free().unwrap();
    assert_eq!(guard.as_mut_ptr().addr() % PAGE, 0);
    assert!(
        guard.bytes_mut().iter().all(|&b| b == 0),
        "committed pages are zero"
    );
    guard.bytes_mut().fill(0xAB);
    let lease = guard.publish(meta(1, 512, 1024)).unwrap();
    assert!(lease.as_bytes().iter().all(|&b| b == 0xAB));
    assert_eq!(lease.as_bytes().len(), 1 << 20);

    drop((lease, pool));
    assert_eq!(
        working_set().unwrap().0,
        min_before,
        "restored when the pool is gone"
    );
}
