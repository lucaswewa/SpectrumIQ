//! Allocates and locks a frame pool the size of the real one, 12 × 200 MB, runs frames through
//! it, and reports the working set.
//!
//! ```text
//! cargo run --release -p siq-buffer --example pool_locked
//! ```
//!
//! Smoke mode (`SIQ_SMOKE=1`) uses 4 × 8 MB. The example exits with an error if the pool can't be
//! locked.

use std::process::ExitCode;
use std::sync::Arc;
use std::time::Instant;

use siq_buffer::{Backend, FramePool, PoolConfig, working_set};
use siq_types::{
    Binning, FrameFlags, FrameMeta, FrameSeq, Millis, PixelFormat, PoseQuality, Roi, SystemClock,
    Timestamp,
};

const MB: usize = 1 << 20;

fn main() -> ExitCode {
    let smoke = std::env::var_os("SIQ_SMOKE").is_some();
    // A 100 MP Mono16 frame is 11648 × 8742 × 2 bytes, about 194 MiB.
    let (slots, width, height) = if smoke {
        (4, 2048, 2048)
    } else {
        (12, 11_648, 8_742)
    };
    let config = PoolConfig {
        name: "camera.pool".into(),
        slots,
        slot_bytes: width as usize * height as usize * 2,
        backend: Backend::WindowsLocked,
    };
    let show = |when: &str| match working_set() {
        Ok((min, max)) => println!(
            "{when:<16} working set {:>6} MB min, {:>6} MB max",
            min / MB,
            max / MB
        ),
        Err(error) => println!("{when:<16} working set unknown: {error}"),
    };
    show("before");
    let started = Instant::now();
    let pool = match FramePool::new(&config, Arc::new(SystemClock::new()), None) {
        Ok(pool) => pool,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::FAILURE;
        }
    };
    let total = pool.slots() * pool.slot_bytes();
    println!(
        "locked {} × {} MB = {} MB in {:.0?}",
        pool.slots(),
        pool.slot_bytes() / MB,
        total / MB,
        started.elapsed()
    );
    show("with the pool");

    // Frames through every slot: fill one row, publish, lease twice, release.
    let started = Instant::now();
    let frames = 3 * u64::from(slots);
    for seq in 0..frames {
        let mut guard = pool.try_acquire_free().expect("a free slot");
        guard.pixels_mut::<u16>()[..width as usize].fill(seq as u16);
        let lease = guard
            .publish(frame(seq, width, height))
            .expect("the frame fits");
        let copy = lease.clone();
        assert_eq!(
            copy.image::<u16>()
                .and_then(|image| image.get(0, 0).copied()),
            Some(seq as u16)
        );
    }
    println!(
        "{frames} frames through the pool in {:.1?}",
        started.elapsed()
    );
    println!("{:?}", pool.stats());

    drop(pool);
    show("after");
    ExitCode::SUCCESS
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
        pixel_format: PixelFormat::Mono14,
        pose: None,
        pose_quality: PoseQuality::Unknown,
        channel: None,
        objective: None,
        calibration: None,
        flags: FrameFlags::default(),
    }
}
