//! The pool: its configuration, its slots, and where released slots go.

use std::fmt;
use std::io;
use std::sync::Arc;

use siq_telemetry::{MetricError, Registry};
use siq_types::Clock;

use crate::alloc::{Backend, PAGE, Region, WorkingSetReservation};
use crate::slot::{FillGuard, Inner, Metrics};

/// The bounds of the lease-duration histogram, in ms.
const LEASE_MS: [f64; 12] = [
    1.0, 2.0, 5.0, 10.0, 20.0, 50.0, 100.0, 200.0, 500.0, 1000.0, 2000.0, 5000.0,
];

/// What happens to a slot when its last lease drops.
///
/// The sink gets the slot as a [`FillGuard`]. It may hand it to the producer again, as a grabber
/// driver requeues its buffer, or drop it, which returns the slot to the free list. It runs on the
/// thread that dropped the lease, so it must be quick and must not block.
pub trait ReleaseSink: Send + Sync {
    /// Takes back a released slot.
    fn release(&self, slot: FillGuard);
}

/// The default sink: released slots go back to the free list.
#[derive(Debug, Default, Clone, Copy)]
pub struct ReturnToPool;

impl ReleaseSink for ReturnToPool {
    fn release(&self, slot: FillGuard) {
        drop(slot);
    }
}

/// How a pool is made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PoolConfig {
    /// The prefix of the pool's metric names, such as `camera.pool`.
    pub name: String,
    /// The number of slots, at least 1.
    pub slots: u16,
    /// The bytes each slot holds at least: a full frame. It rounds up to whole pages.
    pub slot_bytes: usize,
    /// Where the memory comes from.
    pub backend: Backend,
}

/// Why a pool couldn't be made.
#[derive(Debug, thiserror::Error)]
pub enum PoolError {
    /// The configuration asks for no slots, or for empty or impossibly large ones.
    #[error("a pool needs at least one slot of 1 to {} bytes", isize::MAX)]
    InvalidSize,
    /// The memory isn't available.
    #[error("{bytes} bytes for the frame pool can't be allocated: {source}")]
    Allocation {
        /// The pool's total size.
        bytes: usize,
        /// The system's complaint.
        source: io::Error,
    },
    /// The memory can't be locked in RAM.
    #[error(
        "{bytes} bytes of frame pool can't be locked in RAM: {source}. Use fewer slots \
         ([memory] pool_slots), or free memory for the service"
    )]
    Lock {
        /// The pool's total size.
        bytes: usize,
        /// The system's complaint.
        source: io::Error,
    },
    /// A metric of the pool clashes with an existing one.
    #[error(transparent)]
    Metrics(#[from] MetricError),
}

/// A snapshot of where the slots are.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PoolStats {
    /// All slots.
    pub slots: usize,
    /// In the free list.
    pub free: usize,
    /// Held by a [`FillGuard`]: being filled, or queued at a grabber.
    pub filling: usize,
    /// Held by leases.
    pub leased: usize,
    /// The most slots that have been out of the free list at once.
    pub high_water: usize,
}

/// The frame pool: slots allocated once, filled under a [`FillGuard`], read through
/// [`FrameLease`](crate::FrameLease)s.
///
/// Clones share the pool. Its memory is freed when the last clone, guard and lease are gone.
#[derive(Clone)]
pub struct FramePool {
    inner: Arc<Inner>,
}

impl FramePool {
    /// Allocates the slots, and, with [`Backend::WindowsLocked`], locks them in RAM.
    ///
    /// With a `registry`, the pool reports `<name>.free_slots`, `<name>.in_use_high_water` and
    /// `<name>.lease_ms`.
    pub fn new(
        config: &PoolConfig,
        clock: Arc<dyn Clock>,
        registry: Option<&Registry>,
    ) -> Result<Self, PoolError> {
        let slot_bytes = config
            .slot_bytes
            .checked_next_multiple_of(PAGE)
            .filter(|bytes| *bytes > 0 && *bytes <= isize::MAX as usize)
            .ok_or(PoolError::InvalidSize)?;
        let total = slot_bytes
            .checked_mul(usize::from(config.slots))
            .filter(|total| *total > 0)
            .ok_or(PoolError::InvalidSize)?;
        let metrics = match registry {
            Some(registry) => Some(Metrics {
                free: registry.gauge(&format!("{}.free_slots", config.name))?,
                high_water: registry.gauge(&format!("{}.in_use_high_water", config.name))?,
                lease_ms: registry.histogram(&format!("{}.lease_ms", config.name), &LEASE_MS)?,
            }),
            None => None,
        };
        let allocation = |source| PoolError::Allocation {
            bytes: total,
            source,
        };
        let regions = (0..config.slots)
            .map(|_| Region::allocate(slot_bytes, config.backend))
            .collect::<io::Result<Vec<_>>>()
            .map_err(allocation)?;
        let reservation = match config.backend {
            Backend::Heap => None,
            Backend::WindowsLocked => {
                let lock = |source| PoolError::Lock {
                    bytes: total,
                    source,
                };
                let reservation = WorkingSetReservation::raise(total).map_err(lock)?;
                for region in &regions {
                    region.lock().map_err(lock)?;
                }
                Some(reservation)
            }
        };
        let sink: Arc<dyn ReleaseSink> = Arc::new(ReturnToPool);
        let inner = Inner::new(regions, reservation, sink, clock, metrics);
        Ok(Self {
            inner: Arc::new(inner),
        })
    }

    /// A Free slot, to fill, if there is one.
    pub fn try_acquire_free(&self) -> Option<FillGuard> {
        self.inner.take_free()
    }

    /// Every Free slot, to fill or to announce to a grabber.
    pub fn take_all_free(&self) -> Vec<FillGuard> {
        std::iter::from_fn(|| self.inner.take_free()).collect()
    }

    /// Where slots go when their last lease drops: to a grabber, for instance. Leases dropped
    /// after this call use the new sink.
    pub fn set_release_sink(&self, sink: Arc<dyn ReleaseSink>) {
        self.inner.set_sink(sink);
    }

    /// Sends released slots back to the free list again, as when a camera stops.
    pub fn reset_release_sink(&self) {
        self.inner.set_sink(Arc::new(ReturnToPool));
    }

    /// The number of slots.
    pub fn slots(&self) -> usize {
        self.inner.slots()
    }

    /// The bytes per slot: a whole number of pages.
    pub fn slot_bytes(&self) -> usize {
        self.inner.slot_bytes()
    }

    /// Where the slots are now.
    pub fn stats(&self) -> PoolStats {
        let (free, leased, high_water) = self.inner.counts();
        let slots = self.slots();
        PoolStats {
            slots,
            free,
            filling: slots.saturating_sub(free + leased),
            leased,
            high_water,
        }
    }
}

impl fmt::Debug for FramePool {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FramePool")
            .field("slots", &self.slots())
            .field("slot_bytes", &self.slot_bytes())
            .finish()
    }
}
