//! Drives a GRBL stage through the `Stage` trait, in real time, and prints what it reports.
//!
//! ```text
//! cargo run -p siq-grbl --example grbl_drive
//! cargo run -p siq-grbl --example grbl_drive -- --port COM3 [--baud 115200] [--move]
//! ```
//!
//! - **By default, against the emulator,** with the default `[sim.errors]`. The session homes,
//!   moves directly and then from one side, jogs, and stops a move halfway. Each step prints the
//!   reported position and the true stage's, and the status's round trip after a second. It takes
//!   about 4 s, in smoke mode too.
//! - **With `--port`, against a real controller.** It connects and prints the controller's
//!   firmware, travel and status, its `$$` as a configuration's `[stage.expect]` (P10.3), and the
//!   status's round trip over 2 s of polling. Only with `--move` does it move: 1 mm along −X and
//!   back. It never homes.

use std::error::Error;
use std::sync::Arc;
use std::time::Duration;

use siq_config::model::{SimErrors, SimStageConfig};
use siq_devices::{Approach, Device, MoveOpts, Stage, StageStatus, Xyz};
use siq_grbl::{GrblOptions, GrblStage};
use siq_serial::{Framing, PortSettings, duplex};
use siq_sim::grbl::{GrblEmulator, GrblErrors, GrblSettings};
use siq_sim::state::TrueState;
use siq_types::{Clock, MicrometersPerSecond, SystemClock};

fn show(step: &str, status: &StageStatus, truth: Option<&TrueState>) {
    let p = status.position;
    print!(
        "{step:<28} {:?}, homed {}: ({:.1}, {:.1}, {:.1}) µm",
        status.state, status.homed, p.x.0, p.y.0, p.z.0
    );
    match truth.map(TrueState::pose) {
        Some(pose) => {
            let (x, y, z) = (pose.x.0, pose.y.0, pose.z_coarse.0);
            println!(", truly ({x:.1}, {y:.1}, {z:.1})");
        }
        None => println!(),
    }
}

/// Prints the status's round trips so far.
fn round_trip(stage: &GrblStage) {
    if let Some(trip) = stage.round_trip() {
        println!(
            "the status's round trip, {} polls: {:.1?} at least, {:.1?} on average, {:.1?} at most",
            trip.count, trip.min, trip.mean, trip.max
        );
    }
}

async fn emulated(clock: Arc<dyn Clock>) -> Result<(), Box<dyn Error>> {
    let state = Arc::new(TrueState::new(0));
    let errors = GrblErrors::from(&SimErrors::default());
    let settings = GrblSettings::default();
    let emulator = GrblEmulator::new(settings, errors, 1, state.clone(), clock.clone());
    let (host, device) = duplex(Framing::default(), "grbl");
    tokio::spawn(emulator.run(device));
    let options = GrblOptions::from(&SimStageConfig::default());
    let stage = GrblStage::connect(host, options, clock).await?;
    let truth = Some(&*state);
    let status = || *stage.status().borrow();
    show("connected", &status(), truth);
    tokio::time::sleep(Duration::from_secs(1)).await;
    round_trip(&stage);
    show("homed", &stage.home().await?, truth);

    let (direct, one_way) = (
        MoveOpts::default(),
        MoveOpts {
            approach: Approach::Unidirectional,
            ..MoveOpts::default()
        },
    );
    let target = Xyz::from_um(-10_000.0, -5_000.0, -500.0);
    show(
        "to the target, directly",
        &stage.move_to(target, direct).await?,
        truth,
    );
    stage
        .move_to(Xyz::from_um(-20_000.0, -8_000.0, -900.0), direct)
        .await?;
    show(
        "back, from beyond",
        &stage.move_to(target, direct).await?,
        truth,
    );
    show(
        "back, from one side",
        &stage.move_to(target, one_way).await?,
        truth,
    );

    let speed = |um_s| MicrometersPerSecond(um_s);
    stage
        .jog(Xyz::new(speed(-4_000.0), speed(-4_000.0), speed(0.0)))
        .await?;
    tokio::time::sleep(Duration::from_millis(250)).await;
    show("jogging", &status(), truth);
    stage
        .jog(Xyz::new(speed(0.0), speed(0.0), speed(0.0)))
        .await?;
    tokio::time::sleep(Duration::from_millis(50)).await;
    show("jog cancelled", &status(), truth);

    let far = stage.move_to(Xyz::from_um(-60_000.0, -40_000.0, 0.0), direct);
    let halfway = async {
        tokio::time::sleep(Duration::from_millis(400)).await;
        stage.stop().await
    };
    let (moved, stopped) = tokio::join!(far, halfway);
    stopped?;
    println!("{:<28} {moved:?}", "the stopped move");
    show("stopped", &status(), truth);
    Ok(())
}

async fn real(
    port: &str,
    baud: u32,
    moving: bool,
    clock: Arc<dyn Clock>,
) -> Result<(), Box<dyn Error>> {
    let settings = PortSettings {
        baud,
        ..PortSettings::default()
    };
    let transport = siq_serial::open(port, &settings, Framing::default())?;
    let options = GrblOptions::from(&SimStageConfig::default());
    let stage = GrblStage::connect(transport, options, clock).await?;
    println!("firmware: {:?}", stage.info().firmware);
    let limits = stage.limits();
    println!("travel: {:?} to {:?}", limits.min, limits.max);
    show("connected", &stage.status().borrow(), None);
    println!("[stage.expect]");
    let mut numbers: Vec<_> = stage.settings().iter().collect();
    numbers.sort_by_key(|(number, _)| **number);
    for (number, value) in numbers {
        println!("\"${number}\" = {value:?}");
    }
    tokio::time::sleep(Duration::from_secs(2)).await;
    round_trip(&stage);
    if moving {
        let opts = MoveOpts::default();
        show(
            "1 mm along -X",
            &stage
                .move_by(Xyz::from_um(-1_000.0, 0.0, 0.0), opts)
                .await?,
            None,
        );
        show(
            "and back",
            &stage.move_by(Xyz::from_um(1_000.0, 0.0, 0.0), opts).await?,
            None,
        );
    }
    Ok(())
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let value = |name: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
    };
    let clock: Arc<dyn Clock> = Arc::new(SystemClock::new());
    match value("--port") {
        Some(port) => {
            let baud = value("--baud").map_or(Ok(115_200), |baud| baud.parse())?;
            real(port, baud, args.iter().any(|a| a == "--move"), clock).await
        }
        None => emulated(clock).await,
    }
}
