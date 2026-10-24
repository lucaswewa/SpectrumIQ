//! The slots, and the two ways to hold one: a [`FillGuard`] to write it, [`FrameLease`]s to read
//! it.
//!
//! # The ownership protocol
//!
//! The unsafe code in this module is sound because of these rules, which the types enforce.
//!
//! - **Exactly one state.** A slot is *Free* (in the free list), *Filling* (exactly one `FillGuard`
//!   holds it) or *Leased* (`refs > 0`: `FrameLease`s hold it, and no guard does).
//! - **Guards are made only:**
//!   - by taking a slot out of the free list, which happens once per entry, under its lock; or
//!   - when a slot's last lease drops (`refs` reaches 0), which happens once per publish.
//!
//!   Two guards for one slot never exist.
//! - **Leases are made only** by [`FillGuard::publish`], which consumes the guard, and by cloning a
//!   lease.
//! - **So:** a guard's `&mut [u8]` is the only reference to its slot, and leases' `&[u8]`s coexist
//!   with no writer.
//! - **Happens-before.** Publishing stores `refs = 1` with `Release`. A lease reaches another thread
//!   only through a synchronizing channel. The last drop decrements with `Release`, then fences with
//!   `Acquire` before making the guard, as `Arc` does. Every write before a publish is therefore
//!   seen by every reader, and every read is done before the slot is written again.

#![allow(unsafe_code)]

use std::fmt;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering, fence};
use std::sync::{Arc, Mutex, PoisonError, RwLock};
use std::time::Duration;

use siq_telemetry::{Gauge, Histogram};
use siq_types::{Clock, FrameMeta, ImageView, ImageViewMut, Timestamp};

use crate::alloc::{Region, WorkingSetReservation};
use crate::pool::ReleaseSink;

/// A type a slot can be viewed as: no padding, and every bit pattern a valid value.
pub trait Pixel: Copy + sealed::Sealed {}
impl Pixel for u8 {}
impl Pixel for u16 {}

mod sealed {
    pub trait Sealed {}
    impl Sealed for u8 {}
    impl Sealed for u16 {}
}

/// The pool's metrics, if it has a registry.
pub(crate) struct Metrics {
    pub(crate) free: Arc<Gauge>,
    pub(crate) high_water: Arc<Gauge>,
    pub(crate) lease_ms: Arc<Histogram>,
}

/// What every guard and lease of a pool shares.
pub(crate) struct Inner {
    regions: Box<[Region]>,
    /// Leases per slot. 0 unless Leased.
    refs: Box<[AtomicU32]>,
    /// When each slot was last published, in nanoseconds of `clock`.
    published: Box<[AtomicU64]>,
    /// The Free slots. Its capacity is the slot count, so it never allocates after construction.
    free: Mutex<Vec<u32>>,
    sink: RwLock<Arc<dyn ReleaseSink>>,
    clock: Arc<dyn Clock>,
    /// Slots not Free: Filling or Leased.
    in_use: AtomicU32,
    high_water: AtomicU32,
    metrics: Option<Metrics>,
    /// Dropped after the regions, which are declared first.
    _reservation: Option<WorkingSetReservation>,
}

impl Inner {
    pub(crate) fn new(
        regions: Vec<Region>,
        reservation: Option<WorkingSetReservation>,
        sink: Arc<dyn ReleaseSink>,
        clock: Arc<dyn Clock>,
        metrics: Option<Metrics>,
    ) -> Self {
        let count = regions.len();
        let inner = Self {
            refs: (0..count).map(|_| AtomicU32::new(0)).collect(),
            published: (0..count).map(|_| AtomicU64::new(0)).collect(),
            // Popped from the end: slot 0 is handed out first.
            free: Mutex::new((0..count as u32).rev().collect()),
            regions: regions.into(),
            sink: RwLock::new(sink),
            clock,
            in_use: AtomicU32::new(0),
            high_water: AtomicU32::new(0),
            metrics,
            _reservation: reservation,
        };
        inner.report_free(count);
        inner
    }

    pub(crate) fn slots(&self) -> usize {
        self.regions.len()
    }

    pub(crate) fn slot_bytes(&self) -> usize {
        self.regions[0].len()
    }

    /// A guard for a Free slot, if there is one.
    pub(crate) fn take_free(self: &Arc<Self>) -> Option<FillGuard> {
        let mut free = self.free.lock().unwrap_or_else(PoisonError::into_inner);
        let slot = free.pop()?;
        let left = free.len();
        drop(free);
        let in_use = self.in_use.fetch_add(1, Ordering::Relaxed) + 1;
        let high = self
            .high_water
            .fetch_max(in_use, Ordering::Relaxed)
            .max(in_use);
        self.report_free(left);
        if let Some(metrics) = &self.metrics {
            metrics.high_water.set(f64::from(high));
        }
        Some(FillGuard {
            pool: Some(Arc::clone(self)),
            slot,
        })
    }

    /// Puts a slot back in the free list. Only a dropped guard calls this.
    fn give_back(&self, slot: u32) {
        let mut free = self.free.lock().unwrap_or_else(PoisonError::into_inner);
        free.push(slot);
        let left = free.len();
        drop(free);
        self.in_use.fetch_sub(1, Ordering::Relaxed);
        self.report_free(left);
    }

    fn report_free(&self, free: usize) {
        if let Some(metrics) = &self.metrics {
            metrics.free.set(free as f64);
        }
    }

    pub(crate) fn set_sink(&self, sink: Arc<dyn ReleaseSink>) {
        *self.sink.write().unwrap_or_else(PoisonError::into_inner) = sink;
    }

    /// Free, Leased, and the high-water mark of slots in use.
    pub(crate) fn counts(&self) -> (usize, usize, usize) {
        let free = self
            .free
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len();
        let leased = self
            .refs
            .iter()
            .filter(|refs| refs.load(Ordering::Relaxed) > 0)
            .count();
        (
            free,
            leased,
            self.high_water.load(Ordering::Relaxed) as usize,
        )
    }

    fn bytes(&self, slot: u32) -> (*mut u8, usize) {
        let region = &self.regions[slot as usize];
        (region.ptr(), region.len())
    }
}

/// Exclusive ownership of a slot, to fill it: from the free list, or handed back by the release
/// sink when the slot's last lease drops.
///
/// Publishing turns it into the slot's first [`FrameLease`]. Dropping it returns the slot to the
/// free list.
pub struct FillGuard {
    /// `Some` until the guard is published or dropped.
    pool: Option<Arc<Inner>>,
    slot: u32,
}

impl FillGuard {
    fn inner(&self) -> &Inner {
        self.pool
            .as_deref()
            .expect("a guard holds its pool until it is consumed")
    }

    /// The slot's index in the pool.
    pub fn slot(&self) -> usize {
        self.slot as usize
    }

    /// The slot's size in bytes: a whole number of pages.
    pub fn len(&self) -> usize {
        self.inner().slot_bytes()
    }

    /// Whether the slot has no bytes. It never has none.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The slot's first byte, page-aligned, for announcing the slot to a grabber.
    ///
    /// Writes through it must stay within [`len`](Self::len) bytes, and may happen only while this
    /// guard is held. While a grabber may be writing (the buffer is queued), don't read or write
    /// the slot through the guard's other methods: the types can't see the DMA, so that rule is the
    /// driver's to keep.
    pub fn as_mut_ptr(&mut self) -> *mut u8 {
        self.inner().bytes(self.slot).0
    }

    /// The slot's bytes, to write.
    pub fn bytes_mut(&mut self) -> &mut [u8] {
        let (ptr, len) = self.inner().bytes(self.slot);
        // SAFETY: the guard is the slot's only holder (module docs), the region is live while the
        // pool is, and `&mut self` makes this the only borrow of the guard.
        unsafe { std::slice::from_raw_parts_mut(ptr, len) }
    }

    /// The slot as pixels, to write.
    pub fn pixels_mut<T: Pixel>(&mut self) -> &mut [T] {
        let (ptr, len) = self.inner().bytes(self.slot);
        // SAFETY: as `bytes_mut`. The slot is page-aligned, so aligned for `T`. A slot is whole
        // pages, so a whole number of `T`s. `T` has no padding and no invalid values.
        unsafe { std::slice::from_raw_parts_mut(ptr.cast::<T>(), len / size_of::<T>()) }
    }

    /// The first `width × height` pixels of the slot, as an image with contiguous rows.
    pub fn image_mut<T: Pixel>(
        &mut self,
        width: usize,
        height: usize,
    ) -> Result<ImageViewMut<'_, T>, siq_types::ImageError> {
        ImageViewMut::new(self.pixels_mut(), width, height, width)
    }

    /// Makes the filled slot readable, as its first lease.
    ///
    /// Fails, giving the guard back, if `meta` describes an image larger than the slot.
    pub fn publish(mut self, meta: FrameMeta) -> Result<FrameLease, PublishError> {
        let needed = payload_len(&meta);
        let capacity = self.len();
        if needed.is_none_or(|needed| needed > capacity) {
            return Err(PublishError {
                needed,
                capacity,
                guard: self,
            });
        }
        let pool = self
            .pool
            .take()
            .expect("a guard holds its pool until it is consumed");
        let slot = self.slot as usize;
        pool.published[slot].store(pool.clock.now().0, Ordering::Relaxed);
        pool.refs[slot].store(1, Ordering::Release);
        Ok(FrameLease {
            pool,
            slot: self.slot,
            meta,
        })
    }
}

impl Drop for FillGuard {
    fn drop(&mut self) {
        if let Some(pool) = self.pool.take() {
            pool.give_back(self.slot);
        }
    }
}

impl fmt::Debug for FillGuard {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FillGuard")
            .field("slot", &self.slot)
            .finish()
    }
}

/// The bytes the image of `meta` takes, or `None` if that overflows.
fn payload_len(meta: &FrameMeta) -> Option<usize> {
    let (width, height) = meta.image_size();
    (width as usize)
        .checked_mul(height as usize)?
        .checked_mul(meta.pixel_format.bytes_per_pixel())
}

/// A publish that failed: the image doesn't fit the slot. The guard is given back.
#[derive(Debug)]
pub struct PublishError {
    /// The bytes the image needs, or `None` if that is beyond `usize`.
    pub needed: Option<usize>,
    /// The slot's size.
    pub capacity: usize,
    /// The slot, still Filling.
    pub guard: FillGuard,
}

impl fmt::Display for PublishError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.needed {
            Some(needed) => write!(
                f,
                "the image needs {needed} bytes, the slot has {}",
                self.capacity
            ),
            None => write!(f, "the image is too large to measure"),
        }
    }
}

impl std::error::Error for PublishError {}

/// A shared, read-only hold on a published frame: its slot and its metadata.
///
/// Clones are cheap: an atomic increment and a copy of the metadata. When the last lease of a
/// slot drops, the slot goes to the pool's release sink, on the dropping thread.
pub struct FrameLease {
    pool: Arc<Inner>,
    slot: u32,
    meta: FrameMeta,
}

impl FrameLease {
    /// The frame's metadata.
    pub fn meta(&self) -> &FrameMeta {
        &self.meta
    }

    /// The slot's index in the pool.
    pub fn slot(&self) -> usize {
        self.slot as usize
    }

    /// The image's bytes: width × height × bytes per pixel, from the slot's start.
    pub fn as_bytes(&self) -> &[u8] {
        let (ptr, _) = self.pool.bytes(self.slot);
        let len = payload_len(&self.meta).expect("checked when published");
        // SAFETY: the slot is Leased while this lease lives, so nothing writes it (module docs).
        // `len` fits the slot, as checked when published.
        unsafe { std::slice::from_raw_parts(ptr, len) }
    }

    /// The image, if `T` is the size of the frame's pixels.
    pub fn image<T: Pixel>(&self) -> Option<ImageView<'_, T>> {
        if size_of::<T>() != self.meta.pixel_format.bytes_per_pixel() {
            return None;
        }
        let (ptr, _) = self.pool.bytes(self.slot);
        let (width, height) = self.meta.image_size();
        let (width, height) = (width as usize, height as usize);
        // SAFETY: as `as_bytes`. The slot is page-aligned, so aligned for `T`, and the image's
        // `width × height` pixels of `T` fit it, as checked when published.
        let pixels = unsafe { std::slice::from_raw_parts(ptr.cast::<T>(), width * height) };
        ImageView::new(pixels, width, height, width).ok()
    }
}

impl Clone for FrameLease {
    fn clone(&self) -> Self {
        // Relaxed, as for `Arc`: the new lease is made from one that already holds the slot.
        self.pool.refs[self.slot as usize].fetch_add(1, Ordering::Relaxed);
        Self {
            pool: Arc::clone(&self.pool),
            slot: self.slot,
            meta: self.meta,
        }
    }
}

impl Drop for FrameLease {
    fn drop(&mut self) {
        let slot = self.slot as usize;
        if self.pool.refs[slot].fetch_sub(1, Ordering::Release) != 1 {
            return;
        }
        fence(Ordering::Acquire);
        if let Some(metrics) = &self.pool.metrics {
            let published = Timestamp(self.pool.published[slot].load(Ordering::Relaxed));
            let held: Duration = self.pool.clock.now().saturating_since(published);
            metrics.lease_ms.record(held.as_secs_f64() * 1e3);
        }
        let guard = FillGuard {
            pool: Some(Arc::clone(&self.pool)),
            slot: self.slot,
        };
        let sink = Arc::clone(
            &self
                .pool
                .sink
                .read()
                .unwrap_or_else(PoisonError::into_inner),
        );
        sink.release(guard);
    }
}

impl fmt::Debug for FrameLease {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FrameLease")
            .field("slot", &self.slot)
            .field("seq", &self.meta.seq)
            .finish()
    }
}
