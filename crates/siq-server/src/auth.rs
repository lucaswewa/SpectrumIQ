//! Sessions and who may do what over HTTP (ADR-0065, ADR-0066):
//!
//! - `POST /api/siq/auth/login`, `logout` and `local` (the local console's secret), and
//!   `GET /api/siq/auth/me`;
//! - `GET /api/siq/control`, and `POST /api/siq/control/request`, `release`, `heartbeat` and
//!   `take_over`: the Hardware Control Token;
//! - the layer every request passes: it refuses a mutating request from another site's page,
//!   gives a request with a live session's cookie its [`Session`], counts a request of the
//!   token's holder as its heartbeat, and, where `[auth] enforce`, refuses what the caller's role
//!   and token don't allow (`authz`); every change and every refusal goes into the audit trail
//!   (ADR-0067), as do the logins.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use serde::Deserialize;
use serde_json::{Value, json};
use siq_auth::{
    AuditLog, ControlError, ControlManager, Kind, LocalLogin, LoginLimiter, Role, Session,
    SessionPolicy, SessionStore, USERS_FILE, UserStore, clear_cookie, same_origin, session_cookie,
    set_cookie,
};
use siq_config::model::InstrumentConfig;
use siq_types::Clock;
use teta_wot::http::axum::body::{Body, to_bytes};
use teta_wot::http::axum::extract::{ConnectInfo, Request, State};
use teta_wot::http::axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use teta_wot::http::axum::middleware::Next;
use teta_wot::http::axum::response::{IntoResponse, Response};
use teta_wot::http::axum::routing::{get, post};
use teta_wot::http::axum::{Json, Router};
use tracing::{info, warn};

use crate::audit::{audited, event, summary};
use crate::authz;

/// The local console's secret, in `[server] settings`.
pub const LOCAL_LOGIN_FILE: &str = "local-login";

/// The accounts, the live sessions, the control token, and the guards of the login.
pub struct Auth {
    /// The accounts: `users.json` in `[server] settings`.
    pub users: Arc<UserStore>,
    /// The live sessions.
    pub sessions: Arc<SessionStore>,
    /// The Hardware Control Token (ADR-0066).
    pub control: Arc<ControlManager>,
    /// The audit trail, in `[server] audit` (ADR-0067).
    pub audit: Arc<AuditLog>,
    /// The local console's secret, `local-login` in `[server] settings`, if `[auth] local_user`
    /// names its user.
    pub local: Option<LocalLogin>,
    local_user: Option<String>,
    enforce: bool,
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
        if !auth.enforce {
            warn!("[auth] enforce is off: every request is allowed");
        }
        let local = match &auth.local_user {
            Some(_) => Some(LocalLogin::create(
                config.server.settings.join(LOCAL_LOGIN_FILE),
            )?),
            None => None,
        };
        let minutes = |m: u32| Duration::from_secs(60 * u64::from(m));
        let timeout = Duration::from_secs(u64::from(auth.control_timeout_s));
        let audit = Arc::new(AuditLog::open(&config.server.audit, clock.clone())?);
        let policy = SessionPolicy {
            idle: minutes(auth.idle_minutes),
            absolute: minutes(60 * auth.session_hours),
        };
        Ok(Self {
            users: Arc::new(users),
            sessions: Arc::new(SessionStore::new(clock.clone(), policy)),
            control: Arc::new(
                ControlManager::new(clock.clone(), timeout).with_audit(Arc::clone(&audit)),
            ),
            audit,
            local,
            local_user: auth.local_user.clone(),
            enforce: auth.enforce,
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
        let holder = self.control.holder();
        json!({
            "name": session.user,
            "role": session.role,
            "local": session.local,
            "since": session.created_utc,
            "expires": self.sessions.expires(session),
            "holds_token": holder.is_some_and(|h| h.session == session.id),
        })
    }

    /// The answer to a login: who the session is, and its cookie.
    fn opened(&self, session: &Session) -> Response {
        let cookie = set_cookie(&session.id, self.sessions.policy().absolute);
        let mut response = Json(self.describe(session)).into_response();
        if let Ok(cookie) = HeaderValue::from_str(&cookie) {
            response.headers_mut().insert(header::SET_COOKIE, cookie);
        }
        response
    }
}

/// A refusal: `status`, with its reason.
fn refuse(status: StatusCode, why: impl Into<String>) -> Response {
    (status, Json(json!({ "detail": why.into() }))).into_response()
}

/// The layer around every route:
///
/// - a request that would change something, from a browser on another site's page, is refused
///   with 403;
/// - a request with a live session's cookie carries its [`Session`] in its extensions, and is
///   its holder's heartbeat if the session holds the token;
/// - where `[auth] enforce`, a request its caller may not make is refused: 401 without a
///   session, 403 for a role or a missing token (ADR-0066);
/// - a change, its parameters and its answer, and a refusal, go into the audit trail
///   (ADR-0067).
pub(crate) async fn layer(State(auth): State<Arc<Auth>>, request: Request, next: Next) -> Response {
    let reading = matches!(
        *request.method(),
        Method::GET | Method::HEAD | Method::OPTIONS
    );
    let address = (request.extensions().get::<ConnectInfo<SocketAddr>>()).map(|c| c.0.ip());
    let (method, path) = (request.method().clone(), request.uri().path().to_owned());
    let audit = !reading && audited(&path);
    let refused = |session: Option<&Session>, status: StatusCode, why: String| {
        if audit {
            let detail = json!({ "method": method.as_str(), "path": path, "status": status.as_u16(), "why": why });
            auth.audit
                .record(event(Kind::Refused, session, address, detail));
        }
        refuse(status, why)
    };
    if !reading {
        let headers = request.headers();
        let text = |name| {
            headers
                .get(name)
                .and_then(|value: &HeaderValue| value.to_str().ok())
        };
        let host = text(header::HOST).or_else(|| request.uri().authority().map(|a| a.as_str()));
        if !same_origin(text(header::ORIGIN), host, &auth.allowed_origins) {
            let why = "a change must come from the service's own pages";
            return refused(None, StatusCode::FORBIDDEN, why.to_owned());
        }
    }
    let session = auth.session(request.headers());
    let holds = (session.as_ref()).is_some_and(|session| auth.control.heartbeat(&session.id));
    let class = authz::classify(&method, &path);
    if auth.enforce
        && let Err((status, why)) = authz::decide(class, session.as_ref(), holds)
    {
        return refused(session.as_ref(), status, why);
    }
    // A change's body, read for its summary, and given back.
    let (mut parts, body) = request.into_parts();
    let (body, params) = if audit {
        let Ok(bytes) = to_bytes(body, 2 << 20).await else {
            let why = "a request's body is 2 MiB at most".to_owned();
            return refused(session.as_ref(), StatusCode::PAYLOAD_TOO_LARGE, why);
        };
        let params = summary(&bytes);
        (Body::from(bytes), params)
    } else {
        (body, Value::Null)
    };
    if let Some(session) = &session {
        parts.extensions.insert(session.clone());
    }
    let query = parts.uri.query().map(str::to_owned);
    let response = next.run(Request::from_parts(parts, body)).await;
    if audit {
        let detail = json!({
            "method": method.as_str(),
            "path": path,
            "query": query,
            "class": class.map(|c| format!("{c:?}").to_lowercase()),
            "params": params,
            "status": response.status().as_u16(),
        });
        auth.audit
            .record(event(Kind::Request, session.as_ref(), address, detail));
    }
    response
}

/// The session routes.
pub(crate) fn routes(auth: &Arc<Auth>) -> Router {
    Router::new()
        .route("/api/siq/auth/login", post(login))
        .route("/api/siq/auth/logout", post(logout))
        .route("/api/siq/auth/me", get(me))
        .route("/api/siq/auth/local", post(local))
        .route("/api/siq/control", get(control))
        .route("/api/siq/control/request", post(request_token))
        .route("/api/siq/control/release", post(release))
        .route("/api/siq/control/heartbeat", post(heartbeat))
        .route("/api/siq/control/take_over", post(take_over))
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
        let detail = json!({ "name": name, "wait_s": seconds });
        auth.audit
            .record(event(Kind::LoginRefused, None, address, detail));
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
            let detail = json!({ "name": name });
            auth.audit
                .record(event(Kind::LoginFailed, None, address, detail));
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
    let detail = json!({ "role": user.role, "local": false });
    auth.audit
        .record(event(Kind::Login, Some(&session), address, detail));
    auth.opened(&session)
}

/// A local login's body.
#[derive(Deserialize)]
struct Local {
    secret: String,
}

/// `POST /api/siq/auth/local` with `{ "secret" }`, from this computer: a session of the local
/// console, as `[auth] local_user`, which may take the token over. The secret is the one in
/// `local-login` in `[server] settings`, which the next replaces. 403 from another computer, 404
/// without a local user, 401 for another secret.
async fn local(State(auth): State<Arc<Auth>>, request: Request) -> Response {
    let address = (request.extensions().get::<ConnectInfo<SocketAddr>>()).map(|c| c.0.ip());
    if !address.is_some_and(|address| address.is_loopback()) {
        return refuse(
            StatusCode::FORBIDDEN,
            "the local console logs in from this computer only",
        );
    }
    let (Some(login), Some(name)) = (&auth.local, &auth.local_user) else {
        return refuse(
            StatusCode::NOT_FOUND,
            "the local console has no user: set [auth] local_user",
        );
    };
    let Ok(body) = to_bytes(request.into_body(), 4 << 10).await else {
        return refuse(StatusCode::BAD_REQUEST, "the body couldn't be read");
    };
    let Ok(Local { secret }) = serde_json::from_slice(&body) else {
        return refuse(StatusCode::BAD_REQUEST, "expected { \"secret\" }");
    };
    match login.exchange(&secret) {
        Ok(true) => {}
        Ok(false) => {
            warn!(?address, "a local login with another secret");
            let detail = json!({ "local": true });
            auth.audit
                .record(event(Kind::LoginFailed, None, address, detail));
            return refuse(StatusCode::UNAUTHORIZED, "that isn't the local secret");
        }
        Err(error) => return refuse(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
    }
    let user = match auth.users.get(name) {
        Ok(Some(user)) => user,
        Ok(None) => {
            return refuse(
                StatusCode::SERVICE_UNAVAILABLE,
                format!("the local user {name} doesn't exist: add it"),
            );
        }
        Err(error) => return refuse(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
    };
    match auth.sessions.create_local(&user, address) {
        Ok(session) => {
            info!(user = %user.name, "the local console logged in");
            let detail = json!({ "role": user.role, "local": true });
            auth.audit
                .record(event(Kind::Login, Some(&session), address, detail));
            auth.opened(&session)
        }
        Err(error) => refuse(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
    }
}

/// `POST /api/siq/auth/logout`: the session ended, and its cookie cleared.
async fn logout(State(auth): State<Arc<Auth>>, request: Request) -> Response {
    if let Some(session) = request.extensions().get::<Session>() {
        auth.sessions.remove(&session.id);
        auth.control.session_ended(&session.id);
        info!(user = %session.user, "logged out");
        let detail = json!({});
        auth.audit
            .record(event(Kind::Logout, Some(session), None, detail));
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

/// The token's holder, as `{ "user", "since", "local" }`, or `null`.
fn holding(auth: &Auth) -> Value {
    json!({ "holder": auth.control.holder() })
}

/// `GET /api/siq/control`: who holds the token.
async fn control(State(auth): State<Arc<Auth>>) -> Response {
    Json(holding(&auth)).into_response()
}

/// The request's session.
fn caller(request: &Request) -> Option<Session> {
    request.extensions().get::<Session>().cloned()
}

/// 401: the request has no session.
fn unauthorized() -> Response {
    refuse(StatusCode::UNAUTHORIZED, "log in first")
}

/// `POST /api/siq/control/request`: the token for the session if it is free; 409, saying who
/// holds it, if not.
async fn request_token(State(auth): State<Arc<Auth>>, request: Request) -> Response {
    let Some(session) = caller(&request) else {
        return unauthorized();
    };
    match auth.control.request(&session) {
        Ok(holder) => {
            info!(user = %holder.user, "the control token requested");
            Json(holding(&auth)).into_response()
        }
        Err(error) => refuse(StatusCode::CONFLICT, error.to_string()),
    }
}

/// `POST /api/siq/control/release`: the token given back; 409 if the session doesn't hold it.
async fn release(State(auth): State<Arc<Auth>>, request: Request) -> Response {
    let Some(session) = caller(&request) else {
        return unauthorized();
    };
    match auth.control.release(&session.id) {
        Ok(()) => {
            info!(user = %session.user, "the control token released");
            StatusCode::NO_CONTENT.into_response()
        }
        Err(error) => refuse(StatusCode::CONFLICT, error.to_string()),
    }
}

/// `POST /api/siq/control/heartbeat`: the holder's sign of life, though any of its requests is
/// one; 409 if the session doesn't hold the token.
async fn heartbeat(State(auth): State<Arc<Auth>>, request: Request) -> Response {
    let Some(session) = caller(&request) else {
        return unauthorized();
    };
    match auth.control.heartbeat(&session.id) {
        true => StatusCode::NO_CONTENT.into_response(),
        false => refuse(StatusCode::CONFLICT, ControlError::NotHolder.to_string()),
    }
}

/// `POST /api/siq/control/take_over`: the token, from whoever holds it, for an admin or the local
/// console; 403 for others.
async fn take_over(State(auth): State<Arc<Auth>>, request: Request) -> Response {
    let Some(session) = caller(&request) else {
        return unauthorized();
    };
    if session.role != Role::Admin && !session.local {
        return refuse(
            StatusCode::FORBIDDEN,
            "only an admin or the local console takes the control token over",
        );
    }
    let from = auth.control.holder().map(|h| h.user);
    let holder = auth.control.take_over(&session);
    warn!(user = %holder.user, from = ?from, "the control token taken over");
    Json(holding(&auth)).into_response()
}
