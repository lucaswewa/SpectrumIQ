//! Who is calling (ADR-0065), and who operates (ADR-0066): local accounts with argon2id hashes and
//! a role each, browser sessions that end when idle or old, a limit on failed logins, the
//! same-origin check that mutating requests pass, the Hardware Control Token, and the local
//! console's login. Which role may do what, route by route, is `siq-server`'s.
//!
//! - **Accounts** ([`UserStore`]): a JSON file of names, roles and PHC strings, by default
//!   `users.json` in the service's settings. It is written whole and replaced, and read again
//!   when another process, such as the CLI, has changed it. The last admin can't be removed.
//! - **Sessions** ([`SessionStore`]): in memory, each known by 256 random bits carried in an
//!   `HttpOnly`, `SameSite=Strict` cookie ([`set_cookie`]), which `<img>`, `fetch` streams and
//!   WebSockets send without a script's help.
//! - **Logins** ([`LoginLimiter`]): failures counted per name and per address.
//! - **Origins** ([`same_origin`]): a mutating request from a browser comes from the service's
//!   own pages.
//! - **The control token** ([`ControlManager`]): at most one session holds it; it is requested,
//!   released or taken over, and goes by itself when its holder is silent for the timeout.
//! - **The local console** ([`LocalLogin`]): a one-time secret in a file only the service's
//!   account can open, exchanged for a session that may take the token over.
//! - **The audit trail** ([`AuditLog`], [`audit::verify`]): hash-chained JSON Lines, a file a
//!   month, of logins, token changes, changing requests, and job and workflow lifecycles
//!   (ADR-0067).
//! - **The CLI** ([`cli`]): `spectrumiq-server user add|passwd|role|list|remove`, and
//!   [`read_password`], a password typed without being shown.

pub mod audit;
pub mod cli;
mod console;
mod control;
mod limit;
mod local;
mod origin;
mod sessions;
mod users;

pub use audit::{AuditLog, Event, Kind};
pub use console::read_password;
pub use control::{ControlError, ControlManager, ControlState, Holder, Reason};
pub use limit::LoginLimiter;
pub use local::LocalLogin;
pub use origin::{COOKIE, clear_cookie, same_origin, session_cookie, set_cookie};
pub use sessions::{Session, SessionId, SessionPolicy, SessionStore};
pub use users::{MIN_PASSWORD, Role, User, UserError, UserStore};

/// The users' file, in the service's `[server] settings`.
pub const USERS_FILE: &str = "users.json";
