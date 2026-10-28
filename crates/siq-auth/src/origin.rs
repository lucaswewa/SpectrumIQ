//! The same-origin check on mutating requests, and the session cookie.

use std::time::Duration;

use crate::sessions::SessionId;

/// The session cookie's name.
pub const COOKIE: &str = "siq_session";

/// Whether a request whose `Origin` and `Host` headers are these comes from the service's own
/// pages, so that another site's page can't make a logged-in browser change the instrument:
///
/// - no `Origin`: not a browser's cross-site request, but a script's or a tool's;
/// - an `Origin` whose host and port are the request's `Host`: the service's own pages, or a
///   development server's proxying to it;
/// - one of `allowed`, as `scheme://host:port`, whatever its case.
///
/// `null`, a sandboxed or opaque origin, is refused.
pub fn same_origin(origin: Option<&str>, host: Option<&str>, allowed: &[String]) -> bool {
    let Some(origin) = origin.map(str::trim) else {
        return true;
    };
    if allowed
        .iter()
        .any(|a| a.trim_end_matches('/').eq_ignore_ascii_case(origin))
    {
        return true;
    }
    let origin = origin.to_ascii_lowercase();
    let authority = (origin.strip_prefix("http://")).or_else(|| origin.strip_prefix("https://"));
    match (authority, host) {
        (Some(authority), Some(host)) => {
            !authority.is_empty()
                && !authority.contains('/')
                && authority.eq_ignore_ascii_case(host.trim())
        }
        _ => false,
    }
}

/// The session identifier in a `Cookie` header's value, if it has one.
pub fn session_cookie(header: &str) -> Option<SessionId> {
    header.split(';').find_map(|pair| {
        let (name, value) = pair.trim().split_once('=')?;
        (name == COOKIE).then(|| SessionId::parse(value.trim()))?
    })
}

/// The `Set-Cookie` value that gives a browser `id` for `max_age`: not for scripts, not sent with
/// other sites' requests, and with every request to the service, `<img>`, `fetch` streams and
/// WebSockets included.
pub fn set_cookie(id: &SessionId, max_age: Duration) -> String {
    format!(
        "{COOKIE}={}; HttpOnly; SameSite=Strict; Path=/; Max-Age={}",
        id.as_str(),
        max_age.as_secs()
    )
}

/// The `Set-Cookie` value that makes a browser forget the session.
pub fn clear_cookie() -> String {
    format!("{COOKIE}=; HttpOnly; SameSite=Strict; Path=/; Max-Age=0")
}
