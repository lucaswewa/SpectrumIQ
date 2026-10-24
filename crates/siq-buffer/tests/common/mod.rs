//! Helpers shared by the integration tests.

#![allow(dead_code)]

use std::sync::Arc;

use siq_buffer::{Backend, FramePool, PoolConfig};
use siq_types::{
    Binning, FrameFlags, FrameMeta, FrameSeq, ManualClock, Millis, PixelFormat, PoseQuality, Roi,
    Timestamp,
};

/// The metadata of a `width × height` Mono16 frame.
pub fn meta(seq: u64, width: u32, height: u32) -> FrameMeta {
    FrameMeta {
        seq: FrameSeq(seq),
        device_ts: None,
        host_ts_mid_exposure: Timestamp(0),
        exposure: Millis(1.0),
        gain: 1.0,
        roi: Roi::full(width, height),
        binning: Binning::NONE,
        pixel_format: PixelFormat::Mono16,
        pose: None,
        pose_quality: PoseQuality::Unknown,
        channel: None,
        objective: None,
        calibration: None,
        flags: FrameFlags::default(),
    }
}

/// A heap pool of `slots` slots of at least `slot_bytes`, on a manual clock.
pub fn heap_pool(slots: u16, slot_bytes: usize) -> FramePool {
    let config = PoolConfig {
        name: "test.pool".to_owned(),
        slots,
        slot_bytes,
        backend: Backend::Heap,
    };
    FramePool::new(&config, Arc::new(ManualClock::new()), None).expect("a small heap pool")
}
