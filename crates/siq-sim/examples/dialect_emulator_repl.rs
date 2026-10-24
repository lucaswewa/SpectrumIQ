//! Drives the dialect emulator, in real time, with a scripted session in the example dialect, and
//! prints every line sent and received: identification, a move watched by status queries, a stop,
//! a jog, homing, and errors. Then the same move with the dialect waiting for its completion
//! reply.
//!
//! ```text
//! cargo run -p siq-sim --example dialect_emulator_repl
//! ```
//!
//! It takes about 3 s, in smoke mode too.

use std::sync::Arc;
use std::time::Duration;

use siq_ascii::Dialect;
use siq_serial::{DuplexTransport, LineTransport, SerialError, duplex};
use siq_sim::dialect::DialectEmulator;
use siq_sim::state::TrueState;
use siq_types::SystemClock;

/// Sends `line`, and prints it and every line that arrives within `wait`.
async fn send(host: &mut DuplexTransport, line: &str, wait: u64) -> Result<(), SerialError> {
    println!("> {line}");
    host.write_line(line).await?;
    loop {
        match host.read_line(Duration::from_millis(wait)).await {
            Ok(line) => println!("  < {line}"),
            Err(SerialError::Timeout) => return Ok(()),
            Err(error) => return Err(error),
        }
    }
}

fn start(dialect: Dialect) -> (DuplexTransport, Arc<TrueState>) {
    let state = Arc::new(TrueState::new(0));
    let (host, device) = duplex(dialect.framing(), "focus");
    let clock = Arc::new(SystemClock::new());
    tokio::spawn(DialectEmulator::new(Arc::new(dialect), state.clone(), clock).run(device));
    (host, state)
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), SerialError> {
    let dialect = Dialect::example();
    println!("{}, {} µm a step", dialect.name, dialect.step.0);
    let (mut host, state) = start(dialect);
    send(&mut host, "ECHO 0", 30).await?;
    send(&mut host, "ID?", 30).await?;
    // 400 µm at 1000 µm/s.
    send(&mut host, "MA 8000 20000", 30).await?;
    for _ in 0..4 {
        send(&mut host, "ST?", 120).await?;
    }
    println!("  (the true focus: {:.3} µm)", state.pose().focus.0);
    send(&mut host, "MA 0 20000", 150).await?;
    send(&mut host, "HALT", 30).await?;
    send(&mut host, "ST?", 30).await?;
    send(&mut host, "JOG 100000", 200).await?;
    send(&mut host, "HALT", 30).await?;
    send(&mut host, "HOME", 30).await?;
    send(&mut host, "ST?", 800).await?;
    send(&mut host, "ST?", 30).await?;
    send(&mut host, "MA 999999 20000", 30).await?;
    send(&mut host, "FOCUS!", 30).await?;

    println!("\nwith motion.done = \"reply\":");
    let replying = Dialect::EXAMPLE.replace("done = \"poll\"", "done = \"reply\"");
    let (mut host, _) = start(Dialect::parse(&replying, "replying.toml").expect("valid"));
    send(&mut host, "MA 8000 20000", 600).await?;
    Ok(())
}
