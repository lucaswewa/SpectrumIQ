//! The sessions: in memory, each known by 256 random bits, ending after a while without a request
//! or after a fixed time however busy.

use std::collections::HashMap;
use std::fmt;
use std::net::IpAddr;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use chrono::{DateTime, Utc};
use siq_types::{Clock, Timestamp};

use crate::users::{Role, User};

/// A session's identifier: 32 random bytes from the operating system, as 64 hexadecimal digits.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct SessionId(String);

impl SessionId {
    /// A new identifier, unless the operating system has no randomness to give.
    pub fn random() -> Result<Self, getrandom::Error> {
        let mut bytes = [0u8; 32];
        getrandom::fill(&mut bytes)?;
        Ok(Self(bytes.iter().map(|b| format!("{b:02x}")).collect()))
    }

    /// `text` as an identifier, if it is 64 lowercase hexadecimal digits.
    pub fn parse(text: &str) -> Option<Self> {
        let digit = |c: char| c.is_ascii_digit() || ('a'..='f').contains(&c);
        (text.len() == 64 && text.chars().all(digit)).then(|| Self(text.to_owned()))
    }

    /// The digits, for the cookie.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SessionId {
    /// Its first digits only: logs mustn't hold a credential.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SessionId({}…)", &self.0[..8])
    }
}

/// How long sessions last.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionPolicy {
    /// A session ends after this long without a request.
    pub idle: Duration,
    /// And after this long however busy: the cookie's lifetime.
    pub absolute: Duration,
}

impl Default for SessionPolicy {
    /// An hour idle, and twelve hours, a long shift, at most.
    fn default() -> Self {
        Self {
            idle: Duration::from_secs(3600),
            absolute: Duration::from_secs(12 * 3600),
        }
    }
}

/// A session: whose, since when, and from where.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    /// Its identifier.
    pub id: SessionId,
    /// Its user's name.
    pub user: String,
    /// Its user's role, as the store last said.
    pub role: Role,
    /// The address it logged in from, if known.
    pub address: Option<IpAddr>,
    /// When it began, on the store's clock.
    pub created: Timestamp,
    /// When it began, in UTC.
    pub created_utc: DateTime<Utc>,
    /// Its last request, on the store's clock.
    pub seen: Timestamp,
}

/// The live sessions (ADR-0065).
#[derive(Debug)]
pub struct SessionStore {
    clock: Arc<dyn Clock>,
    policy: SessionPolicy,
    sessions: Mutex<HashMap<SessionId, Session>>,
}

impl SessionStore {
    /// No sessions, timed by `clock` with `policy`.
    pub fn new(clock: Arc<dyn Clock>, policy: SessionPolicy) -> Self {
        Self {
            clock,
            policy,
            sessions: Mutex::default(),
        }
    }

    /// How long its sessions last.
    pub fn policy(&self) -> SessionPolicy {
        self.policy
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<SessionId, Session>> {
        self.sessions.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Whether `session` has ended by `now`.
    fn ended(&self, session: &Session, now: Timestamp) -> bool {
        now.saturating_since(session.seen) >= self.policy.idle
            || now.saturating_since(session.created) >= self.policy.absolute
    }

    /// When `session` will end if nothing more is asked of it, in UTC.
    pub fn expires(&self, session: &Session) -> DateTime<Utc> {
        let end = (session.seen + self.policy.idle).min(session.created + self.policy.absolute);
        self.clock.utc(end)
    }

    /// A new session for `user`, from `address`.
    pub fn create(
        &self,
        user: &User,
        address: Option<IpAddr>,
    ) -> Result<Session, getrandom::Error> {
        let now = self.clock.now();
        let session = Session {
            id: SessionId::random()?,
            user: user.name.clone(),
            role: user.role,
            address,
            created: now,
            created_utc: self.clock.utc(now),
            seen: now,
        };
        let mut sessions = self.lock();
        sessions.retain(|_, s| !self.ended(s, now));
        sessions.insert(session.id.clone(), session.clone());
        Ok(session)
    }

    /// The live session `id`, its idle time begun again; one that has ended is forgotten.
    pub fn touch(&self, id: &SessionId) -> Option<Session> {
        let now = self.clock.now();
        let mut sessions = self.lock();
        let session = sessions.get_mut(id)?;
        if self.ended(session, now) {
            sessions.remove(id);
            return None;
        }
        session.seen = now;
        Some(session.clone())
    }

    /// Ends the session `id`: a logout.
    pub fn remove(&self, id: &SessionId) -> Option<Session> {
        self.lock().remove(id)
    }

    /// Ends every session of `user`, as when it is removed: how many there were.
    pub fn end_user(&self, user: &str) -> usize {
        let mut sessions = self.lock();
        let before = sessions.len();
        sessions.retain(|_, s| s.user != user);
        before - sessions.len()
    }

    /// Gives every session of `user` the role `role`.
    pub fn set_role(&self, user: &str, role: Role) {
        for session in self.lock().values_mut().filter(|s| s.user == user) {
            session.role = role;
        }
    }

    /// The live sessions, oldest first.
    pub fn list(&self) -> Vec<Session> {
        let now = self.clock.now();
        let mut sessions = self.lock();
        sessions.retain(|_, s| !self.ended(s, now));
        let mut live: Vec<Session> = sessions.values().cloned().collect();
        live.sort_by_key(|s| s.created);
        live
    }
}
