//! The service's audit trail (ADR-0067), with `[auth] enforce`: logins, failed ones, the token's
//! changes, a change with its parameters and its answer, a refusal, a logout, a job's end and a
//! workflow's run all recorded, by whom and from where, but not a read that is a POST; the query
//! an admin's; and the trail's chain intact. REQ-SEC-003, REQ-NFR-006.

mod common;

use std::net::SocketAddr;
use std::time::Duration;

use serde_json::{Value, json};
use siq_auth::{Role, UserStore};
use siq_jobs::{Job, JobCtx, JobError, JobOutput, Priority, Resources};
use siq_server::Service;
use teta_wot::http::axum::Router;
use teta_wot::http::axum::body::{Body, to_bytes};
use teta_wot::http::axum::extract::ConnectInfo;
use teta_wot::http::axum::http::{Request, StatusCode, header};
use teta_wot::testing::TestClient;
use tower::ServiceExt;

/// A job that does nothing.
struct Nap;

impl Job for Nap {
    fn kind(&self) -> &str {
        "nap"
    }

    fn estimate(&self) -> Resources {
        Resources::default()
    }

    fn run(&mut self, _ctx: &JobCtx) -> Result<JobOutput, JobError> {
        Ok(JobOutput::default())
    }
}

async fn send(
    routes: &Router,
    (method, path): (&str, &str),
    body: Option<Value>,
    cookie: Option<&str>,
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
    let client: SocketAddr = "192.168.1.20:50123".parse().unwrap();
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

async fn login(routes: &Router, name: &str, password: &str) -> (StatusCode, Option<String>) {
    let body = json!({ "name": name, "password": password });
    let (status, _, cookie) = send(routes, ("POST", "/api/siq/auth/login"), Some(body), None).await;
    (status, cookie)
}

#[tokio::test]
async fn who_did_what_is_recorded_and_an_admin_reads_it() {
    let folders = [tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap()];
    let mut config = common::config();
    config.server.settings = folders[0].path().to_owned();
    config.server.data_root = folders[1].path().to_owned();
    config.server.min_free_gb = 0.0;
    config.auth.enforce = true;
    let trail = config.server.audit.clone();
    let users = UserStore::open(folders[0].path().join("users.json")).unwrap();
    users.add("ada", "correct horse", Role::Admin).unwrap();
    users.add("olga", "correct horse", Role::Operator).unwrap();
    users.add("vic", "correct horse", Role::Viewer).unwrap();
    let service = Service::build(config).await.unwrap();
    let routes = service.router();
    let instrument = service.instrument().clone();
    let things = TestClient::start(service.into_parts().0).await.unwrap();

    assert_eq!(
        login(&routes, "olga", "wrong horse").await.0,
        StatusCode::UNAUTHORIZED
    );
    let ada = login(&routes, "ada", "correct horse").await.1.unwrap();
    let olga = login(&routes, "olga", "correct horse").await.1.unwrap();
    let vic = login(&routes, "vic", "correct horse").await.1.unwrap();
    send(
        &routes,
        ("POST", "/api/siq/control/request"),
        None,
        Some(&olga),
    )
    .await;
    let exposure = ("PUT", "/api/v1/camera/exposure_ms");
    let (status, _, _) = send(&routes, exposure, Some(json!(25.0)), Some(&olga)).await;
    assert!(status.is_success());
    let (status, _, _) = send(&routes, exposure, Some(json!(5.0)), Some(&vic)).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let summary = ("POST", "/api/v1/datasets/summary");
    send(&routes, summary, Some(json!({})), Some(&vic)).await;
    send(&routes, ("POST", "/api/siq/auth/logout"), None, Some(&olga)).await;
    // A job, and a workflow's run of one snap.
    let id = instrument
        .jobs
        .submit(Box::new(Nap), Priority::Background, None);
    instrument.jobs.wait(id, Duration::from_secs(10)).unwrap();
    let run = json!({ "name": "one snap", "steps": [{ "step": "snap" }] });
    let invocation = things.post_json("/api/v1/workflow/run", Some(&run)).await;
    let ended = common::finished(&things, invocation).await;
    assert_eq!(ended["status"], "completed", "{ended}");
    tokio::time::sleep(Duration::from_millis(200)).await;

    // The query is an admin's.
    let audit = ("GET", "/api/siq/audit?limit=200");
    assert_eq!(
        send(&routes, audit, None, Some(&vic)).await.0,
        StatusCode::FORBIDDEN
    );
    let (status, page, _) = send(&routes, audit, None, Some(&ada)).await;
    assert_eq!(status, StatusCode::OK);
    let entries = page["entries"].as_array().unwrap();
    let find = |test: &dyn Fn(&Value) -> bool| entries.iter().find(|e| test(e)).cloned();
    let failed = find(&|e| e["kind"] == "login_failed").unwrap();
    assert_eq!(
        (
            failed["detail"]["name"].as_str(),
            failed["address"].as_str()
        ),
        (Some("olga"), Some("192.168.1.20"))
    );
    assert!(find(&|e| e["kind"] == "login" && e["user"] == "ada").is_some());
    let token =
        find(&|e| e["kind"] == "token" && e["detail"]["reason"]["kind"] == "requested").unwrap();
    assert_eq!(token["user"], "olga");
    let change = find(&|e| e["kind"] == "request" && e["user"] == "olga").unwrap();
    let detail = &change["detail"];
    assert_eq!(
        (detail["method"].as_str(), detail["path"].as_str()),
        (Some("PUT"), Some("/api/v1/camera/exposure_ms"))
    );
    assert_eq!(
        (detail["params"].as_f64(), detail["class"].as_str()),
        (Some(25.0), Some("hardware"))
    );
    assert!(change["session"].as_str().unwrap().len() == 8);
    let refused = find(&|e| e["kind"] == "refused" && e["user"] == "vic").unwrap();
    assert_eq!(refused["detail"]["status"], 403);
    assert!(find(&|e| e["detail"]["path"] == summary.1).is_none());
    assert!(find(&|e| e["kind"] == "logout" && e["user"] == "olga").is_some());
    assert!(find(&|e| e["kind"] == "token" && e["detail"]["reason"]["kind"] == "ended").is_some());
    let job = find(&|e| {
        e["kind"] == "job" && e["detail"]["id"] == id.0 && e["detail"]["state"] == "done"
    });
    assert!(job.is_some(), "{entries:#?}");
    let started = find(&|e| e["kind"] == "workflow" && e["detail"]["state"] == "started").unwrap();
    assert_eq!(started["detail"]["run"], "one snap");
    let finished =
        find(&|e| e["kind"] == "workflow" && e["detail"]["state"] == "finished").unwrap();
    assert_eq!(finished["detail"]["frames"], 1);
    // No password anywhere.
    assert!(!page.to_string().contains("correct horse"));

    instrument.auth.audit.flush();
    let verified = siq_auth::audit::verify(&trail).unwrap();
    assert_eq!(
        verified.entries as usize,
        page["total"].as_u64().unwrap() as usize
    );
}
