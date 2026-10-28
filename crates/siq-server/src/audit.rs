//! The service's audit trail (ADR-0067): what the layer, the logins, the jobs, the workflows and
//! the datasets record (the control token records its own changes), and `GET /api/siq/audit`, an
//! admin's query of it.

use std::net::IpAddr;
use std::sync::Arc;

use serde_json::{Value, json};
use siq_auth::audit::Filter;
use siq_auth::{AuditLog, Event, Kind, Session};
use siq_jobs::Scheduler;
use teta_wot::http::axum::extract::{Query, State};
use teta_wot::http::axum::http::StatusCode;
use teta_wot::http::axum::response::{IntoResponse, Response};
use teta_wot::http::axum::routing::get;
use teta_wot::http::axum::{Json, Router};

/// The longest a parameter's string is kept, in characters.
const LONGEST: usize = 120;
/// Past this many bytes, a body is kept as its size and its keys.
const LARGEST: usize = 4096;

/// Whether a change to `path` is audited as a request: not the logins, which have their own
/// entries and carry passwords; not the token's heartbeat, nor the live mosaic's view, which
/// change by the second and change nothing kept.
pub(crate) fn audited(path: &str) -> bool {
    !path.starts_with("/api/siq/auth/")
        && path != "/api/siq/control/heartbeat"
        && path.trim_end_matches('/') != "/api/v1/mosaic_live/viewport"
}

/// An event of `kind` and `detail`, as `session`'s if there is one, from `address`.
pub(crate) fn event(
    kind: Kind,
    session: Option<&Session>,
    address: Option<IpAddr>,
    detail: Value,
) -> Event {
    let event = Event::new(kind, detail);
    match session {
        Some(session) => {
            let address = address.or(session.address);
            event.by(&session.user, Some(session.id.as_str()), address)
        }
        None => Event { address, ..event },
    }
}

/// A request's parameters, for the trail: its JSON body with each string cut to 120 characters;
/// past 4 KB, its size and its keys; not JSON, its size.
pub(crate) fn summary(body: &[u8]) -> Value {
    if body.is_empty() {
        return Value::Null;
    }
    let Ok(value) = serde_json::from_slice::<Value>(body) else {
        return json!({ "bytes": body.len() });
    };
    if body.len() > LARGEST {
        let keys: Vec<&String> = value
            .as_object()
            .map(|o| o.keys().collect())
            .unwrap_or_default();
        return json!({ "bytes": body.len(), "keys": keys });
    }
    cut(value)
}

fn cut(value: Value) -> Value {
    match value {
        Value::String(text) if text.chars().count() > LONGEST => {
            Value::String(text.chars().take(LONGEST).chain(['…']).collect())
        }
        Value::Array(items) => Value::Array(items.into_iter().map(cut).collect()),
        Value::Object(fields) => {
            Value::Object(fields.into_iter().map(|(k, v)| (k, cut(v))).collect())
        }
        other => other,
    }
}

/// Records each job's transitions, queued, started and ended, as the scheduler makes them.
pub(crate) fn record_jobs(jobs: &Scheduler, audit: Arc<AuditLog>) {
    jobs.on_transition(move |job| {
        let detail = json!({
            "id": job.id.0,
            "kind": job.kind,
            "state": job.state,
            "dataset": job.dataset,
            "error": job.error,
        });
        audit.record(Event::new(Kind::Job, detail));
    });
}

/// The query route.
pub(crate) fn routes(audit: &Arc<AuditLog>) -> Router {
    Router::new()
        .route("/api/siq/audit", get(query))
        .with_state(Arc::clone(audit))
}

/// `GET /api/siq/audit?from&to&user&kind&offset&limit`: the entries the filter takes, newest
/// first, and how many there are; an admin's.
async fn query(State(audit): State<Arc<AuditLog>>, Query(filter): Query<Filter>) -> Response {
    match tokio::task::spawn_blocking(move || audit.query(&filter)).await {
        Ok(Ok(page)) => Json(page).into_response(),
        Ok(Err(error)) => failed(error.to_string()),
        Err(error) => failed(error.to_string()),
    }
}

fn failed(why: String) -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({ "detail": why })),
    )
        .into_response()
}
