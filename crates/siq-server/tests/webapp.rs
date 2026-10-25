//! The web app served at `/` beside the API (ADR-0036), from a build in a folder: the page and its
//! assets with their caching, a placeholder without a build, paths that would leave the build
//! refused, and the API's routes untouched.

mod common;

use std::path::Path;

use common::config;
use http_body_util::BodyExt;
use siq_server::Service;
use siq_server::webapp::WebApp;
use teta_wot::http::axum::Router;
use teta_wot::http::axum::body::Body;
use teta_wot::http::axum::http::{Request, StatusCode, header};
use tower::ServiceExt;

/// A status, a header, and the body as text.
async fn get(router: &Router, path: &str) -> (StatusCode, String, String, String) {
    let request = Request::get(path).body(Body::empty()).unwrap();
    let response = router.clone().oneshot(request).await.unwrap();
    let header = |name| {
        let value = response.headers().get(name);
        value.map_or("", |value| value.to_str().unwrap()).to_owned()
    };
    let (kind, cache) = (header(header::CONTENT_TYPE), header(header::CACHE_CONTROL));
    let status = response.status();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        kind,
        cache,
        String::from_utf8_lossy(&body).into_owned(),
    )
}

fn build(dir: &Path) {
    std::fs::create_dir_all(dir.join("assets")).unwrap();
    std::fs::write(dir.join("index.html"), "<div id=\"app\"></div>").unwrap();
    std::fs::write(
        dir.join("assets/index-4f2a.js"),
        "console.log('SpectrumIQ')",
    )
    .unwrap();
    std::fs::write(dir.join("favicon.svg"), "<svg/>").unwrap();
}

async fn serve(source: WebApp) -> Router {
    Service::build(config())
        .await
        .unwrap()
        .with_webapp(source)
        .router()
}

#[tokio::test]
async fn the_app_is_served_at_the_root_with_its_assets() {
    let folder = tempfile::tempdir().unwrap();
    let dist = folder.path().join("dist");
    build(&dist);
    let router = serve(WebApp::Dir(dist)).await;

    for path in ["/", "/index.html"] {
        let (status, kind, cache, body) = get(&router, path).await;
        assert_eq!(status, StatusCode::OK, "{path}");
        assert_eq!(
            (kind.as_str(), cache.as_str()),
            ("text/html; charset=utf-8", "no-store")
        );
        assert_eq!(body, "<div id=\"app\"></div>");
    }
    let (status, kind, cache, body) = get(&router, "/assets/index-4f2a.js").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(kind, "text/javascript; charset=utf-8");
    assert_eq!(cache, "public, max-age=31536000, immutable");
    assert_eq!(body, "console.log('SpectrumIQ')");
    let (status, kind, cache, _) = get(&router, "/favicon.svg").await;
    assert_eq!((status, kind.as_str()), (StatusCode::OK, "image/svg+xml"));
    assert_eq!(cache, "no-cache");

    // Missing files answer as teta-wot does.
    let (status, _, _, body) = get(&router, "/assets/missing.js").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body, r#"{"detail":"Not Found"}"#);
}

#[tokio::test]
async fn a_path_that_would_leave_the_build_is_refused() {
    let folder = tempfile::tempdir().unwrap();
    let dist = folder.path().join("dist");
    build(&dist);
    std::fs::write(folder.path().join("secret.txt"), "not for clients").unwrap();
    let router = serve(WebApp::Dir(dist)).await;
    for path in [
        "/assets/..%2F..%2Fsecret.txt",
        "/assets/%2E%2E/%2E%2E/secret.txt",
    ] {
        let (status, _, _, body) = get(&router, path).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path}");
        assert!(!body.contains("not for clients"), "{path}");
    }
}

#[tokio::test]
async fn without_a_build_the_root_says_how_to_build_one() {
    let empty = tempfile::tempdir().unwrap();
    let router = serve(WebApp::Dir(empty.path().to_owned())).await;
    let (status, kind, cache, body) = get(&router, "/").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        (kind.as_str(), cache.as_str()),
        ("text/html; charset=utf-8", "no-store")
    );
    assert!(
        body.contains("npm run build") && body.contains("--webapp-dir"),
        "{body}"
    );
}

#[tokio::test]
async fn the_api_is_untouched() {
    let folder = tempfile::tempdir().unwrap();
    build(folder.path());
    let router = serve(WebApp::Dir(folder.path().to_owned())).await;
    let (status, kind, _, body) = get(&router, "/api/v1/things/").await;
    assert_eq!(status, StatusCode::OK);
    assert!(kind.starts_with("application/json"), "{kind}");
    assert!(body.contains("\"camera\""), "{body}");
    let (status, _, _, body) = get(&router, "/api/siq/system/health").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("\"status\":\"ok\""), "{body}");
    let (status, _, _, _) = get(&router, "/api/v1/no_such_thing/").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
