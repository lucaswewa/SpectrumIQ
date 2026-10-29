//! What the camera needs of a frame grabber: eGrabber's, through the shim (`sdk`), or the tests'
//! fake. A grabber is used from one thread at a time.

use std::time::Duration;

use serde::Deserialize;
use siq_devices::DeviceError;

/// A grabber's GenTL module, whose features are read and written by name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Module {
    /// The frame grabber card: I/O, PCIe and firmware.
    Interface,
    /// The grabber's side of the camera link: its triggers and strobes.
    Device,
    /// The data stream: buffers, unpacking.
    Stream,
    /// The camera itself.
    Remote,
}

/// A filled buffer, popped: it stays the camera's until requeued.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Popped {
    /// Its index in the order announced.
    pub index: usize,
    /// The bytes filled.
    pub bytes: usize,
    /// When it was filled, in ns of the grabber's clock.
    pub timestamp_ns: u64,
    /// The camera's frame number.
    pub frame_id: u64,
    /// Something went wrong while filling it.
    pub incomplete: bool,
}

/// What the data stream has done since acquisition started.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counters {
    /// Buffers filled.
    pub delivered: u64,
    /// Frames lost because no buffer was queued to fill.
    pub underrun: u64,
}

/// Why a grabber call failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct GrabberError {
    /// What kind of failure it was.
    pub kind: ErrorKind,
    /// The grabber's message, with what was being done.
    pub message: String,
}

/// The kinds of [`GrabberError`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    /// No camera, or no feature, of that name.
    NotFound,
    /// A value the grabber or the camera refused.
    InvalidArgument,
    /// Anything else: the grabber or the camera failed.
    Fault,
}

impl GrabberError {
    /// A failure of `kind`, saying `message`.
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}

impl From<GrabberError> for DeviceError {
    fn from(error: GrabberError) -> Self {
        match error.kind {
            ErrorKind::NotFound => DeviceError::Unsupported(error.message),
            ErrorKind::InvalidArgument => DeviceError::InvalidArgument(error.message),
            ErrorKind::Fault => DeviceError::Fault(error.message),
        }
    }
}

/// A grabber, with its camera open.
pub trait Grabber: Send + 'static {
    /// A feature's value, as text: GenICam converts any type. `Feature.Max`, `.Min` and `.Inc`
    /// read a number's limits.
    fn get(&mut self, module: Module, feature: &str) -> Result<String, GrabberError>;

    /// Sets a feature, from text.
    fn set(&mut self, module: Module, feature: &str, value: &str) -> Result<(), GrabberError>;

    /// Executes a command feature.
    fn execute(&mut self, module: Module, command: &str) -> Result<(), GrabberError>;

    /// The entries an enumeration offers now.
    fn entries(&mut self, module: Module, feature: &str) -> Result<Vec<String>, GrabberError>;

    /// Announces `buffers`, of `size` bytes each, in order, and queues them to be filled: a
    /// popped buffer's index is its place here. It revokes any announced before; none revokes them
    /// all.
    ///
    /// # Safety
    ///
    /// Each buffer must be valid for `size` bytes of writes by DMA, and nothing else may read or
    /// write it while it is queued, until the grabber is stopped and announces again, or is
    /// dropped.
    unsafe fn announce(&mut self, buffers: &[*mut u8], size: usize) -> Result<(), GrabberError>;

    /// Starts filling the buffers queued.
    fn start(&mut self) -> Result<(), GrabberError>;

    /// Stops filling, and flushes the queue.
    fn stop(&mut self) -> Result<(), GrabberError>;

    /// The next filled buffer, or `None` if none was filled within `timeout`.
    fn pop(&mut self, timeout: Duration) -> Result<Option<Popped>, GrabberError>;

    /// Queues popped buffer `index` to be filled again.
    fn requeue(&mut self, index: usize) -> Result<(), GrabberError>;

    /// What the stream has done since it started.
    fn counters(&mut self) -> Result<Counters, GrabberError>;
}
