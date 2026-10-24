//! The frame pool: pre-allocated, page-aligned slots that frames live in, from the camera's DMA
//! to the last consumer, with no allocation and no copy (ADR-0005, ADR-0021).
//!
//! - [`FramePool`] allocates its slots once, from the heap or locked in RAM ([`Backend`]).
//! - A producer fills a slot under a [`FillGuard`], and publishes it as a [`FrameLease`] with the
//!   frame's metadata. A grabber holds guards for the buffers it has queued.
//! - Consumers share the frame through cloned leases, read-only.
//! - When the last lease drops, the slot goes to the pool's [`ReleaseSink`]: back to the grabber,
//!   or back to the free list.
//!
//! The unsafe code is in `alloc` (memory, locking) and `slot` (the ownership protocol that makes
//! sharing the bytes sound, described there).

mod alloc;
mod pool;
mod slot;

pub use alloc::{Backend, PAGE, working_set};
pub use pool::{FramePool, PoolConfig, PoolError, PoolStats, ReleaseSink, ReturnToPool};
pub use slot::{FillGuard, FrameLease, Pixel, PublishError};
