//! The Hardware Control Token (ADR-0066): at most one session holds it, and only its holder changes
//! the instrument. It is requested, released, or taken over; it goes by itself when its holder
//! has been silent for the timeout, without stopping what the holder started. Each change is
//! recorded in the audit trail as it happens, if there is one (ADR-0067): a watch keeps only the
//! latest.

use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::Serialize;
use serde_json::json;
use siq_types::{Clock, Timestamp};
use tokio::sync::watch;

use crate::audit::{AuditLog, Event, Kind};
use crate::sessions::{Session, SessionId};

/// Who holds the token.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct Holder {
    /// The user.
    pub user: String,
    /// Since when, in UTC.
    pub since: DateTime<Utc>,
    /// Whether it is the local console's session.
    pub local: bool,
    /// The session.
    #[serde(skip)]
    #[schemars(skip)]
    pub session: SessionId,
}

/// Why the token last changed hands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum Reason {
    /// Nobody has asked for it yet.
    Start,
    /// Its holder asked for it.
    Requested,
    /// Its holder gave it back.
    Released,
    /// An admin or the local console took it from `from`.
    TakenOver {
        /// The user it was taken from.
        from: String,
    },
    /// Its holder was silent for the timeout.
    TimedOut,
    /// Its holder's session ended: a logout.
    Ended,
}

/// The token's state: who holds it, and why it last changed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct ControlState {
    /// Its holder, if any.
    pub holder: Option<Holder>,
    /// Why it last changed.
    pub reason: Reason,
}

/// Why a request about the token was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ControlError {
    /// Another session holds it.
    #[error("the control token is held by {} since {}", .0.user, .0.since.format("%H:%M:%S UTC"))]
    Held(Holder),
    /// The session doesn't hold it.
    #[error("this session doesn't hold the control token")]
    NotHolder,
}

/// The token, and its holder's last sign of life.
#[derive(Debug)]
struct Held {
    holder: Holder,
    seen: Timestamp,
}

/// The Hardware Control Token's keeper.
#[derive(Debug)]
pub struct ControlManager {
    clock: Arc<dyn Clock>,
    timeout: Duration,
    held: Mutex<Option<Held>>,
    changes: watch::Sender<ControlState>,
    audit: Option<Arc<AuditLog>>,
}

impl ControlManager {
    /// The token, free, timed by `clock`: a holder silent for `timeout` loses it.
    pub fn new(clock: Arc<dyn Clock>, timeout: Duration) -> Self {
        let start = ControlState {
            holder: None,
            reason: Reason::Start,
        };
        Self {
            clock,
            timeout,
            held: Mutex::new(None),
            changes: watch::Sender::new(start),
            audit: None,
        }
    }

    /// The token, its every change recorded in `audit`.
    pub fn with_audit(mut self, audit: Arc<AuditLog>) -> Self {
        self.audit = Some(audit);
        self
    }

    /// Its state as it changes: for the `control` Thing.
    pub fn watch(&self) -> watch::Receiver<ControlState> {
        self.changes.subscribe()
    }

    /// The token, its holder gone if silent for the timeout.
    fn lock(&self) -> std::sync::MutexGuard<'_, Option<Held>> {
        let mut held = self.held.lock().unwrap_or_else(PoisonError::into_inner);
        let now = self.clock.now();
        if held
            .as_ref()
            .is_some_and(|h| now.saturating_since(h.seen) >= self.timeout)
            && let Some(gone) = held.take()
        {
            self.announce(Some(&gone.holder), None, Reason::TimedOut);
        }
        held
    }

    /// Tells the watchers and the audit trail that the token went from `previous` to `holder`.
    fn announce(&self, previous: Option<&Holder>, holder: Option<Holder>, reason: Reason) {
        if let Some(audit) = &self.audit {
            // A change is its new holder's, or, as it goes, its old one's.
            let whose = holder.as_ref().or(previous);
            let detail = json!({
                "holder": holder.as_ref().map(|h| &h.user),
                "from": previous.map(|h| &h.user),
                "local": whose.is_some_and(|h| h.local),
                "reason": reason,
            });
            let mut event = Event::new(Kind::Token, detail);
            if let Some(whose) = whose {
                event = event.by(&whose.user, Some(whose.session.as_str()), None);
            }
            audit.record(event);
        }
        self.changes.send_replace(ControlState { holder, reason });
    }

    fn give(&self, held: &mut Option<Held>, session: &Session, reason: Reason) -> Holder {
        let now = self.clock.now();
        let holder = Holder {
            user: session.user.clone(),
            since: self.clock.utc(now),
            local: session.local,
            session: session.id.clone(),
        };
        let previous = held.replace(Held {
            holder: holder.clone(),
            seen: now,
        });
        self.announce(
            previous.as_ref().map(|p| &p.holder),
            Some(holder.clone()),
            reason,
        );
        holder
    }

    /// Its holder, if any.
    pub fn holder(&self) -> Option<Holder> {
        self.lock().as_ref().map(|h| h.holder.clone())
    }

    /// Gives `session` the token if it is free; keeps it if `session` holds it already.
    pub fn request(&self, session: &Session) -> Result<Holder, ControlError> {
        let mut held = self.lock();
        if let Some(h) = held.as_mut() {
            if h.holder.session != session.id {
                return Err(ControlError::Held(h.holder.clone()));
            }
            h.seen = self.clock.now();
            return Ok(h.holder.clone());
        }
        Ok(self.give(&mut held, session, Reason::Requested))
    }

    /// Takes `session` the token from whoever holds it: the caller is an admin or the local
    /// console.
    pub fn take_over(&self, session: &Session) -> Holder {
        let mut held = self.lock();
        let from = held.as_ref().map(|h| h.holder.clone());
        match from {
            Some(from) if from.session == session.id => from,
            Some(from) => self.give(&mut held, session, Reason::TakenOver { from: from.user }),
            None => self.give(&mut held, session, Reason::Requested),
        }
    }

    /// Gives the token back, if `session` holds it.
    pub fn release(&self, session: &SessionId) -> Result<(), ControlError> {
        let mut held = self.lock();
        if !held.as_ref().is_some_and(|h| &h.holder.session == session) {
            return Err(ControlError::NotHolder);
        }
        if let Some(gone) = held.take() {
            self.announce(Some(&gone.holder), None, Reason::Released);
        }
        Ok(())
    }

    /// Whether `session` holds the token. If it does, this is a sign of life: the timeout begins
    /// again.
    pub fn heartbeat(&self, session: &SessionId) -> bool {
        let now = self.clock.now();
        let mut held = self.lock();
        match held.as_mut() {
            Some(h) if &h.holder.session == session => {
                h.seen = now;
                true
            }
            _ => false,
        }
    }

    /// Lets go of the token if `session`, which has ended, held it.
    pub fn session_ended(&self, session: &SessionId) {
        let mut held = self.lock();
        if held.as_ref().is_some_and(|h| &h.holder.session == session)
            && let Some(gone) = held.take()
        {
            self.announce(Some(&gone.holder), None, Reason::Ended);
        }
    }

    /// Lets the token go if its holder has been silent for the timeout: called now and then, so
    /// the token's watchers hear of it without anyone asking.
    pub fn expire(&self) {
        drop(self.lock());
    }
}
