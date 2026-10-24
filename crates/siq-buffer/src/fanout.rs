//! The fan-out: every published frame goes to every subscriber whose filter takes it, and the
//! producer never waits (ADR-0022).
//!
//! Each subscriber has a mailbox with a [`Policy`]:
//!
//! - [`LatestOnly`](Policy::LatestOnly): one frame, the newest, for previews and statistics;
//! - [`Bounded`](Policy::Bounded): up to `capacity` frames; when full, the oldest or the newest is
//!   dropped;
//! - [`Lossless`](Policy::Lossless): every frame. A slow lossless consumer keeps slots, so the pool
//!   runs dry, and the producer loses frames: that is the backpressure.
//!
//! Mailboxes are allocated at subscription, so publishing never allocates. Frames a policy drops
//! are released after the mailbox's lock is let go, so a release sink never runs under it.
//! Receiving works from threads (`recv_blocking`, `recv_timeout`) and from async code (`recv`,
//! with any executor).

use std::collections::VecDeque;
use std::fmt;
use std::future::Future;
use std::num::NonZeroUsize;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError, RwLock};
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use siq_types::FrameMeta;

use crate::{FrameLease, FramePool};

/// How a subscriber's mailbox keeps frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Policy {
    /// Only the newest frame: a new frame replaces an unread one.
    LatestOnly,
    /// Up to `capacity` frames, in order.
    Bounded {
        /// The most frames kept.
        capacity: NonZeroUsize,
        /// Which frame goes when the mailbox is full.
        overflow: Overflow,
    },
    /// Every frame. The pool's slot count bounds how many can wait.
    Lossless,
}

/// Which frame a full [`Policy::Bounded`] mailbox drops.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Overflow {
    /// The oldest waiting frame, so the mailbox keeps the newest ones.
    DropOldest,
    /// The new frame, so the mailbox keeps the oldest ones.
    DropNewest,
}

/// What a subscriber has received, for monitoring. `received = taken + dropped + still queued`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SubscriberStats {
    /// Frames offered to it: those its filter took.
    pub received: u64,
    /// Frames its policy dropped, unread.
    pub dropped: u64,
    /// Frames it has read.
    pub taken: u64,
    /// The most frames that have waited in its mailbox at once: how far behind it has been.
    pub max_queued: usize,
}

/// Why [`Subscriber::recv_timeout`] returned no frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RecvError {
    /// No frame came in time.
    #[error("no frame came in time")]
    Timeout,
    /// The fan-out is gone, and every frame has been read.
    #[error("the fan-out is closed")]
    Closed,
}

type Filter = Box<dyn Fn(&FrameMeta) -> bool + Send + Sync>;

struct Mailbox {
    name: String,
    policy: Policy,
    filter: Filter,
    state: Mutex<State>,
    ready: Condvar,
}

struct State {
    queue: VecDeque<FrameLease>,
    waker: Option<Waker>,
    closed: bool,
    stats: SubscriberStats,
}

impl State {
    fn take(&mut self) -> Option<FrameLease> {
        let lease = self.queue.pop_front()?;
        self.stats.taken += 1;
        Some(lease)
    }
}

impl Mailbox {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Delivers a frame, and says whether the mailbox took it. A frame the policy drops is
    /// released after the mailbox is unlocked.
    fn offer(&self, lease: FrameLease) -> bool {
        let mut state = self.lock();
        if state.closed {
            drop(state);
            return false;
        }
        state.stats.received += 1;
        let dropped = match self.policy {
            Policy::LatestOnly => state.queue.pop_front(),
            Policy::Bounded { capacity, overflow } if state.queue.len() >= capacity.get() => {
                match overflow {
                    Overflow::DropOldest => state.queue.pop_front(),
                    Overflow::DropNewest => {
                        state.stats.dropped += 1;
                        drop(state);
                        return true;
                    }
                }
            }
            _ => None,
        };
        state.stats.dropped += u64::from(dropped.is_some());
        state.queue.push_back(lease);
        state.stats.max_queued = state.stats.max_queued.max(state.queue.len());
        let waker = state.waker.take();
        drop(state);
        drop(dropped);
        self.ready.notify_one();
        if let Some(waker) = waker {
            waker.wake();
        }
        true
    }

    fn close(&self) {
        let waker = {
            let mut state = self.lock();
            state.closed = true;
            state.waker.take()
        };
        self.ready.notify_all();
        if let Some(waker) = waker {
            waker.wake();
        }
    }
}

struct Shared {
    subscribers: RwLock<Vec<(u64, Arc<Mailbox>)>>,
    next_id: AtomicU64,
    /// Live `Fanout` handles: when the last goes, the fan-out closes.
    handles: AtomicUsize,
    /// Set before the mailboxes are closed, and read by `subscribe` under the write lock, so a
    /// subscriber that joins during `close` is closed too.
    closed: AtomicBool,
    lossless_capacity: usize,
}

impl Shared {
    fn close(&self) {
        self.closed.store(true, Ordering::Release);
        let subscribers = self
            .subscribers
            .read()
            .unwrap_or_else(PoisonError::into_inner);
        for (_, mailbox) in subscribers.iter() {
            mailbox.close();
        }
    }
}

/// The producer's side: publishes frames to every subscriber.
///
/// Clones share the fan-out. When the last clone is dropped, or on [`close`](Self::close),
/// subscribers read what is queued and then get `None`.
pub struct Fanout {
    shared: Arc<Shared>,
}

impl Fanout {
    /// A fan-out for frames from `pool`, whose slot count bounds lossless mailboxes.
    pub fn new(pool: &FramePool) -> Self {
        Self {
            shared: Arc::new(Shared {
                subscribers: RwLock::new(Vec::new()),
                next_id: AtomicU64::new(0),
                handles: AtomicUsize::new(1),
                closed: AtomicBool::new(false),
                lossless_capacity: pool.slots(),
            }),
        }
    }

    /// A new subscriber, called `name`, that gets the frames `filter` takes, kept as `policy` says.
    pub fn subscribe(
        &self,
        name: impl Into<String>,
        policy: Policy,
        filter: impl Fn(&FrameMeta) -> bool + Send + Sync + 'static,
    ) -> Subscriber {
        let capacity = match policy {
            Policy::LatestOnly => 1,
            Policy::Bounded { capacity, .. } => capacity.get(),
            Policy::Lossless => self.shared.lossless_capacity,
        };
        let mailbox = Arc::new(Mailbox {
            name: name.into(),
            policy,
            filter: Box::new(filter),
            state: Mutex::new(State {
                queue: VecDeque::with_capacity(capacity),
                waker: None,
                closed: false,
                stats: SubscriberStats::default(),
            }),
            ready: Condvar::new(),
        });
        let id = self.shared.next_id.fetch_add(1, Ordering::Relaxed);
        let mut subscribers = self
            .shared
            .subscribers
            .write()
            .unwrap_or_else(PoisonError::into_inner);
        if self.shared.closed.load(Ordering::Acquire) {
            mailbox.lock().closed = true;
        }
        subscribers.push((id, Arc::clone(&mailbox)));
        Subscriber {
            shared: Arc::clone(&self.shared),
            id,
            mailbox,
        }
    }

    /// Offers `lease` to every subscriber, and returns how many took it. It never waits for a
    /// consumer.
    ///
    /// A frame a policy drops is released here, so the pool's release sink may run on this
    /// thread. It must not subscribe or unsubscribe.
    pub fn publish(&self, lease: &FrameLease) -> usize {
        let subscribers = self
            .shared
            .subscribers
            .read()
            .unwrap_or_else(PoisonError::into_inner);
        let mut delivered = 0;
        for (_, mailbox) in subscribers.iter() {
            if (mailbox.filter)(lease.meta()) && mailbox.offer(lease.clone()) {
                delivered += 1;
            }
        }
        delivered
    }

    /// Closes the fan-out now: subscribers read what is queued, then get `None`.
    pub fn close(&self) {
        self.shared.close();
    }

    /// Every subscriber's name and statistics.
    pub fn stats(&self) -> Vec<(String, SubscriberStats)> {
        let subscribers = self
            .shared
            .subscribers
            .read()
            .unwrap_or_else(PoisonError::into_inner);
        let stats = |mailbox: &Mailbox| (mailbox.name.clone(), mailbox.lock().stats);
        subscribers
            .iter()
            .map(|(_, mailbox)| stats(mailbox))
            .collect()
    }
}

impl Clone for Fanout {
    fn clone(&self) -> Self {
        self.shared.handles.fetch_add(1, Ordering::Relaxed);
        Self {
            shared: Arc::clone(&self.shared),
        }
    }
}

impl Drop for Fanout {
    fn drop(&mut self) {
        if self.shared.handles.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.shared.close();
        }
    }
}

impl fmt::Debug for Fanout {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Fanout")
            .field("subscribers", &self.stats())
            .finish()
    }
}

/// A consumer's side: receives the frames its filter takes, kept as its policy says.
///
/// Dropping it unsubscribes it, and releases the frames still in its mailbox.
pub struct Subscriber {
    shared: Arc<Shared>,
    id: u64,
    mailbox: Arc<Mailbox>,
}

impl Subscriber {
    /// The name given at subscription.
    pub fn name(&self) -> &str {
        &self.mailbox.name
    }

    /// What it has received so far.
    pub fn stats(&self) -> SubscriberStats {
        self.mailbox.lock().stats
    }

    /// The next frame, if one is waiting.
    pub fn try_recv(&mut self) -> Option<FrameLease> {
        self.mailbox.lock().take()
    }

    /// The next frame, waiting for it on this thread; `None` once the fan-out is closed and the
    /// mailbox is empty.
    pub fn recv_blocking(&mut self) -> Option<FrameLease> {
        let mut state = self.mailbox.lock();
        loop {
            if let Some(lease) = state.take() {
                return Some(lease);
            }
            if state.closed {
                return None;
            }
            state = self
                .mailbox
                .ready
                .wait(state)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }

    /// The next frame, waiting for it on this thread at most `timeout`.
    pub fn recv_timeout(&mut self, timeout: Duration) -> Result<FrameLease, RecvError> {
        let state = self.mailbox.lock();
        let waiting = |state: &mut State| state.queue.is_empty() && !state.closed;
        let (mut state, _) = self
            .mailbox
            .ready
            .wait_timeout_while(state, timeout, waiting)
            .unwrap_or_else(PoisonError::into_inner);
        match state.take() {
            Some(lease) => Ok(lease),
            None if state.closed => Err(RecvError::Closed),
            None => Err(RecvError::Timeout),
        }
    }

    /// The next frame, as a future: `None` once the fan-out is closed and the mailbox is empty.
    pub fn recv(&mut self) -> Recv<'_> {
        Recv { subscriber: self }
    }
}

impl Drop for Subscriber {
    fn drop(&mut self) {
        let mut subscribers = self
            .shared
            .subscribers
            .write()
            .unwrap_or_else(PoisonError::into_inner);
        // The list's handle on the mailbox goes; ours, and the frames still queued in it, go after
        // the lock is let go.
        subscribers.retain(|(id, _)| *id != self.id);
    }
}

impl fmt::Debug for Subscriber {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Subscriber")
            .field("name", &self.name())
            .field("stats", &self.stats())
            .finish()
    }
}

/// The future of [`Subscriber::recv`].
#[must_use = "a future does nothing until awaited"]
#[derive(Debug)]
pub struct Recv<'a> {
    subscriber: &'a mut Subscriber,
}

impl Future for Recv<'_> {
    type Output = Option<FrameLease>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut state = self.subscriber.mailbox.lock();
        if let Some(lease) = state.take() {
            return Poll::Ready(Some(lease));
        }
        if state.closed {
            return Poll::Ready(None);
        }
        match &mut state.waker {
            Some(waker) if waker.will_wake(cx.waker()) => {}
            slot => *slot = Some(cx.waker().clone()),
        }
        Poll::Pending
    }
}
