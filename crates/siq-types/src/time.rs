//! Monotonic timestamps, and the [`Clock`] that code which timestamps takes.
//!
//! A service shares one `Arc<dyn Clock>`: timestamps from different clocks aren't comparable.
//! Tests use [`ManualClock`], which only moves when told to, so that timing is deterministic.

#[cfg(test)]
mod tests;

use std::fmt;
use std::ops::{Add, Sub};
use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use chrono::{DateTime, TimeDelta, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A point on a monotonic clock: nanoseconds since the clock's epoch.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(transparent)]
pub struct Timestamp(pub u64);

impl Timestamp {
    /// The time from `earlier` to this one, or zero if `earlier` is later.
    pub fn saturating_since(self, earlier: Timestamp) -> Duration {
        Duration::from_nanos(self.0.saturating_sub(earlier.0))
    }
}

fn nanos(duration: Duration) -> u64 {
    u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX)
}

/// Later by `duration`, saturating at the end of time.
impl Add<Duration> for Timestamp {
    type Output = Self;
    fn add(self, duration: Duration) -> Self {
        Self(self.0.saturating_add(nanos(duration)))
    }
}

/// Earlier by `duration`, saturating at the epoch.
impl Sub<Duration> for Timestamp {
    type Output = Self;
    fn sub(self, duration: Duration) -> Self {
        Self(self.0.saturating_sub(nanos(duration)))
    }
}

/// The time, for code that timestamps.
pub trait Clock: Send + Sync + fmt::Debug {
    /// Now, on the monotonic clock.
    fn now(&self) -> Timestamp;

    /// The wall-clock time of `timestamp`, in UTC.
    fn utc(&self, timestamp: Timestamp) -> DateTime<Utc>;

    /// Now, in UTC.
    fn now_utc(&self) -> DateTime<Utc> {
        self.utc(self.now())
    }

    /// Waits until the clock reaches `deadline`, or until `timeout` of real time has passed, and
    /// says whether the deadline was reached. A thread that must also notice other events, such
    /// as a camera being stopped, waits in steps of `timeout`.
    fn wait_until(&self, deadline: Timestamp, timeout: Duration) -> bool;

    /// Blocks until the clock reaches `deadline`.
    fn sleep_until(&self, deadline: Timestamp) {
        while !self.wait_until(deadline, Duration::from_secs(3600)) {}
    }
}

/// The system's monotonic clock, with its epoch at the clock's creation.
#[derive(Debug, Clone, Copy)]
pub struct SystemClock {
    epoch: Instant,
    epoch_utc: DateTime<Utc>,
}

impl SystemClock {
    /// A clock whose epoch is now.
    pub fn new() -> Self {
        Self {
            epoch: Instant::now(),
            epoch_utc: Utc::now(),
        }
    }
}

impl Default for SystemClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for SystemClock {
    fn now(&self) -> Timestamp {
        Timestamp(nanos(self.epoch.elapsed()))
    }

    /// Sleeps the thread.
    fn wait_until(&self, deadline: Timestamp, timeout: Duration) -> bool {
        let remaining = deadline.saturating_since(self.now());
        std::thread::sleep(remaining.min(timeout));
        self.now() >= deadline
    }

    fn utc(&self, timestamp: Timestamp) -> DateTime<Utc> {
        utc_after(self.epoch_utc, timestamp)
    }
}

/// A clock that moves only when [`advance`](Self::advance) or [`set`](Self::set) is called.
/// A thread waiting on it wakes when another thread moves it past the deadline.
#[derive(Debug)]
pub struct ManualClock {
    now: Mutex<Timestamp>,
    moved: Condvar,
    epoch_utc: DateTime<Utc>,
}

impl ManualClock {
    /// A clock at its epoch, which is 2026-01-01T00:00:00Z.
    pub fn new() -> Self {
        Self::starting_at(DateTime::from_timestamp(1_767_225_600, 0).unwrap_or_default())
    }

    /// A clock at its epoch, which is `epoch_utc`.
    pub fn starting_at(epoch_utc: DateTime<Utc>) -> Self {
        let now = Mutex::new(Timestamp(0));
        Self {
            now,
            moved: Condvar::new(),
            epoch_utc,
        }
    }

    /// Moves the clock forward by `duration`.
    pub fn advance(&self, duration: Duration) {
        let mut now = self.lock();
        *now = *now + duration;
        self.moved.notify_all();
    }

    /// Moves the clock to `timestamp`, if that is later. A monotonic clock never goes back.
    pub fn set(&self, timestamp: Timestamp) {
        let mut now = self.lock();
        *now = (*now).max(timestamp);
        self.moved.notify_all();
    }

    fn lock(&self) -> MutexGuard<'_, Timestamp> {
        self.now.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Default for ManualClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for ManualClock {
    fn now(&self) -> Timestamp {
        *self.lock()
    }

    /// Blocks on a condition variable until another thread moves the clock past `deadline`, or
    /// `timeout` of real time passes.
    fn wait_until(&self, deadline: Timestamp, timeout: Duration) -> bool {
        let now = self.lock();
        let (now, _) = self
            .moved
            .wait_timeout_while(now, timeout, |now| *now < deadline)
            .unwrap_or_else(PoisonError::into_inner);
        *now >= deadline
    }

    fn utc(&self, timestamp: Timestamp) -> DateTime<Utc> {
        utc_after(self.epoch_utc, timestamp)
    }
}

fn utc_after(epoch: DateTime<Utc>, timestamp: Timestamp) -> DateTime<Utc> {
    // i64 nanoseconds reach 292 years past the epoch; later saturates.
    let offset = TimeDelta::nanoseconds(i64::try_from(timestamp.0).unwrap_or(i64::MAX));
    epoch
        .checked_add_signed(offset)
        .unwrap_or(DateTime::<Utc>::MAX_UTC)
}
