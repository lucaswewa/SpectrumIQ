//! The audit trail and its verification (ADR-0067): a trail written across a month's end, as the
//! service writes it, verified, then edited, cut and reordered, each found where it was done; or
//! a trail of yours verified, as `spectrumiq-server audit verify` does.
//!
//! ```text
//! cargo run -p siq-auth --example audit_verify [-- <audit folder>]
//! ```

use std::error::Error;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use siq_auth::audit::verify;
use siq_auth::{AuditLog, Event, Kind};
use siq_types::ManualClock;

fn main() -> Result<(), Box<dyn Error>> {
    if let Some(folder) = std::env::args().nth(1) {
        let verified = verify(Path::new(&folder))?;
        println!(
            "{} entries in {} file(s), the chain intact; its last hash {}",
            verified.entries, verified.files, verified.head
        );
        return Ok(());
    }
    let folder = tempfile::tempdir()?;
    let clock = Arc::new(ManualClock::starting_at("2026-10-31T23:59:56Z".parse()?));
    let log = AuditLog::open(folder.path(), clock.clone())?;
    let address = "192.168.1.20".parse().ok();
    let session = Some("4f1c9a2be07d");
    let events = [
        Event::new(Kind::Login, json!({ "role": "operator" })).by("olga", session, address),
        Event::new(Kind::Token, json!({ "holder": "olga", "reason": { "kind": "requested" } }))
            .by("olga", session, None),
        Event::new(
            Kind::Request,
            json!({ "method": "POST", "path": "/api/v1/stage/move_to", "params": { "x": 1000.0, "y": -500.0 }, "status": 201 }),
        )
        .by("olga", session, address),
        Event::new(Kind::Workflow, json!({ "run": "slide 3", "state": "started" })),
        Event::new(Kind::Workflow, json!({ "run": "slide 3", "state": "finished", "frames": 48 })),
        Event::new(Kind::Logout, json!({})).by("olga", session, address),
    ];
    for event in events {
        log.record(event);
        clock.advance(Duration::from_secs(2));
    }
    drop(log);
    let verified = verify(folder.path())?;
    println!(
        "written: {} entries in {} file(s), across the month's end; the last hash {}",
        verified.entries,
        verified.files,
        &verified.head[..16]
    );
    for entry in std::fs::read_dir(folder.path())? {
        let path = entry?.path();
        let text = std::fs::read_to_string(&path)?;
        println!("  {}: {} lines", path.display(), text.lines().count());
    }

    let november = folder.path().join("2026-11.jsonl");
    let original = std::fs::read_to_string(&november)?;
    let tamper = |what: &str, text: String| -> Result<(), Box<dyn Error>> {
        std::fs::write(&november, text)?;
        match verify(folder.path()) {
            Ok(_) => println!("{what}: not found!"),
            Err(broken) => println!("{what}: found at {broken}"),
        }
        std::fs::write(&november, &original)?;
        Ok(())
    };
    tamper(
        "the frames edited",
        original.replace("\"frames\":48", "\"frames\":12"),
    )?;
    let lines: Vec<&str> = original.lines().collect();
    let cut: String = (lines.iter().enumerate())
        .filter(|(k, _)| *k != 0)
        .map(|(_, line)| format!("{line}\n"))
        .collect();
    tamper("the first line removed", cut)?;
    let mut moved = lines.clone();
    moved.swap(0, 1);
    tamper(
        "two lines swapped",
        moved.iter().map(|line| format!("{line}\n")).collect(),
    )?;
    println!("restored: {} entries", verify(folder.path())?.entries);
    Ok(())
}
