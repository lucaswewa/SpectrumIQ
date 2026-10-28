//! The audit trail (ADR-0067): entries chained and verified, queried newest first by user, kind and
//! time; an edit, a removal, a reordering and an edit with its hash made again all found at their
//! line; and a month's file continuing the last month's chain, across a restart too.
//! REQ-SEC-003, REQ-NFR-006.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use siq_auth::audit::{Filter, GENESIS, verify};
use siq_auth::{AuditLog, Event, Kind};
use siq_types::ManualClock;

fn at(text: &str) -> DateTime<Utc> {
    text.parse().unwrap()
}

/// A trail in `folder` on a clock starting at `start`, with `n` entries a second apart:
/// alternately ada's logins and bob's requests.
fn trail(folder: &Path, start: &str, n: usize) -> (AuditLog, Arc<ManualClock>) {
    let clock = Arc::new(ManualClock::starting_at(at(start)));
    let log = AuditLog::open(folder, clock.clone()).unwrap();
    record(&log, &clock, n, 0);
    (log, clock)
}

fn record(log: &AuditLog, clock: &ManualClock, n: usize, from: usize) {
    for k in from..from + n {
        let event = match k % 2 {
            0 => Event::new(Kind::Login, json!({ "local": false })),
            _ => Event::new(
                Kind::Request,
                json!({ "method": "PUT", "path": "/api/v1/camera/exposure_ms", "params": 20.0, "status": 204 }),
            ),
        };
        let user = if k % 2 == 0 { "ada" } else { "bob" };
        let session = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        log.record(event.by(user, Some(session), "192.168.1.20".parse().ok()));
        clock.advance(Duration::from_secs(1));
    }
    log.flush();
}

fn lines(path: &Path) -> Vec<String> {
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect()
}

fn write(path: &Path, lines: &[String]) {
    std::fs::write(
        path,
        lines.iter().map(|l| format!("{l}\n")).collect::<String>(),
    )
    .unwrap();
}

#[test]
fn entries_are_chained_and_queried() {
    let folder = tempfile::tempdir().unwrap();
    let (log, _clock) = trail(folder.path(), "2026-10-27T09:00:00Z", 6);
    let verified = verify(folder.path()).unwrap();
    assert_eq!((verified.entries, verified.files), (6, 1));
    let written = lines(&folder.path().join("2026-10.jsonl"));
    let first: Value = serde_json::from_str(&written[0]).unwrap();
    assert_eq!(first["prev_hash"], GENESIS);
    assert_eq!(first["seq"], 1);
    assert_eq!(
        first["session"], "01234567",
        "a session's first digits only"
    );
    assert_eq!(first["address"], "192.168.1.20");
    assert!(written[5].ends_with(&format!(",\"hash\":\"{}\"}}", verified.head)));

    // Newest first, by user, kind and time, a page at a time.
    let page = log.query(&Filter::default()).unwrap();
    assert_eq!((page.total, page.entries[0]["seq"].as_u64()), (6, Some(6)));
    let bob = Filter {
        user: Some("bob".into()),
        ..Filter::default()
    };
    let page = log.query(&bob).unwrap();
    assert_eq!(page.total, 3);
    assert!(page.entries.iter().all(|e| e["kind"] == "request"));
    let logins = Filter {
        kind: Some(Kind::Login),
        from: Some(at("2026-10-27T09:00:01Z")),
        to: Some(at("2026-10-27T09:00:05Z")),
        offset: 1,
        limit: 1,
        ..Filter::default()
    };
    let page = log.query(&logins).unwrap();
    assert_eq!(page.total, 2, "the logins at 2 s and 4 s");
    assert_eq!(page.entries.len(), 1);
    assert_eq!(page.entries[0]["seq"], 3);
}

#[test]
fn an_edit_a_removal_and_a_reordering_are_found_where_they_are() {
    let folder = tempfile::tempdir().unwrap();
    let (log, _clock) = trail(folder.path(), "2026-10-27T09:00:00Z", 6);
    drop(log);
    let path = folder.path().join("2026-10.jsonl");
    let original = lines(&path);
    let broken = |edited: Vec<String>| {
        write(&path, &edited);
        let found = verify(folder.path()).unwrap_err();
        write(&path, &original);
        (found.line, found.why)
    };

    // An edit: the status of a request changed.
    let mut edited = original.clone();
    edited[3] = edited[3].replace("\"status\":204", "\"status\":403");
    assert_eq!(
        broken(edited),
        (4, "its hash isn't its text's: edited".into())
    );
    // A removal, and a reordering.
    let mut removed = original.clone();
    removed.remove(2);
    let follows = "it doesn't follow the line before: one removed or moved";
    assert_eq!(broken(removed), (3, follows.into()));
    let mut reordered = original.clone();
    reordered.swap(1, 2);
    assert_eq!(broken(reordered), (2, follows.into()));
    // An edit whose hash was made again: the next line no longer follows it.
    let mut rehashed = original.clone();
    let text = rehashed[3].replace("bob", "eve");
    let at = text.rfind(",\"hash\":\"").unwrap();
    let body = format!("{}}}", &text[..at]);
    let hash: String = <sha2::Sha256 as sha2::Digest>::digest(body.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    rehashed[3] = format!("{},\"hash\":\"{hash}\"}}", &text[..at]);
    assert_eq!(broken(rehashed), (5, follows.into()));
    assert_eq!(verify(folder.path()).unwrap().entries, 6, "restored");
}

#[test]
fn a_month_s_file_continues_the_last_month_s_chain() {
    let folder = tempfile::tempdir().unwrap();
    // Two entries before midnight at the month's end, two after.
    let (log, clock) = trail(folder.path(), "2026-10-31T23:59:58Z", 4);
    drop(log);
    let october = lines(&folder.path().join("2026-10.jsonl"));
    let november = lines(&folder.path().join("2026-11.jsonl"));
    assert_eq!((october.len(), november.len()), (2, 2));
    let last: Value = serde_json::from_str(&october[1]).unwrap();
    let next: Value = serde_json::from_str(&november[0]).unwrap();
    assert_eq!(next["prev_hash"], last["hash"]);

    // A restart continues the chain.
    let log = AuditLog::open(folder.path(), clock.clone()).unwrap();
    record(&log, &clock, 2, 4);
    let verified = verify(folder.path()).unwrap();
    assert_eq!((verified.entries, verified.files), (6, 2));

    // Without October, November's first line follows nothing.
    std::fs::remove_file(folder.path().join("2026-10.jsonl")).unwrap();
    let found = verify(folder.path()).unwrap_err();
    assert!(
        found.file.ends_with("2026-11.jsonl") && found.line == 1,
        "{found}"
    );
}
