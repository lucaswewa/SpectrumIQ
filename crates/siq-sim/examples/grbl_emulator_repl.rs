//! Drives the GRBL emulator, in real time, with a scripted session, and prints every line sent
//! and received: unlock, settings, a move watched by status reports, a dwell, a jog cancelled, a
//! feed hold, and a reset in motion.
//!
//! ```text
//! cargo run -p siq-sim --example grbl_emulator_repl
//! ```
//!
//! It takes about 3 s, in smoke mode too.

use std::sync::Arc;
use std::time::Duration;

use siq_config::model::SimErrors;
use siq_serial::{DuplexTransport, Framing, LineTransport, SerialError, duplex};
use siq_sim::grbl::{GrblEmulator, GrblErrors, GrblSettings};
use siq_sim::state::TrueState;
use siq_types::SystemClock;

/// Prints every line that arrives within `wait`.
async fn drain(host: &mut DuplexTransport, wait: Duration) -> Result<(), SerialError> {
    loop {
        match host.read_line(wait).await {
            Ok(line) => println!("  < {line}"),
            Err(SerialError::Timeout) => return Ok(()),
            Err(error) => return Err(error),
        }
    }
}

async fn send(host: &mut DuplexTransport, line: &str) -> Result<(), SerialError> {
    println!("> {line}");
    host.write_line(line).await?;
    drain(host, Duration::from_millis(60)).await
}

async fn realtime(host: &mut DuplexTransport, byte: u8, name: &str) -> Result<(), SerialError> {
    println!("! {name}");
    host.write_realtime(byte).await?;
    drain(host, Duration::from_millis(60)).await
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), SerialError> {
    let state = Arc::new(TrueState::new(0));
    let errors = GrblErrors::from(&SimErrors::default());
    let emulator = GrblEmulator::new(
        GrblSettings::default(),
        errors,
        1,
        state.clone(),
        Arc::new(SystemClock::new()),
    );
    let (mut host, device) = duplex(Framing::default(), "grbl");
    tokio::spawn(emulator.run(device));

    println!("(the port opens)");
    drain(&mut host, Duration::from_millis(60)).await?;
    send(&mut host, "$X").await?;
    send(&mut host, "$110=6000").await?;
    send(&mut host, "G21 G90").await?;
    send(&mut host, "G0 X-20 Y-10").await?;
    for _ in 0..3 {
        realtime(&mut host, b'?', "?").await?;
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
    // `ok` for the dwell comes once the move has finished.
    println!("> G4 P0");
    host.write_line("G4 P0").await?;
    drain(&mut host, Duration::from_millis(600)).await?;
    realtime(&mut host, b'?', "?").await?;
    let truth = state.pose();
    println!(
        "  (the true stage: {:.3}, {:.3} mm: backlash, scale and random error)",
        truth.x.0 / 1e3,
        truth.y.0 / 1e3
    );

    send(&mut host, "$J=G91 X-5 F600").await?;
    tokio::time::sleep(Duration::from_millis(200)).await;
    realtime(&mut host, 0x85, "jog cancel").await?;
    realtime(&mut host, b'?', "?").await?;

    send(&mut host, "G1 X-30 F1200").await?;
    tokio::time::sleep(Duration::from_millis(200)).await;
    realtime(&mut host, b'!', "feed hold").await?;
    realtime(&mut host, b'?', "?").await?;
    realtime(&mut host, b'~', "resume").await?;
    tokio::time::sleep(Duration::from_millis(100)).await;
    realtime(&mut host, 0x18, "soft reset, in motion").await?;
    realtime(&mut host, b'?', "?").await?;
    send(&mut host, "G0 X200").await?;
    Ok(())
}
