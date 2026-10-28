//! Who may do what (ADR-0066): every route of the OpenAPI document and of SpectrumIQ's own is
//! classified; the decision for each class and caller is architecture §10's matrix; and, over
//! HTTP with `[auth] enforce`, a viewer reads but doesn't change, an operator changes the
//! instrument only with the control token, stops without it, and can't take it from another; an
//! admin takes it over; a silent holder loses it; and the local console logs in once per secret,
//! from this computer only. REQ-SEC-002, REQ-SEC-004.

mod common;

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::time::Duration;

use serde_json::{Value, json};
use siq_auth::{Role, Session, SessionPolicy, SessionStore, User, UserStore};
use siq_server::Service;
use siq_server::authz::{Class, classify, decide};
use teta_wot::http::axum::Router;
use teta_wot::http::axum::body::{Body, to_bytes};
use teta_wot::http::axum::extract::ConnectInfo;
use teta_wot::http::axum::http::{Method, Request, StatusCode, header};
use teta_wot::testing::TestClient;
use tower::ServiceExt;

/// SpectrumIQ's own routes, with their parameters filled in.
const OWN: [(&str, &str); 19] = [
    ("GET", "/"),
    ("GET", "/index.html"),
    ("GET", "/assets/index.js"),
    ("GET", "/favicon.svg"),
    ("GET", "/third-party-licences.txt"),
    ("GET", "/api/siq/system/health"),
    ("GET", "/api/siq/system/metrics"),
    ("GET", "/api/siq/system/memory"),
    ("POST", "/api/siq/auth/login"),
    ("POST", "/api/siq/auth/logout"),
    ("POST", "/api/siq/auth/local"),
    ("GET", "/api/siq/auth/me"),
    ("GET", "/api/siq/control"),
    ("POST", "/api/siq/control/request"),
    ("POST", "/api/siq/control/release"),
    ("POST", "/api/siq/control/heartbeat"),
    ("POST", "/api/siq/control/take_over"),
    ("GET", "/api/siq/data/1/manifest"),
    ("GET", "/api/siq/data/1/stitched/0/0/0/0/0_0.jpeg"),
];

#[tokio::test]
async fn every_route_is_classified() {
    let document = siq_server::openapi().await.unwrap();
    let mut classes = BTreeMap::new();
    let mut unclassified = Vec::new();
    for (path, item) in document["paths"].as_object().unwrap() {
        let filled = path.replace("{id}", "1").replace("{blob_id}", "1");
        for method in item.as_object().unwrap().keys() {
            let Ok(method) = method.to_uppercase().parse::<Method>() else {
                continue;
            };
            match classify(&method, &filled) {
                Some(class) if method != Method::GET => {
                    classes.insert(format!("{method} {path}"), format!("{class:?}"));
                }
                Some(_) => {}
                None => unclassified.push(format!("{method} {path}")),
            }
        }
    }
    for (method, path) in OWN {
        if classify(&method.parse().unwrap(), path).is_none() {
            unclassified.push(format!("{method} {path}"));
        }
    }
    assert!(unclassified.is_empty(), "unclassified: {unclassified:#?}");
    // Every change, by its class: a new affordance shows here.
    insta::assert_json_snapshot!("route_classes", classes);
    assert_eq!(
        classify(&Method::GET, "/api/v1/camera/ws"),
        Some(Class::Read)
    );
    assert_eq!(classify(&Method::POST, "/api/v1/nothing/here"), None);
}

#[test]
fn the_decision_is_architecture_s_matrix() {
    let clock = std::sync::Arc::new(siq_types::ManualClock::new());
    let sessions = SessionStore::new(clock, SessionPolicy::default());
    let session = |role| -> Session {
        let user = User {
            name: format!("{role}"),
            role,
            created: chrono::Utc::now(),
            changed: chrono::Utc::now(),
        };
        sessions.create(&user, None).unwrap()
    };
    let (viewer, operator, admin) = (
        session(Role::Viewer),
        session(Role::Operator),
        session(Role::Admin),
    );
    // Unauthenticated, viewer, operator, operator with the token, admin, admin with the token.
    let callers = [
        (None, false),
        (Some(&viewer), false),
        (Some(&operator), false),
        (Some(&operator), true),
        (Some(&admin), false),
        (Some(&admin), true),
    ];
    let matrix = [
        (Class::Public, [true, true, true, true, true, true]),
        (Class::Read, [false, true, true, true, true, true]),
        (Class::Stop, [false, false, true, true, true, true]),
        (Class::Processing, [false, false, true, true, true, true]),
        (Class::Control, [false, false, true, true, true, true]),
        (Class::Hardware, [false, false, false, true, false, true]),
        (Class::Admin, [false, false, false, false, true, true]),
    ];
    for (class, allowed) in matrix {
        for ((caller, holds), allowed) in callers.iter().zip(allowed) {
            let decided = decide(Some(class), *caller, *holds);
            assert_eq!(
                decided.is_ok(),
                allowed,
                "{class:?} for {caller:?}, token {holds}"
            );
            if let Err((status, _)) = decided {
                let expected = match caller {
                    None => StatusCode::UNAUTHORIZED,
                    Some(_) => StatusCode::FORBIDDEN,
                };
                assert_eq!(status, expected);
            }
        }
    }
    assert!(
        decide(None, Some(&admin), true).is_err(),
        "an unclassified change"
    );
}

/// The service with `[auth] enforce`, a token timeout of a second, and an admin, two operators,
/// a viewer and the local console's user; its Things started.
async fn rig() -> (Router, (TestClient, tempfile::TempDir)) {
    let settings = tempfile::tempdir().unwrap();
    let mut config = common::config();
    config.server.settings = settings.path().to_owned();
    config.auth.enforce = true;
    config.auth.control_timeout_s = 1;
    config.auth.local_user = Some("console".to_owned());
    let users = UserStore::open(settings.path().join("users.json")).unwrap();
    for (name, role) in [
        ("ada", Role::Admin),
        ("olga", Role::Operator),
        ("otto", Role::Operator),
        ("vic", Role::Viewer),
        ("console", Role::Operator),
    ] {
        users.add(name, "correct horse", role).unwrap();
    }
    let service = Service::build(config).await.unwrap();
    let routes = service.router();
    let things = TestClient::start(service.into_parts().0).await.unwrap();
    (routes, (things, settings))
}

/// `method` on `path` with `body`, as `cookie` if given, from `from`: its status and answer.
async fn send(
    routes: &Router,
    (method, path): (&str, &str),
    body: Option<Value>,
    cookie: Option<&str>,
    from: &str,
) -> (StatusCode, Value, Option<String>) {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header(header::HOST, "127.0.0.1:5000");
    if let Some(cookie) = cookie {
        request = request.header(header::COOKIE, cookie);
    }
    let body = match body {
        Some(body) => {
            request = request.header(header::CONTENT_TYPE, "application/json");
            Body::from(body.to_string())
        }
        None => Body::empty(),
    };
    let mut request = request.body(body).unwrap();
    let client: SocketAddr = format!("{from}:50123").parse().unwrap();
    request.extensions_mut().insert(ConnectInfo(client));
    let response = routes.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let cookie = (response.headers().get(header::SET_COOKIE))
        .map(|set| set.to_str().unwrap().split(';').next().unwrap().to_owned());
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        cookie,
    )
}

const LAN: &str = "192.168.1.20";

/// Logs `name` in: its cookie.
async fn login(routes: &Router, name: &str) -> String {
    let body = json!({ "name": name, "password": "correct horse" });
    let (status, _, cookie) = send(
        routes,
        ("POST", "/api/siq/auth/login"),
        Some(body),
        None,
        LAN,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    cookie.unwrap()
}

const EXPOSURE: (&str, &str) = ("PUT", "/api/v1/camera/exposure_ms");
const HALT: (&str, &str) = ("POST", "/api/v1/stage/halt");
const REQUEST: (&str, &str) = ("POST", "/api/siq/control/request");
const CONTROL: (&str, &str) = ("GET", "/api/siq/control");

#[tokio::test]
async fn the_token_decides_who_changes_the_instrument() {
    let (routes, _kept) = rig().await;
    // Without a session, only the public routes.
    let position = ("GET", "/api/v1/stage/position");
    let (status, _, _) = send(&routes, position, None, None, LAN).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let health = ("GET", "/api/siq/system/health");
    assert_eq!(
        send(&routes, health, None, None, LAN).await.0,
        StatusCode::OK
    );

    let [ada, olga, otto, vic] = [
        login(&routes, "ada").await,
        login(&routes, "olga").await,
        login(&routes, "otto").await,
        login(&routes, "vic").await,
    ];
    let twenty = Some(json!(20.0));
    // A viewer reads, and changes nothing, not even a stop.
    assert_eq!(
        send(&routes, position, None, Some(&vic), LAN).await.0,
        StatusCode::OK
    );
    let (status, refused, _) = send(&routes, EXPOSURE, twenty.clone(), Some(&vic), LAN).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(
        refused["detail"],
        "an operator's to do, and vic is a viewer"
    );
    assert_eq!(
        send(&routes, HALT, None, Some(&vic), LAN).await.0,
        StatusCode::FORBIDDEN
    );

    // An operator needs the token, which then is hers alone.
    let (status, refused, _) = send(&routes, EXPOSURE, twenty.clone(), Some(&olga), LAN).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(
        refused["detail"]
            .as_str()
            .unwrap()
            .contains("control token")
    );
    let (status, held, _) = send(&routes, REQUEST, None, Some(&olga), LAN).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(held["holder"]["user"], "olga");
    let (status, _, _) = send(&routes, EXPOSURE, twenty.clone(), Some(&olga), LAN).await;
    assert!(status.is_success(), "{status}");
    let (status, refused, _) = send(&routes, REQUEST, None, Some(&otto), LAN).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(refused["detail"].as_str().unwrap().contains("held by olga"));
    let (status, _, _) = send(&routes, EXPOSURE, twenty.clone(), Some(&otto), LAN).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    // Stopping needs no token.
    assert!(
        send(&routes, HALT, None, Some(&otto), LAN)
            .await
            .0
            .is_success()
    );
    // Processing needs an operator; deleting data, an admin.
    let auto = ("PUT", "/api/v1/jobs/auto_stitch");
    assert!(
        send(&routes, auto, Some(json!(true)), Some(&otto), LAN)
            .await
            .0
            .is_success()
    );
    let delete = ("POST", "/api/v1/datasets/delete");
    let id = Some(json!({ "id": "019a0000-0000-7000-8000-000000000000" }));
    assert_eq!(
        send(&routes, delete, id, Some(&olga), LAN).await.0,
        StatusCode::FORBIDDEN
    );

    // An operator can't take it over; an admin can.
    let take = ("POST", "/api/siq/control/take_over");
    assert_eq!(
        send(&routes, take, None, Some(&otto), LAN).await.0,
        StatusCode::FORBIDDEN
    );
    let (status, held, _) = send(&routes, take, None, Some(&ada), LAN).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(held["holder"]["user"], "ada");
    let (status, _, _) = send(&routes, EXPOSURE, twenty.clone(), Some(&olga), LAN).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "no longer hers");
    let release = ("POST", "/api/siq/control/release");
    assert_eq!(
        send(&routes, release, None, Some(&olga), LAN).await.0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        send(&routes, release, None, Some(&ada), LAN).await.0,
        StatusCode::NO_CONTENT
    );
    let (_, state, _) = send(&routes, CONTROL, None, Some(&vic), LAN).await;
    assert_eq!(state["holder"], Value::Null);
}

#[tokio::test]
async fn a_silent_holder_loses_the_token() {
    let (routes, (things, _settings)) = rig().await;
    let [olga, otto] = [login(&routes, "olga").await, login(&routes, "otto").await];
    send(&routes, REQUEST, None, Some(&olga), LAN).await;
    // Her requests are her heartbeat: half a second apart, she keeps it.
    for _ in 0..3 {
        tokio::time::sleep(Duration::from_millis(500)).await;
        let (_, me, _) = send(&routes, ("GET", "/api/siq/auth/me"), None, Some(&olga), LAN).await;
        assert_eq!(me["holds_token"], true);
    }
    assert_eq!(
        things.get("/api/v1/control/holder").await.json(),
        json!("olga")
    );
    // Then silent for longer than the timeout: the Thing's ticker lets it go.
    tokio::time::sleep(Duration::from_millis(2500)).await;
    assert_eq!(
        things.get("/api/v1/control/holder").await.json(),
        Value::Null
    );
    let (status, held, _) = send(&routes, REQUEST, None, Some(&otto), LAN).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(held["holder"]["user"], "otto");
}

#[tokio::test]
async fn the_local_console_logs_in_once_a_secret_from_this_computer() {
    let (routes, (_things, settings)) = rig().await;
    let file = settings.path().join("local-login");
    let local = ("POST", "/api/siq/auth/local");
    let secret = std::fs::read_to_string(&file).unwrap();
    let body = Some(json!({ "secret": secret }));
    let (status, _, _) = send(&routes, local, body.clone(), None, LAN).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "not from another computer");
    let (status, me, cookie) = send(&routes, local, body.clone(), None, "127.0.0.1").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        (me["name"].as_str(), me["local"].as_bool()),
        (Some("console"), Some(true))
    );
    // Used once: the file holds the next secret.
    let (status, _, _) = send(&routes, local, body, None, "127.0.0.1").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let next = Some(json!({ "secret": std::fs::read_to_string(&file).unwrap() }));
    assert_eq!(
        send(&routes, local, next, None, "127.0.0.1").await.0,
        StatusCode::OK
    );

    // The console, an operator, takes the token over from another operator.
    let olga = login(&routes, "olga").await;
    send(&routes, REQUEST, None, Some(&olga), LAN).await;
    let take = ("POST", "/api/siq/control/take_over");
    let console = cookie.unwrap();
    let (status, held, _) = send(&routes, take, None, Some(&console), "127.0.0.1").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        (
            held["holder"]["user"].as_str(),
            held["holder"]["local"].as_bool()
        ),
        (Some("console"), Some(true))
    );
}
