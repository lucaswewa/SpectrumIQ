//! The Hardware Control Token and the local console's login (ADR-0066), on a paused clock: the
//! token requested, refused to another, released, taken over, timed out after its holder's
//! silence, and let go when its session ends, each change watched; the local secret used once,
//! in a file whose access list holds this account alone. REQ-SEC-002, REQ-SEC-004.

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use siq_auth::{
    ControlError, ControlManager, LocalLogin, Reason, Role, Session, SessionPolicy, SessionStore,
    User,
};
use siq_types::ManualClock;

const SECOND: Duration = Duration::from_secs(1);

fn session(sessions: &SessionStore, name: &str, role: Role, local: bool) -> Session {
    let user = User {
        name: name.to_owned(),
        role,
        created: Utc::now(),
        changed: Utc::now(),
    };
    match local {
        true => sessions.create_local(&user, None).unwrap(),
        false => sessions.create(&user, None).unwrap(),
    }
}

#[test]
fn the_token_is_held_by_one_session_and_handed_over() {
    let clock = Arc::new(ManualClock::new());
    let sessions = SessionStore::new(clock.clone(), SessionPolicy::default());
    let control = ControlManager::new(clock.clone(), 30 * SECOND);
    let mut watch = control.watch();
    let olga = session(&sessions, "olga", Role::Operator, false);
    let otto = session(&sessions, "otto", Role::Operator, false);
    let ada = session(&sessions, "ada", Role::Admin, false);

    assert_eq!(control.request(&olga).unwrap().user, "olga");
    assert!(watch.has_changed().unwrap());
    assert_eq!(watch.borrow_and_update().reason, Reason::Requested);
    // Asking again keeps it; another is refused, told who holds it.
    assert_eq!(control.request(&olga).unwrap().user, "olga");
    let Err(ControlError::Held(holder)) = control.request(&otto) else {
        panic!("otto was given it")
    };
    assert_eq!(holder.user, "olga");
    assert_eq!(control.release(&otto.id), Err(ControlError::NotHolder));
    assert!(control.heartbeat(&olga.id) && !control.heartbeat(&otto.id));

    // Released, then otto's; then an admin takes it over.
    control.release(&olga.id).unwrap();
    assert_eq!(watch.borrow_and_update().reason, Reason::Released);
    assert_eq!(control.request(&otto).unwrap().user, "otto");
    assert_eq!(control.take_over(&ada).user, "ada");
    let state = watch.borrow_and_update().clone();
    assert_eq!(
        state.reason,
        Reason::TakenOver {
            from: "otto".into()
        }
    );
    assert_eq!(state.holder.unwrap().user, "ada");
    assert!(!control.heartbeat(&otto.id));

    // A logout lets it go.
    control.session_ended(&ada.id);
    assert_eq!(control.holder(), None);
    assert_eq!(watch.borrow_and_update().reason, Reason::Ended);
}

#[test]
fn a_silent_holder_loses_the_token_and_a_heartbeat_keeps_it() {
    let clock = Arc::new(ManualClock::new());
    let sessions = SessionStore::new(clock.clone(), SessionPolicy::default());
    let control = ControlManager::new(clock.clone(), 30 * SECOND);
    let olga = session(&sessions, "olga", Role::Operator, false);
    let console = session(&sessions, "console", Role::Operator, true);
    control.request(&olga).unwrap();
    // A heartbeat every 25 s keeps it for two minutes.
    for _ in 0..5 {
        clock.advance(25 * SECOND);
        assert!(control.heartbeat(&olga.id));
    }
    // 30 s of silence: it goes by itself, watched without anyone asking.
    let mut watch = control.watch();
    clock.advance(30 * SECOND);
    control.expire();
    assert_eq!(watch.borrow_and_update().reason, Reason::TimedOut);
    assert_eq!(control.holder(), None);
    // The local console's session says so.
    assert!(control.request(&console).unwrap().local);
}

#[test]
fn the_local_secret_works_once_and_only_this_account_can_read_it() {
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("settings").join("local-login");
    let login = LocalLogin::create(&path).unwrap();
    let first = std::fs::read_to_string(&path).unwrap();
    assert_eq!(first.len(), 64);
    assert!(!login.exchange("0123456789abcdef").unwrap());
    assert!(login.exchange(&first).unwrap(), "the secret the file holds");
    // Used once: the file holds the next, and the first no longer works.
    let second = std::fs::read_to_string(&path).unwrap();
    assert_ne!(first, second);
    assert!(!login.exchange(&first).unwrap());
    assert!(
        login.exchange(&format!("{second}\r\n")).unwrap(),
        "as read from a file"
    );

    // One entry, this account with full access, and nothing inherited: `icacls` lists each
    // entry as `name:(rights)`, inherited ones with `(I)`.
    let listing = std::process::Command::new("icacls")
        .arg(&path)
        .output()
        .unwrap();
    let listing = String::from_utf8_lossy(&listing.stdout);
    let entries: Vec<&str> = listing.lines().filter(|line| line.contains(":(")).collect();
    let account = std::env::var("USERNAME").unwrap().to_lowercase();
    assert_eq!(entries.len(), 1, "{listing}");
    assert!(entries[0].to_lowercase().contains(&account), "{listing}");
    assert!(
        entries[0].contains("(F)") && !entries[0].contains("(I)"),
        "{listing}"
    );
}
