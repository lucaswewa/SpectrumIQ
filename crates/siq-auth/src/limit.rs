//! Failed logins, counted per name and per address, so that passwords can't be guessed quickly.

use std::collections::{HashMap, VecDeque};
use std::net::IpAddr;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use siq_types::{Clock, Timestamp};

/// What failures are counted against.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Key {
    Name(String),
    Address(IpAddr),
}

/// The failed logins of each name and address within the window (ADR-0065). A name may fail
/// `max_failures` times within `window`, and an address four times as often, as a lab's shared
/// computer serves many; past that, logins are refused until the oldest failure leaves the
/// window, even with the right password.
#[derive(Debug)]
pub struct LoginLimiter {
    clock: Arc<dyn Clock>,
    max_failures: usize,
    window: Duration,
    failures: Mutex<HashMap<Key, VecDeque<Timestamp>>>,
}

impl LoginLimiter {
    /// No failures yet, timed by `clock`.
    pub fn new(clock: Arc<dyn Clock>, max_failures: usize, window: Duration) -> Self {
        Self {
            clock,
            max_failures: max_failures.max(1),
            window,
            failures: Mutex::default(),
        }
    }

    fn keys(name: &str, address: Option<IpAddr>) -> impl Iterator<Item = (Key, usize)> {
        let name = (Key::Name(name.to_owned()), 1);
        let address = address.map(|address| (Key::Address(address), 4));
        std::iter::once(name).chain(address)
    }

    /// How long `name`, from `address`, must wait before trying again: `None` if it may now.
    pub fn wait(&self, name: &str, address: Option<IpAddr>) -> Option<Duration> {
        let now = self.clock.now();
        let mut failures = self.failures.lock().unwrap_or_else(PoisonError::into_inner);
        Self::keys(name, address)
            .filter_map(|(key, share)| {
                let times = failures.get_mut(&key)?;
                while times
                    .front()
                    .is_some_and(|&t| now.saturating_since(t) >= self.window)
                {
                    times.pop_front();
                }
                // Past the limit, the wait is until enough of the oldest have left the window.
                let over = times.len().checked_sub(self.max_failures * share)?;
                times
                    .get(over)
                    .map(|&t| self.window - now.saturating_since(t))
            })
            .max()
    }

    /// Counts a failed login of `name` from `address`.
    pub fn failed(&self, name: &str, address: Option<IpAddr>) {
        let now = self.clock.now();
        let mut failures = self.failures.lock().unwrap_or_else(PoisonError::into_inner);
        for (key, _) in Self::keys(name, address) {
            failures.entry(key).or_default().push_back(now);
        }
    }

    /// Forgets `name`'s failures, after it logged in. The address's stay counted.
    pub fn succeeded(&self, name: &str) {
        let mut failures = self.failures.lock().unwrap_or_else(PoisonError::into_inner);
        failures.remove(&Key::Name(name.to_owned()));
    }
}
