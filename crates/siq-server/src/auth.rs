//! Sessions over HTTP (ADR-0065): `POST /api/siq/auth/login` and `logout`, and
//! `GET /api/siq/auth/me`; and the layer every request passes, which refuses a mutating request
//! from another site's page, and gives a request with a live session's cookie its [`Session`].
//! Which role may do what, route by route, is P9.2's.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use serde::Deserialize;
use serde_json::{Value, json};
use siq_auth::{
    LoginLimiter, Session, SessionPolicy, SessionStore, USERS_FILE, UserStore, clear_cookie,
    same_origin, session_cookie, set_cookie,
};
use siq_config::model::InstrumentConfig;
use siq_types::Clock;
use teta_wot::http::axum::body::to_bytes;
use teta_wot::http::axum::extract::{ConnectInfo, Request, State};
use teta_wot::http::axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use teta_wot::http::axum::middleware::Next;
use teta_wot::http::axum::response::{IntoResponse, Response};
use teta_wot::http::axum::routing::{get, post};
use teta_wot::http::axum::{Json, Router};
use tracing::{info, warn};

/// The accounts, the live sessions, and the guards of the login.
pub struct Auth {
    /// The accounts: `users.json` in `[server] settings`.
    pub users: Arc<UserStore>,
    /// The live sessions.
    pub sessions: Arc<SessionStore>,
    limiter: LoginLimiter,
    allowed_origins: Vec<String>,
}

impl Auth {
    /// The accounts of `config`'s settings, and no sessions yet, timed by `clock`.
    pub fn open(config: &InstrumentConfig, clock: Arc<dyn Clock>) -> anyhow::Result<Self> {
        let auth = &config.auth;
        let users = UserStore::open(config.server.settings.join(USERS_FILE))?;
        if users.is_empty()? {
            warn!(
                file = %users.path().display(),
                "no users yet: add the first admin with `spectrumiq-server user add <name> --role admin -c <config>`"
            );
        }
        let minutes = |m: u32| Duration::from_secs(60 * u64::from(m));
        let policy = SessionPolicy {
            idle: minutes(auth.idle_minutes),
            absolute: minutes(60 * auth.session_hours),
        };
        Ok(Self {
            users: Arc::new(users),
            sessions: Arc::new(SessionStore::new(clock.clone(), policy)),
            limiter: LoginLimiter::new(
                clock,
                auth.max_failures as usize,
                minutes(auth.failure_minutes),
            ),
            allowed_origins: auth.allowed_origins.clone(),
        })
    }

    /// The live session of `headers`' cookie, its idle time begun again and its role the
    /// store's now: `None` if there is none, or its user has been removed.
    pub fn session(&self, headers: &HeaderMap) -> Option<Session> {
        let id = (headers.get_all(header::COOKIE).iter())
            .filter_map(|value| value.to_str().ok())
            .find_map(session_cookie)?;
        let session = self.sessions.touch(&id)?;
        match self.users.get(&session.user) {
            Ok(Some(user)) if user.role != session.role => {
                self.sessions.set_role(&user.name, user.role);
                Some(Session {
                    role: user.role,
                    ..session
                })
            }
            Ok(Some(_)) => Some(session),
            Ok(None) => {
                self.sessions.end_user(&session.user);
                None
            }
            Err(error) => {
                warn!(%error, "reading the users");
                Some(session)
            }
        }
    }

    /// What `/me` says of `session`.
    fn describe(&self, session: &Session) -> Value {
        json!({
            "name": session.user,
            "role": session.role,
            "since": session.created_utc,
            "expires": self.sessions.expires(session),
        })
    }
}

/// A refusal: `status`, with its reason.
fn refuse(status: StatusCode, why: impl Into<String>) -> Response {
    (status, Json(json!({ "detail": why.into() }))).into_response()
}

/// The layer around every route: a request that would change something, from a browser on another
/// site's page, is refused with 403; a request with a live session's cookie carries its
/// [`Session`] in its extensions.
pub(crate) async fn layer(
    State(auth): State<Arc<Auth>>,
    mut request: Request,
    next: Next,
) -> Response {
    let reading = matches!(
        *request.method(),
        Method::GET | Method::HEAD | Method::OPTIONS
    );
    if !reading {
        let headers = request.headers();
        let text = |name| {
            headers
                .get(name)
                .and_then(|value: &HeaderValue| value.to_str().ok())
        };
        let host = text(header::HOST).or_else(|| request.uri().authority().map(|a| a.as_str()));
        if !same_origin(text(header::ORIGIN), host, &auth.allowed_origins) {
            return refuse(
                StatusCode::FORBIDDEN,
                "a change must come from the service's own pages",
            );
        }
    }
    if let Some(session) = auth.session(request.headers()) {
        request.extensions_mut().insert(session);
    }
    next.run(request).await
}

/// The session routes.
pub(crate) fn routes(auth: &Arc<Auth>) -> Router {
    Router::new()
        .route("/api/siq/auth/login", post(login))
        .route("/api/siq/auth/logout", post(logout))
        .route("/api/siq/auth/me", get(me))
        .with_state(Arc::clone(auth))
}

/// A login's body.
#[derive(Deserialize)]
struct Login {
    name: String,
    password: String,
}

/// `POST /api/siq/auth/login` with `{ "name", "password" }`: a session, in a cookie, and who it
/// is; 401 if the name or the password is wrong; 429, with `Retry-After`, after too many failures.
async fn login(State(auth): State<Arc<Auth>>, request: Request) -> Response {
    let address = (request.extensions().get::<ConnectInfo<SocketAddr>>()).map(|c| c.0.ip());
    let Ok(body) = to_bytes(request.into_body(), 64 << 10).await else {
        return refuse(StatusCode::BAD_REQUEST, "the body couldn't be read");
    };
    let Ok(Login { name, password }) = serde_json::from_slice(&body) else {
        return refuse(
            StatusCode::BAD_REQUEST,
            "expected { \"name\", \"password\" }",
        );
    };
    if let Some(wait) = auth.limiter.wait(&name, address) {
        let seconds = wait.as_secs_f64().ceil() as u64;
        warn!(name, ?address, "a login refused: too many failures");
        let mut refused = refuse(
            StatusCode::TOO_MANY_REQUESTS,
            format!("too many failed logins: try again in {seconds} s"),
        );
        refused
            .headers_mut()
            .insert(header::RETRY_AFTER, HeaderValue::from(seconds));
        return refused;
    }
    // Hashing takes tens of milliseconds of a processor: not on the runtime's threads.
    let users = Arc::clone(&auth.users);
    let asked = name.clone();
    let checked = tokio::task::spawn_blocking(move || users.verify(&asked, &password)).await;
    let user = match checked {
        Ok(Ok(Some(user))) => user,
        Ok(Ok(None)) => {
            auth.limiter.failed(&name, address);
            info!(name, ?address, "a failed login");
            return refuse(
                StatusCode::UNAUTHORIZED,
                "the name or the password is wrong",
            );
        }
        Ok(Err(error)) => return refuse(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
        Err(error) => return refuse(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
    };
    auth.limiter.succeeded(&name);
    let session = match auth.sessions.create(&user, address) {
        Ok(session) => session,
        Err(error) => return refuse(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
    };
    info!(user = %user.name, role = %user.role, ?address, "logged in");
    let cookie = set_cookie(&session.id, auth.sessions.policy().absolute);
    let mut response = Json(auth.describe(&session)).into_response();
    if let Ok(cookie) = HeaderValue::from_str(&cookie) {
        response.headers_mut().insert(header::SET_COOKIE, cookie);
    }
    response
}

/// `POST /api/siq/auth/logout`: the session ended, and its cookie cleared.
async fn logout(State(auth): State<Arc<Auth>>, request: Request) -> Response {
    if let Some(session) = request.extensions().get::<Session>() {
        auth.sessions.remove(&session.id);
        info!(user = %session.user, "logged out");
    }
    let mut response = StatusCode::NO_CONTENT.into_response();
    if let Ok(cookie) = HeaderValue::from_str(&clear_cookie()) {
        response.headers_mut().insert(header::SET_COOKIE, cookie);
    }
    response
}

/// `GET /api/siq/auth/me`: who the session is, and when it ends; 401 without one.
async fn me(State(auth): State<Arc<Auth>>, request: Request) -> Response {
    match request.extensions().get::<Session>() {
        Some(session) => Json(auth.describe(session)).into_response(),
        None => refuse(StatusCode::UNAUTHORIZED, "not logged in"),
    }
}
