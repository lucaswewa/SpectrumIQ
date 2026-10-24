//! The fan-out through its public API: policies, filters, unsubscribing, closing, waiting from
//! threads and from async code, and a soak with slow consumers.

mod common;

use std::future::Future;
use std::num::NonZeroUsize;
use std::pin::pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::{Context, Poll, Wake, Waker};
use std::thread;
use std::time::Duration;

use common::{heap_pool, meta};
use siq_buffer::*;

fn bounded(capacity: usize, overflow: Overflow) -> Policy {
    Policy::Bounded {
        capacity: NonZeroUsize::new(capacity).unwrap(),
        overflow,
    }
}

/// Publishes frame `seq` from `pool` through `fanout`, and returns how many subscribers took it.
fn publish(pool: &FramePool, fanout: &Fanout, seq: u64) -> usize {
    let guard = pool.try_acquire_free().expect("a free slot");
    fanout.publish(&guard.publish(meta(seq, 8, 8)).unwrap())
}

fn drain(subscriber: &mut Subscriber) -> Vec<u64> {
    std::iter::from_fn(|| subscriber.try_recv())
        .map(|lease| lease.meta().seq.0)
        .collect()
}

/// Runs a future on this thread, parking it until the future's waker is woken.
fn block_on<F: Future>(future: F) -> F::Output {
    struct Unpark(thread::Thread);
    impl Wake for Unpark {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
    }
    let waker = Waker::from(Arc::new(Unpark(thread::current())));
    let mut context = Context::from_waker(&waker);
    let mut future = pin!(future);
    loop {
        if let Poll::Ready(output) = future.as_mut().poll(&mut context) {
            return output;
        }
        thread::park();
    }
}

// REQ-ACQ-005
#[test]
fn latest_only_keeps_the_newest_frame_and_releases_the_others() {
    let pool = heap_pool(4, 128);
    let fanout = Fanout::new(&pool);
    let mut preview = fanout.subscribe("preview", Policy::LatestOnly, |_| true);
    for seq in 1..=3 {
        assert_eq!(publish(&pool, &fanout, seq), 1);
        assert_eq!(pool.stats().leased, 1, "the replaced frame went back");
    }
    assert_eq!(drain(&mut preview), [3]);
    let stats = preview.stats();
    assert_eq!(
        (stats.received, stats.dropped, stats.taken, stats.max_queued),
        (3, 2, 1, 1)
    );
    assert_eq!(pool.stats().free, 4);
}

// REQ-ACQ-005
#[test]
fn a_full_bounded_mailbox_drops_the_oldest_or_the_newest() {
    let pool = heap_pool(8, 128);
    let fanout = Fanout::new(&pool);
    let mut newest = fanout.subscribe("newest", bounded(2, Overflow::DropOldest), |_| true);
    let mut oldest = fanout.subscribe("oldest", bounded(2, Overflow::DropNewest), |_| true);
    for seq in 1..=5 {
        assert_eq!(publish(&pool, &fanout, seq), 2);
    }
    assert_eq!(pool.stats().leased, 4, "each mailbox keeps two");
    assert_eq!(drain(&mut newest), [4, 5]);
    assert_eq!(drain(&mut oldest), [1, 2]);
    for subscriber in [&newest, &oldest] {
        let stats = subscriber.stats();
        assert_eq!((stats.received, stats.dropped, stats.taken), (5, 3, 2));
        assert_eq!(stats.max_queued, 2);
    }
    let names: Vec<_> = fanout.stats().into_iter().map(|(name, _)| name).collect();
    assert_eq!(names, ["newest", "oldest"]);
}

// REQ-ACQ-005, REQ-NFR-003
#[test]
fn a_lossless_subscriber_keeps_every_frame_so_the_pool_runs_dry() {
    let pool = heap_pool(3, 128);
    let fanout = Fanout::new(&pool);
    let mut writer = fanout.subscribe("writer", Policy::Lossless, |_| true);
    for seq in 1..=3 {
        publish(&pool, &fanout, seq);
    }
    assert!(
        pool.try_acquire_free().is_none(),
        "the backpressure reaches the producer"
    );
    assert_eq!(drain(&mut writer), [1, 2, 3]);
    assert_eq!(writer.stats().dropped, 0);
    assert_eq!(pool.stats().free, 3);
}

// REQ-ACQ-005
#[test]
fn the_filter_chooses_the_frames() {
    let pool = heap_pool(8, 128);
    let fanout = Fanout::new(&pool);
    let mut even = fanout.subscribe("even", Policy::Lossless, |meta| {
        meta.seq.0.is_multiple_of(2)
    });
    let mut all = fanout.subscribe("all", bounded(8, Overflow::DropNewest), |_| true);
    let delivered: Vec<_> = (1..=6).map(|seq| publish(&pool, &fanout, seq)).collect();
    assert_eq!(delivered, [1, 2, 1, 2, 1, 2]);
    assert_eq!(drain(&mut even), [2, 4, 6]);
    assert_eq!(drain(&mut all), [1, 2, 3, 4, 5, 6]);
    assert_eq!(
        even.stats().received,
        3,
        "frames the filter refused aren't counted"
    );
}

// REQ-ACQ-005
#[test]
fn unsubscribing_releases_the_queued_frames_but_not_those_taken() {
    let pool = heap_pool(4, 128);
    let fanout = Fanout::new(&pool);
    let mut leaving = fanout.subscribe("leaving", bounded(4, Overflow::DropOldest), |_| true);
    let mut staying = fanout.subscribe("staying", Policy::LatestOnly, |_| true);
    for seq in 1..=3 {
        publish(&pool, &fanout, seq);
    }
    let kept = leaving.try_recv().unwrap();
    drop(leaving);
    assert_eq!(
        pool.stats().leased,
        2,
        "frame 1, still held, and frame 3, in `staying`"
    );
    assert_eq!(kept.meta().seq.0, 1);
    assert_eq!(publish(&pool, &fanout, 4), 1, "only `staying` is left");
    assert_eq!(drain(&mut staying), [4]);
    drop(kept);
    assert_eq!(pool.stats().free, 4);
}

// REQ-ACQ-005, REQ-NFR-003
#[test]
fn subscribers_come_and_go_while_frames_are_in_flight() {
    let pool = Arc::new(heap_pool(8, 128));
    let fanout = Fanout::new(&pool);
    let stop = Arc::new(AtomicBool::new(false));
    let producer = {
        let (pool, fanout, stop) = (Arc::clone(&pool), fanout.clone(), Arc::clone(&stop));
        thread::spawn(move || {
            let mut seq = 0;
            while !stop.load(Ordering::Relaxed) {
                if let Some(guard) = pool.try_acquire_free() {
                    fanout.publish(&guard.publish(meta(seq, 8, 8)).unwrap());
                    seq += 1;
                }
                thread::yield_now();
            }
            seq
        })
    };
    for round in 0..200_u32 {
        let mut subscriber = fanout.subscribe("brief", bounded(3, Overflow::DropOldest), |_| true);
        if round.is_multiple_of(2) {
            drop(subscriber.recv_timeout(Duration::from_millis(1)));
        }
    }
    stop.store(true, Ordering::Relaxed);
    assert!(producer.join().unwrap() > 0);
    assert!(fanout.stats().is_empty());
    assert_eq!(pool.stats().free, 8);
}

// REQ-ACQ-005
#[test]
fn after_close_subscribers_read_what_is_queued_then_get_none() {
    let pool = heap_pool(4, 128);
    let fanout = Fanout::new(&pool);
    let mut subscriber = fanout.subscribe("late", Policy::Lossless, |_| true);
    publish(&pool, &fanout, 1);
    publish(&pool, &fanout, 2);
    let other = fanout.clone();
    drop(fanout);
    assert_eq!(
        subscriber
            .recv_timeout(Duration::ZERO)
            .unwrap()
            .meta()
            .seq
            .0,
        1
    );
    assert_eq!(
        publish(&pool, &other, 3),
        1,
        "a clone keeps the fan-out open"
    );

    other.close();
    assert_eq!(
        publish(&pool, &other, 4),
        0,
        "closed mailboxes take nothing"
    );
    assert_eq!(subscriber.recv_blocking().unwrap().meta().seq.0, 2);
    assert_eq!(subscriber.recv_blocking().unwrap().meta().seq.0, 3);
    assert!(subscriber.recv_blocking().is_none());
    assert_eq!(
        subscriber.recv_timeout(Duration::from_secs(1)).unwrap_err(),
        RecvError::Closed
    );
    assert!(block_on(subscriber.recv()).is_none());

    let mut after = other.subscribe("after", Policy::LatestOnly, |_| true);
    assert!(
        after.recv_blocking().is_none(),
        "a subscriber of a closed fan-out is closed"
    );
    assert_eq!(pool.stats().free, 4);
}

// REQ-ACQ-005
#[test]
fn a_waiting_thread_wakes_for_the_next_frame() {
    let pool = heap_pool(4, 128);
    let fanout = Fanout::new(&pool);
    let mut subscriber = fanout.subscribe("thread", Policy::LatestOnly, |_| true);
    let short = Duration::from_millis(20);
    assert_eq!(
        subscriber.recv_timeout(short).unwrap_err(),
        RecvError::Timeout
    );
    let waiting = thread::spawn(move || subscriber.recv_blocking().map(|lease| lease.meta().seq.0));
    thread::sleep(short);
    publish(&pool, &fanout, 7);
    assert_eq!(waiting.join().unwrap(), Some(7));
}

// REQ-ACQ-005
#[test]
fn an_async_consumer_wakes_for_each_frame_and_ends_on_close() {
    let pool = heap_pool(4, 128);
    let fanout = Fanout::new(&pool);
    let mut subscriber = fanout.subscribe("async", Policy::Lossless, |_| true);
    let consumer = thread::spawn(move || {
        block_on(async {
            let mut seen = Vec::new();
            while let Some(lease) = subscriber.recv().await {
                seen.push(lease.meta().seq.0);
            }
            seen
        })
    });
    for seq in 0..100 {
        let guard = loop {
            match pool.try_acquire_free() {
                Some(guard) => break guard,
                None => thread::yield_now(),
            }
        };
        fanout.publish(&guard.publish(meta(seq, 8, 8)).unwrap());
    }
    drop(fanout);
    assert_eq!(consumer.join().unwrap(), (0..100).collect::<Vec<_>>());
    assert_eq!(pool.stats().free, 4);
}

/// Reads frames until the fan-out closes, sleeping `pause` per frame, and checks their order.
fn consume(mut subscriber: Subscriber, pause: Duration) -> thread::JoinHandle<(Subscriber, u64)> {
    thread::spawn(move || {
        let (mut read, mut last) = (0, None);
        while let Some(lease) = subscriber.recv_blocking() {
            assert!(last < Some(lease.meta().seq.0), "frames come in order");
            last = Some(lease.meta().seq.0);
            read += 1;
            thread::sleep(pause);
        }
        (subscriber, read)
    })
}

// REQ-ACQ-005, REQ-NFR-003
#[test]
fn two_slow_consumers_lose_frames_counted_while_a_lossless_one_gets_all_it_asked_for() {
    const FRAMES: u64 = 3000;
    let pool = heap_pool(12, 64 * 64 * 2);
    let fanout = Fanout::new(&pool);
    let slow = Duration::from_millis(1);
    let preview = consume(
        fanout.subscribe("preview", Policy::LatestOnly, |_| true),
        slow,
    );
    let stats = consume(
        fanout.subscribe("stats", bounded(4, Overflow::DropOldest), |_| true),
        slow,
    );
    let saved = |meta: &siq_types::FrameMeta| meta.seq.0.is_multiple_of(3);
    let writer = consume(
        fanout.subscribe("writer", Policy::Lossless, saved),
        Duration::ZERO,
    );

    let (mut published, mut dry, mut wanted) = (0, 0, 0);
    for seq in 0..FRAMES {
        match pool.try_acquire_free() {
            Some(guard) => {
                fanout.publish(&guard.publish(meta(seq, 64, 64)).unwrap());
                published += 1;
                wanted += u64::from(seq.is_multiple_of(3));
            }
            None => dry += 1,
        }
    }
    drop(fanout);

    for (consumer, slow) in [(preview, true), (stats, true), (writer, false)] {
        let (subscriber, read) = consumer.join().unwrap();
        let stats = subscriber.stats();
        assert_eq!(stats.taken, read, "{}", subscriber.name());
        assert_eq!(
            stats.received,
            stats.taken + stats.dropped,
            "{}",
            subscriber.name()
        );
        assert_eq!(slow, stats.dropped > 0, "{}: {stats:?}", subscriber.name());
        if !slow {
            assert_eq!(
                stats.received, wanted,
                "the writer got every frame it asked for"
            );
        }
    }
    assert_eq!(published + dry, FRAMES);
    assert!(published > 0);
    assert_eq!(pool.stats().free, 12);
}
