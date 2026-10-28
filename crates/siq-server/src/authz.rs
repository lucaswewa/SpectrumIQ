//! Who may do what (ADR-0066, architecture §10): every route's class, by its method and path, and
//! whether a caller of a role, holding the control token or not, may make it. teta-wot's
//! WebSocket is observe-only, so its upgrade, a `GET`, is a read.

use siq_auth::{Role, Session};
use teta_wot::http::axum::http::{Method, StatusCode};

/// What a request does, for who may make it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Class {
    /// The web app, the login, the health: anyone.
    Public,
    /// Reading: properties, events, streams, tiles, descriptions. Any user.
    Read,
    /// Stopping: a `halt`, or cancelling an invocation, a run's included. Operators and admins,
    /// without the token.
    Stop,
    /// Processing: jobs, exports, the shading's estimate. Operators and admins.
    Processing,
    /// The token itself: requesting, releasing, taking over. Operators and admins; taking over,
    /// admins and the local console.
    Control,
    /// Changing the instrument: its devices, the display, the live mosaic, the workflows and
    /// the calibrations. Operators and admins holding the token.
    Hardware,
    /// Deleting data, managing users. Admins.
    Admin,
}

/// The Things that are the instrument: changing them takes the token.
const HARDWARE: [&str; 12] = [
    "camera",
    "display",
    "stage",
    "focus",
    "illumination",
    "filter_wheel",
    "objective",
    "modality",
    "imaging",
    "workflow",
    "calibration",
    "mosaic_live",
];

/// The class of a request of `method` on `path`; `None` for a change to a route not classified,
/// which is refused.
pub fn classify(method: &Method, path: &str) -> Option<Class> {
    let path = path.trim_end_matches('/');
    if matches!(
        path,
        "" | "/index.html"
            | "/favicon.svg"
            | "/third-party-licences.txt"
            | "/api/siq/system/health"
    ) || path.starts_with("/assets/")
        || path.starts_with("/api/siq/auth/")
        || *method == Method::OPTIONS
    {
        return Some(Class::Public);
    }
    if matches!(*method, Method::GET | Method::HEAD) {
        return Some(Class::Read);
    }
    if let Some(action) = path.strip_prefix("/api/siq/control/") {
        return matches!(action, "request" | "release" | "heartbeat" | "take_over")
            .then_some(Class::Control);
    }
    if path.starts_with("/api/v1/action_invocations/") && *method == Method::DELETE {
        return Some(Class::Stop);
    }
    let mut parts = path.strip_prefix("/api/v1/")?.split('/');
    let (thing, affordance) = (parts.next()?, parts.next()?);
    match (thing, affordance) {
        ("stage" | "focus", "halt") => Some(Class::Stop),
        ("workflow", "estimate_tile_scan") | ("datasets", "summary") => Some(Class::Read),
        ("calibration", "estimate_shading")
        | ("mosaic_live", "save_snapshot")
        | ("datasets", "export_dzi" | "export_ome_tiff")
        | ("jobs", _) => Some(Class::Processing),
        ("datasets", "delete") => Some(Class::Admin),
        ("datasets", "snap_to_dataset") => Some(Class::Hardware),
        (thing, _) if HARDWARE.contains(&thing) => Some(Class::Hardware),
        _ => None,
    }
}

/// Whether `caller`, holding the token if `holds`, may make a request of `class`: if not, the
/// status and the reason — 401 without a session, 403 otherwise.
pub fn decide(
    class: Option<Class>,
    caller: Option<&Session>,
    holds: bool,
) -> Result<(), (StatusCode, String)> {
    let Some(class) = class else {
        return Err((
            StatusCode::FORBIDDEN,
            "this route isn't classified, so no one may change it".to_owned(),
        ));
    };
    if class == Class::Public {
        return Ok(());
    }
    let Some(caller) = caller else {
        return Err((StatusCode::UNAUTHORIZED, "log in first".to_owned()));
    };
    let refused = |why: String| Err((StatusCode::FORBIDDEN, why));
    let role = caller.role;
    match class {
        Class::Public | Class::Read => Ok(()),
        Class::Stop | Class::Processing | Class::Control if role >= Role::Operator => Ok(()),
        Class::Hardware if role >= Role::Operator && holds => Ok(()),
        Class::Hardware if role >= Role::Operator => {
            refused("changing the instrument needs the control token: request it first".into())
        }
        Class::Admin if role == Role::Admin => Ok(()),
        Class::Admin => refused(format!("an admin's to do, and {} is a {role}", caller.user)),
        _ => refused(format!(
            "an operator's to do, and {} is a {role}",
            caller.user
        )),
    }
}
