//! Random sequences of operations against a model of where every slot is. Each fill stamps its
//! slot, and every lease must keep reading its stamp: a slot handed out twice, or written while
//! leased, would show.

mod common;

use common::{heap_pool, meta};
use proptest::prelude::*;
use siq_buffer::{FillGuard, FrameLease};

const SLOTS: u16 = 4;

#[derive(Debug, Clone, Copy)]
enum Op {
    Acquire,
    Publish(usize),
    DropGuard(usize),
    Clone(usize),
    DropLease(usize),
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        Just(Op::Acquire),
        any::<usize>().prop_map(Op::Publish),
        any::<usize>().prop_map(Op::DropGuard),
        any::<usize>().prop_map(Op::Clone),
        any::<usize>().prop_map(Op::DropLease),
    ]
}

proptest! {
    // REQ-ACQ-002
    #[test]
    fn slots_are_never_shared_with_a_writer(ops in proptest::collection::vec(op(), 1..200)) {
        let pool = heap_pool(SLOTS, 4096);
        let mut guards: Vec<FillGuard> = Vec::new();
        let mut leases: Vec<(FrameLease, u16)> = Vec::new();
        let mut stamp = 0u16;
        for op in ops {
            match op {
                Op::Acquire => {
                    let guard = pool.try_acquire_free();
                    let free = usize::from(SLOTS) - guards.len() - distinct(&leases);
                    prop_assert_eq!(guard.is_some(), free > 0);
                    guards.extend(guard);
                }
                Op::Publish(i) if !guards.is_empty() => {
                    let mut guard = guards.swap_remove(i % guards.len());
                    stamp += 1;
                    guard.pixels_mut::<u16>().fill(stamp);
                    leases.push((guard.publish(meta(u64::from(stamp), 32, 32)).unwrap(), stamp));
                }
                Op::DropGuard(i) if !guards.is_empty() => drop(guards.swap_remove(i % guards.len())),
                Op::Clone(i) if !leases.is_empty() => {
                    let (lease, stamp) = &leases[i % leases.len()];
                    leases.push((lease.clone(), *stamp));
                }
                Op::DropLease(i) if !leases.is_empty() => drop(leases.swap_remove(i % leases.len())),
                _ => {}
            }
            for (lease, stamp) in &leases {
                let image = lease.image::<u16>().unwrap();
                prop_assert!(image.rows().all(|row| row.iter().all(|p| p == stamp)));
            }
            let mut held: Vec<usize> = guards.iter().map(FillGuard::slot).collect();
            let leased = distinct(&leases);
            held.extend(leases.iter().map(|(lease, _)| lease.slot()));
            held.sort_unstable();
            held.dedup();
            prop_assert_eq!(held.len(), guards.len() + leased, "a guard's slot is also leased");
            let stats = pool.stats();
            prop_assert_eq!((stats.filling, stats.leased), (guards.len(), leased));
            prop_assert_eq!(stats.free + stats.filling + stats.leased, usize::from(SLOTS));
        }
    }
}

/// The number of slots the leases hold.
fn distinct(leases: &[(FrameLease, u16)]) -> usize {
    let mut slots: Vec<usize> = leases.iter().map(|(lease, _)| lease.slot()).collect();
    slots.sort_unstable();
    slots.dedup();
    slots.len()
}
