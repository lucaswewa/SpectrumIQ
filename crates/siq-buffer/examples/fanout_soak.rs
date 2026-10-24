//! A camera-rate producer and three consumers through the fan-out: a slow preview, a slower
//! statistics view, and a lossless writer that takes the frames flagged to save, one in ten. It
//! prints what each consumer got and lost, and fails if the counts don't add up or a slot is never
//! returned.
//!
//! ```text
//! cargo run --release -p siq-buffer --example fanout_soak
//! ```
//!
//! The full run is the camera's: 60 s at 9 frames/s, with 12 slots of 100 MP Mono16 (194 MB each).
//! Smoke mode (`SIQ_SMOKE=1`) runs 2 s at 100 frames/s with 128 × 128 frames. The consumers' pace
//! is set relative to the frame period.

use std::num::NonZeroUsize;
use std::process::ExitCode;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use siq_buffer::{Backend, Fanout, FramePool, Overflow, Policy, PoolConfig, Subscriber};
use siq_types::{
    Binning, FrameFlags, FrameMeta, FrameSeq, Millis, PixelFormat, PoseQuality, Roi, SystemClock,
    Timestamp,
};

fn main() -> ExitCode {
    let smoke = std::env::var_os("SIQ_SMOKE").is_some();
    let (run, period, width, height) = if smoke {
        (Duration::from_secs(2), Duration::from_millis(10), 128, 128)
    } else {
        (
            Duration::from_secs(60),
            Duration::from_millis(111),
            11_648,
            8_742,
        )
    };
    let config = PoolConfig {
        name: "camera.pool".into(),
        slots: 12,
        slot_bytes: width as usize * height as usize * 2,
        backend: Backend::Heap,
    };
    let pool = FramePool::new(&config, Arc::new(SystemClock::new()), None).expect("a heap pool");
    let fanout = Fanout::new(&pool);
    let bounded = Policy::Bounded {
        capacity: NonZeroUsize::new(4).unwrap(),
        overflow: Overflow::DropOldest,
    };
    let saved = |meta: &FrameMeta| meta.flags.save;
    let consumers = [
        consume(
            fanout.subscribe("preview", Policy::LatestOnly, |_| true),
            period * 3 / 2,
        ),
        consume(
            fanout.subscribe("statistics", bounded, |_| true),
            period * 5 / 2,
        ),
        consume(
            fanout.subscribe("writer", Policy::Lossless, saved),
            period * 5,
        ),
    ];

    // The producer: one frame per period, or a lost frame when the pool is dry.
    let started = Instant::now();
    let (mut published, mut lost) = (0u64, 0u64);
    let (mut seq, mut next) = (0, started);
    while started.elapsed() < run {
        match pool.try_acquire_free() {
            Some(mut guard) => {
                guard.pixels_mut::<u16>()[..width as usize].fill(seq as u16);
                let lease = guard.publish(frame(seq, width, height));
                fanout.publish(&lease.expect("the frame fits"));
                published += 1;
            }
            None => lost += 1,
        }
        seq += 1;
        next += period;
        thread::sleep(next.saturating_duration_since(Instant::now()));
    }
    drop(fanout);

    let mb = pool.slot_bytes() as f64 / f64::from(1 << 20);
    println!("{published} frames of {mb:.2} MB published, {lost} lost to a dry pool, in {run:?}");
    let mut ok = true;
    for consumer in consumers {
        let (subscriber, read) = consumer.join().expect("a consumer");
        let stats = subscriber.stats();
        println!(
            "{:<10} received {:>5}, read {:>5}, dropped {:>5}, at most {} queued",
            subscriber.name(),
            stats.received,
            read,
            stats.dropped,
            stats.max_queued
        );
        ok &= stats.taken == read && stats.received == stats.taken + stats.dropped;
    }
    let stats = pool.stats();
    println!("{stats:?}");
    ok &= stats.free == stats.slots;
    if ok {
        ExitCode::SUCCESS
    } else {
        eprintln!("the counts don't add up");
        ExitCode::FAILURE
    }
}

/// Reads frames until the fan-out closes, taking `pace` per frame.
fn consume(mut subscriber: Subscriber, pace: Duration) -> thread::JoinHandle<(Subscriber, u64)> {
    thread::spawn(move || {
        let mut read = 0;
        while let Some(lease) = subscriber.recv_blocking() {
            assert_eq!(
                lease.as_bytes()[0],
                lease.meta().seq.0 as u8,
                "the frame is intact"
            );
            read += 1;
            thread::sleep(pace);
        }
        (subscriber, read)
    })
}

fn frame(seq: u64, width: u32, height: u32) -> FrameMeta {
    FrameMeta {
        seq: FrameSeq(seq),
        device_ts: None,
        host_ts_mid_exposure: Timestamp(0),
        exposure: Millis(10.0),
        gain: 1.0,
        roi: Roi::full(width, height),
        binning: Binning::NONE,
        pixel_format: PixelFormat::Mono16,
        pose: None,
        pose_quality: PoseQuality::Unknown,
        channel: None,
        objective: None,
        calibration: None,
        flags: FrameFlags {
            save: seq.is_multiple_of(10),
            moving: false,
        },
    }
}
