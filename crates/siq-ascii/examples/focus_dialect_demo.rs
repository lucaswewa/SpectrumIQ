//! Reads a dialect, shows what it sends and what a broken one reports, then drives the dialect
//! emulator with it in real time, and prints what the drive reports and where the focus truly is.
//!
//! ```text
//! cargo run -p siq-ascii --example focus_dialect_demo [-- path/to/dialect.toml]
//! ```
//!
//! The dialect defaults to `config/dialects/example-focus.toml`. The session homes, moves, moves
//! down unidirectionally, jogs and stops. Then it moves again with the dialect switched to wait for
//! its completion reply. It takes about 3 s, in smoke mode too.

use std::error::Error;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use siq_ascii::{AsciiFocusDrive, Dialect, lengths};
use siq_devices::{Approach, Device, FocusDrive, FocusStatus, MoveOpts};
use siq_serial::duplex;
use siq_sim::dialect::DialectEmulator;
use siq_sim::state::TrueState;
use siq_types::{Clock, Micrometers, MicrometersPerSecond, SystemClock};

fn show(step: &str, status: &FocusStatus, state: &TrueState) {
    let (position, truth) = (status.position.0, state.pose().focus.0);
    let homed = if status.homed { "homed" } else { "not homed" };
    println!(
        "{step:<30} {:?}, {homed}: {position:.2} µm, truly {truth:.3} µm",
        status.state
    );
}

/// Drives the emulator with `dialect`, and gives back the drive.
async fn session(
    dialect: Dialect,
    clock: Arc<dyn Clock>,
    state: Arc<TrueState>,
) -> Result<(), Box<dyn Error>> {
    let dialect = Arc::new(dialect);
    let (host, port) = duplex(dialect.framing(), "focus");
    tokio::spawn(DialectEmulator::new(dialect.clone(), state.clone(), clock.clone()).run(port));
    let drive = AsciiFocusDrive::connect(host, dialect, clock).await?;
    let info = drive.info();
    println!(
        "connected to {} {} ({:?})",
        info.vendor, info.model, info.firmware
    );
    show("connected", &drive.status().borrow(), &state);
    show("homed", &drive.home().await?, &state);
    let fast = MoveOpts {
        speed: Some(MicrometersPerSecond(5_000.0)),
        approach: Approach::Direct,
    };
    show(
        "to 2000 µm",
        &drive.move_to(Micrometers(2_000.0), fast).await?,
        &state,
    );
    let one_way = MoveOpts {
        approach: Approach::Unidirectional,
        ..fast
    };
    show(
        "down 500 µm, arriving up",
        &drive.move_by(Micrometers(-500.0), one_way).await?,
        &state,
    );
    drive.jog(MicrometersPerSecond(1_000.0)).await?;
    tokio::time::sleep(Duration::from_millis(300)).await;
    show("jogging", &drive.status().borrow(), &state);
    drive.stop().await?;
    show("stopped", &drive.status().borrow(), &state);
    Ok(())
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn Error>> {
    let dialect = match std::env::args().nth(1) {
        Some(path) => Dialect::load(Path::new(&path))?,
        None => Dialect::example(),
    };
    let [min, max] = dialect.motion.travel;
    println!(
        "{}: {} µm a step, travel {min} to {max}",
        dialect.name, dialect.step.0
    );
    let commands = &dialect.commands;
    let values = lengths([
        ("target", 1_234.5),
        ("speed", 2_000.0),
        ("velocity", -500.0),
    ]);
    for (name, command) in [
        ("move", Some(&commands.move_to)),
        ("jog", commands.jog.as_ref()),
    ] {
        if let Some(command) = command {
            println!(
                "  {name:<6} sends {:?}",
                command.send.render(&values, dialect.step.0)
            );
        }
    }

    let broken = Dialect::EXAMPLE
        .replace("MA {target:steps}", "MA {to:steps}")
        .replace("step = 0.05", "step = 0");
    if let Err(error) = Dialect::parse(&broken, "broken.toml") {
        println!("a broken dialect reports:\n{error}");
    }

    let clock: Arc<dyn Clock> = Arc::new(SystemClock::new());
    println!("\npolling for the end of each move:");
    session(dialect.clone(), clock.clone(), Arc::new(TrueState::new(0))).await?;
    println!("\nwaiting for the completion reply:");
    let replying = Dialect::EXAMPLE.replace("done = \"poll\"", "done = \"reply\"");
    session(
        Dialect::parse(&replying, "replying.toml")?,
        clock,
        Arc::new(TrueState::new(0)),
    )
    .await
}
