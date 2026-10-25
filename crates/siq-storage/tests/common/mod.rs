//! Helpers shared by the integration tests: a heap pool, its fan-out, and frames to publish.

#![allow(dead_code)]

use std::sync::Arc;

use siq_buffer::{Backend, Fanout, FrameLease, FramePool, Policy, PoolConfig, Subscriber};
use siq_types::{
    Binning, ChannelId, FrameFlags, FrameMeta, FrameSeq, ManualClock, Millis, PixelFormat,
    PoseQuality, Roi, Timestamp,
};

/// A width that makes a frame's bytes no whole number of pages: 1000 × 3 Mono16 is 6000 bytes.
pub const WIDTH: u32 = 1000;
pub const HEIGHT: u32 = 3;

/// A heap pool of frames of [`WIDTH`] × [`HEIGHT`], and its fan-out.
pub fn pool(slots: u16) -> (FramePool, Fanout) {
    let config = PoolConfig {
        name: "test.pool".to_owned(),
        slots,
        slot_bytes: (WIDTH * HEIGHT * 2) as usize,
        backend: Backend::Heap,
    };
    let pool = FramePool::new(&config, Arc::new(ManualClock::new()), None).unwrap();
    let fanout = Fanout::new(&pool);
    (pool, fanout)
}

/// The recorder's subscriber: every frame marked to save.
pub fn recorder(fanout: &Fanout) -> Subscriber {
    fanout.subscribe("recorder", Policy::Lossless, |meta| meta.flags.save)
}

/// Frame `seq`'s pixels: each its index plus `seq`, so frames differ.
pub fn pixel(seq: u64, index: usize) -> u16 {
    (index as u64 + seq * 1000) as u16
}

/// Publishes frame `seq`, marked to save or not, and returns it.
pub fn publish(pool: &FramePool, fanout: &Fanout, seq: u64, save: bool) -> FrameLease {
    let mut guard = pool.try_acquire_free().expect("a free slot");
    for (index, value) in guard.pixels_mut::<u16>().iter_mut().enumerate() {
        *value = pixel(seq, index);
    }
    let meta = FrameMeta {
        seq: FrameSeq(seq),
        device_ts: None,
        host_ts_mid_exposure: Timestamp(seq * 1_000_000),
        exposure: Millis(10.0),
        gain: 1.0,
        roi: Roi::full(WIDTH, HEIGHT),
        binning: Binning::NONE,
        pixel_format: PixelFormat::Mono14,
        pose: None,
        pose_quality: PoseQuality::Unknown,
        channel: Some(ChannelId(2)),
        objective: None,
        calibration: None,
        flags: FrameFlags {
            save,
            moving: false,
        },
    };
    let lease = guard.publish(meta).unwrap();
    fanout.publish(&lease);
    lease
}
