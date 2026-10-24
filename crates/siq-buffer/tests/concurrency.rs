//! A producer and two consumers on their own threads, as the camera and its consumers run.

mod common;

use std::sync::mpsc;
use std::thread;

use common::{heap_pool, meta};

const FRAMES: u16 = 2000;

// REQ-ACQ-002, REQ-NFR-003
#[test]
fn frames_cross_threads_intact_and_every_slot_comes_back() {
    let pool = heap_pool(4, 64 * 64 * 2);
    let (to_a, from_producer_a) = mpsc::sync_channel(2);
    let (to_b, from_producer_b) = mpsc::sync_channel(2);
    let consumer = |receiver: mpsc::Receiver<siq_buffer::FrameLease>| {
        thread::spawn(move || {
            let mut seen = 0;
            for lease in receiver {
                let stamp = lease.meta().seq.0 as u16;
                let image = lease.image::<u16>().unwrap();
                assert!(image.rows().all(|row| row.iter().all(|&p| p == stamp)));
                seen += 1;
                thread::yield_now();
            }
            seen
        })
    };
    let (a, b) = (consumer(from_producer_a), consumer(from_producer_b));

    for stamp in 1..=FRAMES {
        let mut guard = loop {
            match pool.try_acquire_free() {
                Some(guard) => break guard,
                None => thread::yield_now(),
            }
        };
        guard.image_mut::<u16>(64, 64).unwrap().fill(stamp);
        let lease = guard.publish(meta(u64::from(stamp), 64, 64)).unwrap();
        to_a.send(lease.clone()).unwrap();
        to_b.send(lease).unwrap();
    }
    drop((to_a, to_b));
    assert_eq!(a.join().unwrap(), FRAMES);
    assert_eq!(b.join().unwrap(), FRAMES);
    let stats = pool.stats();
    assert_eq!((stats.free, stats.leased, stats.filling), (4, 0, 0));
}
