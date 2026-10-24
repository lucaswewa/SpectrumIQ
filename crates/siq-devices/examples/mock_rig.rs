//! A mock rig: homes the stage, visits a 3 × 3 grid, snaps a frame at each stop, and prints the
//! stage's status stream as it changes.
//!
//! ```text
//! cargo run -p siq-devices --example mock_rig
//! ```
//!
//! Everything completes at once, so it runs the same in smoke mode.

use std::sync::Arc;

use siq_buffer::{Backend, FillGuard, FramePool, PoolConfig};
use siq_devices::mock::{MockCamera, MockStage, small_sensor};
use siq_devices::*;
use siq_types::{FrameMeta, SystemClock};

/// Publishes each frame the camera hands over, prints it, and lets it go.
struct Print;

impl FrameSink for Print {
    fn frame(&self, slot: FillGuard, meta: FrameMeta) {
        let lease = slot.publish(meta).expect("the frame fits");
        let (width, height) = meta.binning.image_size(meta.roi.width, meta.roi.height);
        println!(
            "    frame {} at {} µs: {width}×{height} {:?}, every byte {}",
            meta.seq.0,
            meta.host_ts_mid_exposure.0 / 1000,
            meta.pixel_format,
            lease.as_bytes()[0]
        );
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), DeviceError> {
    let clock = Arc::new(SystemClock::new());
    let travel = Travel {
        min: Xyz::from_um(0.0, 0.0, -5_000.0),
        max: Xyz::from_um(100_000.0, 75_000.0, 0.0),
    };
    let stage: Arc<dyn Stage> = Arc::new(MockStage::new(travel, clock.clone()));
    let camera: Arc<dyn Camera> = Arc::new(MockCamera::new(small_sensor(), clock.clone()));

    // The status stream, printed as it changes.
    let mut status = stage.status();
    let printer = tokio::spawn(async move {
        while status.changed().await.is_ok() {
            let now = *status.borrow_and_update();
            let at = now.position;
            println!(
                "stage {:?} at ({:.0}, {:.0}, {:.0}), homed: {}, at {} µs",
                now.state,
                at.x,
                at.y,
                at.z,
                now.homed,
                now.ts.0 / 1000
            );
        }
        println!("stage gone: the status stream ended");
    });

    let settings = CameraSettings {
        trigger: TriggerMode::Software,
        ..camera.settings()
    };
    camera.configure(settings).await?;
    let config = PoolConfig {
        name: "camera.pool".into(),
        slots: 4,
        slot_bytes: settings.frame_bytes(),
        backend: Backend::Heap,
    };
    let pool = FramePool::new(&config, clock.clone(), None).expect("a small pool");
    camera.start(pool, Arc::new(Print)).await?;

    stage.home().await?;
    tokio::task::yield_now().await; // let the printer show each status, rather than the last
    let consistent = MoveOpts {
        approach: Approach::Unidirectional,
        ..MoveOpts::default()
    };
    for row in 0..3 {
        for column in 0..3 {
            let (x, y) = (1_000.0 * f64::from(column), 1_000.0 * f64::from(row));
            stage
                .move_to(Xyz::from_um(x, y, -2_000.0), consistent)
                .await?;
            tokio::task::yield_now().await;
            camera.trigger().await?;
        }
    }
    camera.stop().await?;
    println!("camera: {:?}", camera.stats());

    drop(stage);
    printer.await.expect("the printer finishes");
    Ok(())
}
