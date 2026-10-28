//! Sessions over HTTP (ADR-0065): a login gives a cookie that carries the session, `me` says who it
//! is, and a logout ends it; wrong passwords are refused, and past the limit the right one too; a
//! mutating request from another site's page is refused, on SpectrumIQ's routes and teta-wot's;
//! and a user removed or given another role by the CLI is so at once. REQ-SEC-001.

mod common;

use std::net::SocketAddr;

use serde_json::{Value, json};
use siq_auth::{Role, UserStore};
use siq_server::Service;
use teta_wot::http::axum::Router;
use teta_wot::http::axum::body::{Body, to_bytes};
use teta_wot::http::axum::extract::ConnectInfo;
use teta_wot::http::axum::http::{Request, StatusCode, header};
use teta_wot::testing::TestClient;
use tower::ServiceExt;

/// The service's routes, its Things started, with an admin `ada` and a viewer `bob` in its
/// settings.
async fn rig() -> (Router, UserStore, (TestClient, tempfile::TempDir)) {
    let settings = tempfile::tempdir().unwrap();
    let mut config = common::config();
    config.server.settings = settings.path().to_owned();
    let users = UserStore::open(settings.path().join("users.json")).unwrap();
    users.add("ada", "correct horse", Role::Admin).unwrap();
    users.add("bob", "battery staple", Role::Viewer).unwrap();
    let service = Service::build(config).await.unwrap();
    let routes = service.router();
    let things = TestClient::start(service.into_parts().0).await.unwrap();
    (routes, users, (things, settings))
}

/// `method` on `path`, from 192.168.1.20 to 127.0.0.1:5000, with `cookie` and `origin` if given.
async fn send(
    routes: &Router,
    method: &str,
    path: &str,
    body: Option<Value>,
    cookie: Option<&str>,
    origin: Option<&str>,
) -> (StatusCode, header::HeaderMap, Value) {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header(header::HOST, "127.0.0.1:5000");
    if let Some(cookie) = cookie {
        request = request.header(header::COOKIE, cookie);
    }
    if let Some(origin) = origin {
        request = request.header(header::ORIGIN, origin);
    }
    let body = match body {
        Some(body) => {
            request = request.header(header::CONTENT_TYPE, "application/json");
            Body::from(body.to_string())
        }
        None => Body::empty(),
    };
    let mut request = request.body(body).unwrap();
    let client: SocketAddr = "192.168.1.20:50123".parse().unwrap();
    request.extensions_mut().insert(ConnectInfo(client));
    let response = routes.clone().oneshot(request).await.unwrap();
    let (status, headers) = (response.status(), response.headers().clone());
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, headers, value)
}

/// Logs `name` in with `password`: the cookie to send back.
async fn login(routes: &Router, name: &str, password: &str) -> String {
    let body = json!({ "name": name, "password": password });
    let (status, headers, _) = send(
        routes,
        "POST",
        "/api/siq/auth/login",
        Some(body),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let set = headers[header::SET_COOKIE].to_str().unwrap();
    set.split(';').next().unwrap().to_owned()
}

#[tokio::test]
async fn a_login_s_cookie_carries_the_session_until_the_logout() {
    let (routes, _users, _kept) = rig().await;
    let body = json!({ "name": "ada", "password": "correct horse" });
    let (status, headers, me) = send(
        &routes,
        "POST",
        "/api/siq/auth/login",
        Some(body),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        (me["name"].as_str(), me["role"].as_str()),
        (Some("ada"), Some("admin"))
    );
    let set = headers[header::SET_COOKIE].to_str().unwrap();
    assert!(set.starts_with("siq_session="), "{set}");
    assert!(
        set.ends_with("; HttpOnly; SameSite=Strict; Path=/; Max-Age=43200"),
        "{set}"
    );
    let cookie = set.split(';').next().unwrap();

    let (status, _, me) = send(&routes, "GET", "/api/siq/auth/me", None, Some(cookie), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(me["name"], "ada");
    assert!(me["expires"].as_str().unwrap() > me["since"].as_str().unwrap());
    let (status, _, _) = send(&routes, "GET", "/api/siq/auth/me", None, None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let (status, headers, _) = send(
        &routes,
        "POST",
        "/api/siq/auth/logout",
        None,
        Some(cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(
        headers[header::SET_COOKIE]
            .to_str()
            .unwrap()
            .contains("Max-Age=0")
    );
    let (status, _, _) = send(&routes, "GET", "/api/siq/auth/me", None, Some(cookie), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn wrong_passwords_are_refused_and_past_the_limit_the_right_one_too() {
    let (routes, _users, _kept) = rig().await;
    for password in ["wrong", "wronger", "battery stapler", "staple battery", "?"] {
        let body = json!({ "name": "bob", "password": password });
        let (status, _, refused) = send(
            &routes,
            "POST",
            "/api/siq/auth/login",
            Some(body),
            None,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(refused["detail"], "the name or the password is wrong");
    }
    let body = json!({ "name": "bob", "password": "battery staple" });
    let (status, headers, _) = send(
        &routes,
        "POST",
        "/api/siq/auth/login",
        Some(body),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    let wait: u64 = headers[header::RETRY_AFTER]
        .to_str()
        .unwrap()
        .parse()
        .unwrap();
    assert!((295..=300).contains(&wait), "{wait} s");
    // Another name, from the same address, may still log in.
    login(&routes, "ada", "correct horse").await;
    let body = json!({ "name": "nobody", "password": "correct horse" });
    let (status, _, _) = send(
        &routes,
        "POST",
        "/api/siq/auth/login",
        Some(body),
        None,
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "an unknown name, as a wrong password"
    );
}

#[tokio::test]
async fn changes_from_another_site_s_page_are_refused() {
    let (routes, _users, _kept) = rig().await;
    let cookie = login(&routes, "ada", "correct horse").await;
    let evil = Some("http://evil.example");
    // A Thing's property, behind teta-wot's routes, and a SpectrumIQ route.
    let off = Some(json!(false));
    let property = "/api/v1/jobs/auto_stitch";
    let (status, _, refused) =
        send(&routes, "PUT", property, off.clone(), Some(&cookie), evil).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(
        refused["detail"],
        "a change must come from the service's own pages"
    );
    let (status, _, _) = send(
        &routes,
        "POST",
        "/api/siq/auth/logout",
        None,
        Some(&cookie),
        evil,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    // Reading from anywhere, and changing from the service's own pages or a script, pass.
    let (status, _, _) = send(&routes, "GET", property, None, Some(&cookie), evil).await;
    assert_eq!(status, StatusCode::OK);
    let own = Some("http://127.0.0.1:5000");
    let (status, _, _) = send(&routes, "PUT", property, off.clone(), Some(&cookie), own).await;
    assert!(status.is_success(), "{status}");
    let (status, _, _) = send(&routes, "PUT", property, off, Some(&cookie), None).await;
    assert!(status.is_success(), "{status}");
}

#[tokio::test]
async fn the_cli_s_changes_to_a_user_reach_its_sessions_at_once() {
    let (routes, users, _kept) = rig().await;
    let cookie = login(&routes, "bob", "battery staple").await;
    users.set_role("bob", Role::Operator).unwrap();
    let (_, _, me) = send(
        &routes,
        "GET",
        "/api/siq/auth/me",
        None,
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(me["role"], "operator");
    users.remove("bob").unwrap();
    let (status, _, _) = send(
        &routes,
        "GET",
        "/api/siq/auth/me",
        None,
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}
