//! The users, over HTTP (ADR-0068): `GET`, `POST /api/siq/users`, and `PUT` and `DELETE
//! /api/siq/users/{name}`, an admin's, for the admin pages. A user removed, or given a new
//! password, has its sessions ended.

use std::sync::Arc;

use serde::Deserialize;
use serde_json::json;
use siq_auth::{Role, UserError};
use teta_wot::http::axum::extract::{Path, State};
use teta_wot::http::axum::http::StatusCode;
use teta_wot::http::axum::response::{IntoResponse, Response};
use teta_wot::http::axum::routing::{get, put};
use teta_wot::http::axum::{Json, Router};

use crate::auth::Auth;

/// The users routes.
pub(crate) fn routes(auth: &Arc<Auth>) -> Router {
    Router::new()
        .route("/api/siq/users", get(list).post(add))
        .route("/api/siq/users/{name}", put(change).delete(remove))
        .with_state(Arc::clone(auth))
}

/// A refusal of the store's, with its status.
fn refused(error: UserError) -> Response {
    let status = match error {
        UserError::Name(_) | UserError::Password => StatusCode::BAD_REQUEST,
        UserError::Exists(_) | UserError::LastAdmin => StatusCode::CONFLICT,
        UserError::Unknown(_) => StatusCode::NOT_FOUND,
        UserError::Io { .. } | UserError::Format { .. } | UserError::Hash(_) => {
            StatusCode::INTERNAL_SERVER_ERROR
        }
    };
    (status, Json(json!({ "detail": error.to_string() }))).into_response()
}

/// `GET /api/siq/users`: the users, by name, without their hashes.
async fn list(State(auth): State<Arc<Auth>>) -> Response {
    match auth.users.list() {
        Ok(users) => Json(users).into_response(),
        Err(error) => refused(error),
    }
}

/// A new user.
#[derive(Deserialize)]
struct New {
    name: String,
    password: String,
    role: Role,
}

/// `POST /api/siq/users` with `{ "name", "password", "role" }`: 201 and the user.
async fn add(State(auth): State<Arc<Auth>>, Json(new): Json<New>) -> Response {
    let users = Arc::clone(&auth.users);
    let added = tokio::task::spawn_blocking(move || users.add(&new.name, &new.password, new.role));
    match added.await {
        Ok(Ok(user)) => (StatusCode::CREATED, Json(user)).into_response(),
        Ok(Err(error)) => refused(error),
        Err(error) => refused(UserError::Hash(error.to_string())),
    }
}

/// A change to a user: its role, its password, or both.
#[derive(Deserialize)]
struct Change {
    role: Option<Role>,
    password: Option<String>,
}

/// `PUT /api/siq/users/{name}` with `{ "role"?, "password"? }`: the user. A new password ends its
/// sessions; the last admin keeps its role.
async fn change(
    State(auth): State<Arc<Auth>>,
    Path(name): Path<String>,
    Json(change): Json<Change>,
) -> Response {
    let users = Arc::clone(&auth.users);
    let asked = name.clone();
    let password = change.password.clone();
    let changed = tokio::task::spawn_blocking(move || {
        let mut user = match change.role {
            Some(role) => users.set_role(&asked, role)?,
            None => users
                .get(&asked)?
                .ok_or(UserError::Unknown(asked.clone()))?,
        };
        if let Some(password) = &password {
            user = users.set_password(&asked, password)?;
        }
        Ok::<_, UserError>(user)
    });
    match changed.await {
        Ok(Ok(user)) => {
            auth.sessions.set_role(&user.name, user.role);
            if change.password.is_some() {
                auth.sessions.end_user(&user.name);
            }
            Json(user).into_response()
        }
        Ok(Err(error)) => refused(error),
        Err(error) => refused(UserError::Hash(error.to_string())),
    }
}

/// `DELETE /api/siq/users/{name}`: 204, its sessions ended; the last admin stays.
async fn remove(State(auth): State<Arc<Auth>>, Path(name): Path<String>) -> Response {
    match auth.users.remove(&name) {
        Ok(user) => {
            auth.sessions.end_user(&user.name);
            StatusCode::NO_CONTENT.into_response()
        }
        Err(error) => refused(error),
    }
}
