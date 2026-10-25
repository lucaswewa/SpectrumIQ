//! The web app, served beside the API (ADR-0036).
//!
//! | Route | What | Cached |
//! |---|---|---|
//! | `GET /`, `GET /index.html` | The app's page, or a placeholder when there is no build | never |
//! | `GET /assets/{*path}` | Its scripts, styles and fonts, named by their content's hash | a year |
//! | `GET /favicon.svg`, `GET /third-party-licences.txt` | Its icon, and its npm packages' licences | revalidated |
//!
//! The files come from `web/dist`: embedded in release builds, read from disk in debug builds,
//! or read from the folder `--webapp-dir` gives. Build the web app before a release server
//! (`npm run build` in `web/`).

use std::borrow::Cow;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use rust_embed::RustEmbed;
use serde_json::json;
use teta_wot::http::axum::extract::{Path as UrlPath, State};
use teta_wot::http::axum::http::{HeaderValue, StatusCode, header};
use teta_wot::http::axum::response::{Html, IntoResponse, Response};
use teta_wot::http::axum::routing::get;
use teta_wot::http::axum::{Json, Router};

/// `web/dist`, embedded in release builds and read from disk in debug ones. It may be missing.
#[derive(RustEmbed)]
#[folder = "../../web/dist"]
#[allow_missing = true]
struct Built;

/// Where the web app's files come from.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum WebApp {
    /// The build compiled into the server: `web/dist`, when it was compiled.
    #[default]
    Embedded,
    /// A build in a folder, such as `web/dist`.
    Dir(PathBuf),
}

impl WebApp {
    /// The file at `path` in the build. A path that could leave the build's folder is refused.
    async fn file(&self, path: &str) -> Option<Cow<'static, [u8]>> {
        let plain = Path::new(path).components();
        if !plain
            .into_iter()
            .all(|part| matches!(part, Component::Normal(_)))
        {
            return None;
        }
        match self {
            Self::Embedded => Built::get(path).map(|file| file.data),
            Self::Dir(dir) => tokio::fs::read(dir.join(path)).await.ok().map(Cow::Owned),
        }
    }
}

const NEVER: &str = "no-store";
const REVALIDATE: &str = "no-cache";
const A_YEAR: &str = "public, max-age=31536000, immutable";

/// The web app's routes, from `source`.
pub fn routes(source: WebApp) -> Router {
    let source = Arc::new(source);
    let file = |name: &'static str, kind: &'static str| {
        get(move |State(source): State<Arc<WebApp>>| async move {
            match source.file(name).await {
                Some(data) => respond(data, kind, REVALIDATE),
                None => not_found(),
            }
        })
    };
    Router::new()
        .route("/", get(page))
        .route("/index.html", get(page))
        .route("/assets/{*path}", get(asset))
        .route("/favicon.svg", file("favicon.svg", "image/svg+xml"))
        .route(
            "/third-party-licences.txt",
            file("third-party-licences.txt", "text/plain; charset=utf-8"),
        )
        .with_state(source)
}

async fn page(State(source): State<Arc<WebApp>>) -> Response {
    match source.file("index.html").await {
        Some(page) => respond(page, "text/html; charset=utf-8", NEVER),
        None => cached(Html(PLACEHOLDER).into_response(), NEVER),
    }
}

async fn asset(State(source): State<Arc<WebApp>>, UrlPath(path): UrlPath<String>) -> Response {
    match source.file(&format!("assets/{path}")).await {
        Some(data) => respond(data, media_type(&path), A_YEAR),
        None => not_found(),
    }
}

fn respond(data: Cow<'static, [u8]>, kind: &'static str, cache: &'static str) -> Response {
    cached(
        ([(header::CONTENT_TYPE, kind)], data.into_owned()).into_response(),
        cache,
    )
}

fn cached(mut response: Response, cache: &'static str) -> Response {
    let value = HeaderValue::from_static(cache);
    response.headers_mut().insert(header::CACHE_CONTROL, value);
    response
}

/// As teta-wot answers a path it doesn't know.
fn not_found() -> Response {
    (StatusCode::NOT_FOUND, Json(json!({"detail": "Not Found"}))).into_response()
}

/// A built file's media type, by its extension.
fn media_type(path: &str) -> &'static str {
    match Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
    {
        Some("js" | "mjs") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("woff2") => "font/woff2",
        Some("json" | "map") => "application/json",
        _ => "application/octet-stream",
    }
}

/// The page at `/` when the server has no build of the web app.
const PLACEHOLDER: &str = r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>SpectrumIQ</title>
<style>
  body { margin: 0; font-family: 'Segoe UI', system-ui, sans-serif; line-height: 1.5; color: #1a1a1a; }
  main { max-width: 40rem; margin: 4rem auto; padding: 0 1.5rem; }
  h1 { font-weight: 300; color: #b41d73; }
  code { font-family: 'Cascadia Mono', Consolas, monospace; }
</style>
</head>
<body>
<main>
<h1>The SpectrumIQ service is running</h1>
<p>It has no web app to show: the web app wasn't built when this server was.</p>
<ul>
<li>Build the web app (<code>npm run build</code> in <code>web/</code>), then the server.</li>
<li>Or start the server with <code>--webapp-dir web/dist</code>.</li>
</ul>
<p>The API is at <a href="/api/v1/things/">/api/v1/</a>, and its documentation at <a href="/docs">/docs</a>.</p>
</main>
</body>
</html>
"#;
