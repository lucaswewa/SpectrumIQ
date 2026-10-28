//! Sessions, failed logins and origins (ADR-0065), on a paused clock: a session ends when idle and
//! when old however busy; failures past the limit wait for the window, an address allowed more;
//! the same-origin check; the session cookie. REQ-SEC-001.

use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use siq_auth::{
    LoginLimiter, Role, SessionId, SessionPolicy, SessionStore, User, clear_cookie, same_origin,
    session_cookie, set_cookie,
};
use siq_types::{Clock, ManualClock};

const MINUTE: Duration = Duration::from_secs(60);

fn user(name: &str, role: Role) -> User {
    User {
        name: name.to_owned(),
        role,
        created: Utc::now(),
        changed: Utc::now(),
    }
}

#[test]
fn a_session_ends_when_idle_and_when_old_however_busy() {
    let clock = Arc::new(ManualClock::new());
    let policy = SessionPolicy {
        idle: 30 * MINUTE,
        absolute: 120 * MINUTE,
    };
    let sessions = SessionStore::new(clock.clone(), policy);
    let ada = sessions.create(&user("ada", Role::Admin), None).unwrap();
    // Busy: a request every 29 minutes keeps it, until two hours.
    for _ in 0..4 {
        clock.advance(29 * MINUTE);
        assert!(sessions.touch(&ada.id).is_some());
    }
    clock.advance(4 * MINUTE);
    assert!(sessions.touch(&ada.id).is_none(), "two hours, however busy");
    // Idle: 30 minutes without a request.
    let bob = sessions.create(&user("bob", Role::Viewer), None).unwrap();
    clock.advance(29 * MINUTE);
    let seen = sessions.touch(&bob.id).unwrap();
    assert_eq!(sessions.expires(&seen), clock.utc(seen.seen + policy.idle));
    clock.advance(30 * MINUTE);
    assert!(sessions.touch(&bob.id).is_none(), "half an hour idle");
    assert!(sessions.list().is_empty());
}

#[test]
fn sessions_end_by_their_user_and_follow_its_role() {
    let clock = Arc::new(ManualClock::new());
    let sessions = SessionStore::new(clock, SessionPolicy::default());
    let address = Some(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 20)));
    let first = sessions
        .create(&user("bob", Role::Viewer), address)
        .unwrap();
    let second = sessions.create(&user("bob", Role::Viewer), None).unwrap();
    let ada = sessions.create(&user("ada", Role::Admin), None).unwrap();
    assert_ne!(first.id, second.id);
    sessions.set_role("bob", Role::Operator);
    assert_eq!(sessions.touch(&first.id).unwrap().role, Role::Operator);
    assert_eq!(sessions.touch(&first.id).unwrap().address, address);
    assert_eq!(sessions.end_user("bob"), 2);
    assert!(sessions.touch(&second.id).is_none());
    assert!(sessions.remove(&ada.id).is_some());
    assert!(sessions.touch(&ada.id).is_none());
}

#[test]
fn failed_logins_past_the_limit_wait_for_the_window() {
    let clock = Arc::new(ManualClock::new());
    let limiter = LoginLimiter::new(clock.clone(), 5, 5 * MINUTE);
    let here = Some(IpAddr::V4(Ipv4Addr::LOCALHOST));
    for _ in 0..5 {
        assert_eq!(limiter.wait("ada", here), None);
        limiter.failed("ada", here);
        clock.advance(MINUTE / 2);
    }
    // Five failures in two and a half minutes: the next waits for the first to leave the window.
    assert_eq!(limiter.wait("ada", here), Some(5 * MINUTE - 5 * MINUTE / 2));
    assert_eq!(
        limiter.wait("bob", here),
        None,
        "another name, from the same address"
    );
    clock.advance(5 * MINUTE / 2);
    assert_eq!(limiter.wait("ada", here), None);
    // A success forgets the name's failures.
    limiter.failed("ada", here);
    limiter.succeeded("ada");
    assert_eq!(limiter.wait("ada", None), None);

    // An address may fail four times as often, over every name.
    let lab = Some(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 7)));
    for k in 0..20 {
        limiter.failed(&format!("user{k}"), lab);
    }
    assert!(limiter.wait("anyone", lab).is_some());
    assert!(limiter.wait("anyone", None).is_none());
}

#[test]
fn mutations_come_from_the_service_s_own_pages() {
    let allowed = vec!["http://localhost:5199/".to_owned()];
    let host = Some("127.0.0.1:5000");
    assert!(same_origin(None, host, &allowed), "a script, not a browser");
    assert!(same_origin(Some("http://127.0.0.1:5000"), host, &allowed));
    assert!(same_origin(Some("HTTP://127.0.0.1:5000"), host, &[]));
    assert!(same_origin(Some("http://LOCALHOST:5199"), host, &allowed));
    assert!(!same_origin(Some("http://127.0.0.1:5001"), host, &allowed));
    assert!(!same_origin(Some("http://evil.example"), host, &allowed));
    assert!(!same_origin(Some("null"), host, &allowed));
    assert!(!same_origin(Some("http://127.0.0.1:5000"), None, &[]));
    assert!(!same_origin(Some("http://127.0.0.1:5000/x"), host, &[]));
}

#[test]
fn the_cookie_carries_the_session_to_the_service_alone() {
    let id = SessionId::random().unwrap();
    assert_eq!(id.as_str().len(), 64);
    assert!(!format!("{id:?}").contains(id.as_str()), "not in the logs");
    let set = set_cookie(&id, 12 * 60 * MINUTE);
    assert_eq!(
        set,
        format!(
            "siq_session={}; HttpOnly; SameSite=Strict; Path=/; Max-Age=43200",
            id.as_str()
        )
    );
    let header = format!("theme=dark; siq_session={}; other=1", id.as_str());
    assert_eq!(session_cookie(&header), Some(id));
    assert_eq!(session_cookie("siq_session=not-an-id"), None);
    assert_eq!(session_cookie("theme=dark"), None);
    assert!(clear_cookie().contains("Max-Age=0"));
}
