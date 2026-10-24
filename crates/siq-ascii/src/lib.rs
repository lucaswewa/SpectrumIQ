//! Data-driven ASCII dialects, and a focus drive that speaks one (REQ-STG-002, ADR-0030).
//!
//! **Dialects** (P1.10a) describe a device's serial protocol in a TOML file instead of code:
//!
//! - **A [`Dialect`]** gives the terminators, the step size, the timeouts, how the device moves and
//!   how the end of a move shows, the replies every command may get, and the commands.
//! - **A [`Template`]** is a line with fields, such as `MA {target:steps}`. It renders a line from
//!   values, and matches a line back into values through the anchored regex it compiles. So one
//!   description serves the driver and the emulator, `siq-sim`'s `DialectEmulator`.
//! - **[`Dialect::parse`]** checks the whole file, and reports every problem with its path.
//!
//! **[`AsciiFocusDrive`]** (P1.10b) is a `siq-devices` [`FocusDrive`] over a dialect:
//!
//! - **Commands** are sent one at a time, and each waits for its reply. The driver matches every
//!   line received against the reply's template, the dialect's errors, and its completion reply.
//!   Other lines are skipped.
//! - **Connecting** sends the dialect's `init` lines, then `identify` if the dialect has it, and
//!   reads the status.
//! - **A move ends** when the status's `moving` flag clears (`motion.done = "poll"`). Or it ends
//!   when the device sends `replies.complete` (`"reply"`), with a fresh status then. Either must
//!   happen within twice the move's time at its speed, plus 2 s, on the clock.
//! - **The status** is polled at `motion.poll_hz`, and timestamped at its arrival less half the
//!   round trip.
//! - **`stop`** interrupts a move: its `move_to` fails `Stopped`.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, PoisonError, Weak};
use std::time::{Duration, Instant};

use siq_devices::{
    Approach, BoxFuture, Device, DeviceError, DeviceInfo, DeviceResult, FocusDrive, FocusStatus,
    MotionState, MoveOpts, Travel, watch,
};
use siq_serial::{LineTransport, SerialError};
use siq_types::{Clock, Micrometers, MicrometersPerSecond, Timestamp};
use tokio::sync::Mutex;
use tracing::debug;

mod dialect;
mod template;

pub use dialect::{
    Command, Commands, Dialect, DialectError, Done, Motion, Replies, Terminators, Timeouts, lengths,
};
pub use template::{Kind, Template, Value, Values};

/// How long the driver waits for a completion reply before it lets others use the link.
const STEP: Duration = Duration::from_millis(20);

/// A focus drive that speaks a dialect over `T`.
pub struct AsciiFocusDrive<T: LineTransport> {
    info: DeviceInfo,
    inner: Arc<Inner<T>>,
}

struct Inner<T> {
    dialect: Arc<Dialect>,
    clock: Arc<dyn Clock>,
    link: Mutex<Link<T>>,
    status: watch::Sender<FocusStatus>,
    /// What the driver last set the drive doing, for a status without the device's word on it.
    doing: std::sync::Mutex<MotionState>,
    /// Whether the drive was homed, for a status without a `homed` flag.
    homed: AtomicBool,
    /// How many stops there have been: a move that sees a new one has been stopped.
    stops: AtomicU64,
}

struct Link<T> {
    transport: T,
    /// Whether the completion reply has come since the last move was sent.
    completed: bool,
}

fn gone(error: SerialError) -> DeviceError {
    DeviceError::Disconnected(format!("the focus drive's link: {error}"))
}

impl<T: LineTransport + 'static> AsciiFocusDrive<T> {
    /// Connects to the drive over `transport`, which must use the dialect's framing, and starts
    /// polling it. Its timing is on `clock`, which also timestamps the status.
    pub async fn connect(
        transport: T,
        dialect: Arc<Dialect>,
        clock: Arc<dyn Clock>,
    ) -> DeviceResult<Self> {
        let status = FocusStatus {
            position: Micrometers(0.0),
            state: MotionState::Idle,
            homed: false,
            ts: clock.now(),
        };
        let inner = Arc::new(Inner {
            dialect: dialect.clone(),
            clock,
            link: Mutex::new(Link {
                transport,
                completed: false,
            }),
            status: watch::Sender::new(status),
            doing: std::sync::Mutex::new(MotionState::Idle),
            homed: AtomicBool::new(false),
            stops: AtomicU64::new(0),
        });
        let mut info = DeviceInfo::new(dialect.name.clone(), "focus drive");
        {
            let mut link = inner.link.lock().await;
            for line in &dialect.commands.init {
                inner
                    .exchange(&mut link, line, None, &Values::new())
                    .await?;
            }
            if let Some(identify) = &dialect.commands.identify {
                let reply = identify.reply.as_ref();
                let none = Values::new();
                let values = inner
                    .exchange(&mut link, &identify.send, reply, &none)
                    .await?;
                let text = |name: &str| match values.get(name) {
                    Some(Value::Text(text)) => Some(text.clone()),
                    _ => None,
                };
                info.model = text("model").unwrap_or(info.model);
                (info.firmware, info.serial) = (text("firmware"), text("serial"));
            }
            inner.query(&mut link).await?;
        }
        let period = dialect.motion.poll_hz.period().to_duration();
        let period = period.unwrap_or(Duration::from_millis(100));
        tokio::spawn(poll(Arc::downgrade(&inner), period));
        Ok(Self { info, inner })
    }
}

/// Polls the status until the drive goes away.
async fn poll<T: LineTransport>(inner: Weak<Inner<T>>, period: Duration) {
    loop {
        tokio::time::sleep(period).await;
        let Some(inner) = inner.upgrade() else { return };
        let mut link = inner.link.lock().await;
        match inner.query(&mut link).await {
            Ok(_) => {}
            Err(DeviceError::Disconnected(_)) => {
                let now = inner.clock.now();
                let disconnected = (MotionState::Disconnected, now);
                inner
                    .status
                    .send_modify(|status| (status.state, status.ts) = disconnected);
                return;
            }
            Err(error) => debug!(%error, "a focus status poll failed"),
        }
    }
}

impl<T: LineTransport> Inner<T> {
    fn set_doing(&self, state: MotionState) {
        *self.doing.lock().unwrap_or_else(PoisonError::into_inner) = state;
    }

    /// Sends `send` with `values`, and waits for `reply`, or `replies.ok`.
    async fn exchange(
        &self,
        link: &mut Link<T>,
        send: &Template,
        reply: Option<&Template>,
        values: &Values,
    ) -> DeviceResult<Values> {
        let dialect = &self.dialect;
        let (step, line) = (dialect.step.0, send.render(values, dialect.step.0));
        link.transport.write_line(&line).await.map_err(gone)?;
        let reply = reply.unwrap_or(&dialect.replies.ok);
        let wait = dialect.timeouts.reply.to_duration().unwrap_or_default();
        let deadline = Instant::now() + wait;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let text = match link.transport.read_line(left).await {
                Ok(text) => text,
                Err(SerialError::Timeout) => {
                    let message = format!("{} didn't answer `{line}`", dialect.name);
                    return Err(DeviceError::Timeout(message));
                }
                Err(error) => return Err(gone(error)),
            };
            if let Some(values) = reply.matches(&text, step) {
                return Ok(values);
            }
            if let Some(error) = dialect.error(&text) {
                return Err(error);
            }
            match &dialect.replies.complete {
                Some(complete) if complete.matches(&text, step).is_some() => link.completed = true,
                _ => debug!(text, "skipped a line from the focus drive"),
            }
        }
    }

    /// Asks for the status, and publishes it.
    async fn query(&self, link: &mut Link<T>) -> DeviceResult<FocusStatus> {
        let command = &self.dialect.commands.status;
        let sent = self.clock.now();
        let reply = command.reply.as_ref();
        let values = self
            .exchange(link, &command.send, reply, &Values::new())
            .await?;
        let now = self.clock.now();
        let ts = Timestamp(now.0 - now.saturating_since(sent).as_nanos() as u64 / 2);
        let flag = |name: &str| match values.get(name) {
            Some(Value::Flag(flag)) => Some(*flag),
            _ => None,
        };
        let position = match values.get("position") {
            Some(Value::Length(position)) => Micrometers(*position),
            _ => self.status.borrow().position,
        };
        // The device's word on whether it moves wins; what the driver asked says what kind.
        let mut doing = self.doing.lock().unwrap_or_else(PoisonError::into_inner);
        let state = match (flag("moving"), *doing) {
            (Some(false), _) => MotionState::Idle,
            (Some(true), MotionState::Idle) => MotionState::Moving,
            (_, doing) => doing,
        };
        *doing = state;
        drop(doing);
        let homed = flag("homed").unwrap_or(self.homed.load(Ordering::Relaxed));
        let status = FocusStatus {
            position,
            state,
            homed,
            ts,
        };
        self.status.send_replace(status);
        Ok(status)
    }

    /// Sends `command` with `values`, as what starts a motion of kind `doing`.
    async fn start(
        &self,
        command: &Command,
        values: &Values,
        doing: MotionState,
        stops: u64,
    ) -> DeviceResult<()> {
        let mut link = self.link.lock().await;
        if self.stops.load(Ordering::Acquire) != stops {
            return Err(DeviceError::Stopped);
        }
        link.completed = false;
        let reply = command.reply.as_ref();
        self.exchange(&mut link, &command.send, reply, values)
            .await?;
        self.set_doing(doing);
        Ok(())
    }

    /// Waits, until `deadline` on the clock, for the motion to end, by the completion reply if
    /// `completion`, then for a status that says the drive is still.
    async fn settle(
        &self,
        stops: u64,
        deadline: Timestamp,
        completion: bool,
    ) -> DeviceResult<FocusStatus> {
        let period = self
            .dialect
            .motion
            .poll_hz
            .period()
            .to_duration()
            .unwrap_or(STEP);
        loop {
            if self.stops.load(Ordering::Acquire) != stops {
                return Err(DeviceError::Stopped);
            }
            if self.clock.now() >= deadline {
                let message = format!("{} didn't finish in time", self.dialect.name);
                return Err(DeviceError::Timeout(message));
            }
            let mut link = self.link.lock().await;
            if completion && !link.completed {
                // The completion reply comes between commands, so wait for it with the link held.
                let complete = self.dialect.replies.complete.as_ref();
                match link.transport.read_line(STEP).await {
                    Ok(text) if complete.is_some_and(|c| c.matches(&text, 1.0).is_some()) => {
                        link.completed = true;
                    }
                    Ok(text) => match self.dialect.error(&text) {
                        Some(error) => return Err(error),
                        None => debug!(text, "skipped a line from the focus drive"),
                    },
                    Err(SerialError::Timeout) => {}
                    Err(error) => return Err(gone(error)),
                }
            }
            if !completion || link.completed {
                if completion && !self.dialect.reports("moving") {
                    self.set_doing(MotionState::Idle);
                }
                let status = self.query(&mut link).await?;
                if status.state == MotionState::Idle {
                    return Ok(status);
                }
                drop(link);
                tokio::time::sleep(period).await;
            }
        }
    }

    fn travel(&self) -> Travel {
        let [min, max] = self.dialect.motion.travel;
        Travel { min, max }
    }

    /// Moves to `target` at `speed` µm/s, and waits for it to end.
    async fn reach(&self, target: f64, speed: f64, stops: u64) -> DeviceResult<FocusStatus> {
        let from = self.status.borrow().position.0;
        let values = lengths([("target", target), ("speed", speed)]);
        self.start(
            &self.dialect.commands.move_to,
            &values,
            MotionState::Moving,
            stops,
        )
        .await?;
        let seconds = 2.0 * (target - from).abs() / speed + 2.0;
        let deadline = self.clock.now() + Duration::from_secs_f64(seconds);
        self.settle(stops, deadline, self.dialect.motion.done == Done::Reply)
            .await
    }

    async fn go(&self, target: Micrometers, opts: MoveOpts) -> DeviceResult<FocusStatus> {
        let travel = self.travel();
        if !travel.contains(target) {
            let message = format!("{target} is outside the focus drive's travel");
            return Err(DeviceError::InvalidArgument(message));
        }
        let speed = opts.speed.unwrap_or(self.dialect.motion.speed).0;
        if !(speed.is_finite() && speed > 0.0) {
            return Err(DeviceError::InvalidArgument(format!(
                "the speed {speed} µm/s"
            )));
        }
        let stops = self.stops.load(Ordering::Acquire);
        // A unidirectional move arrives going up: one that would go down first goes past.
        let below = target.0 - self.dialect.motion.overshoot.0;
        let from = self.status.borrow().position;
        if opts.approach == Approach::Unidirectional && target <= from {
            self.reach(below.max(travel.min.0), speed, stops).await?;
        }
        self.reach(target.0, speed, stops).await
    }

    async fn jog_at(&self, velocity: MicrometersPerSecond) -> DeviceResult<()> {
        if velocity.0 == 0.0 {
            return self.halt().await;
        }
        let Some(jog) = &self.dialect.commands.jog else {
            let message = format!("{} has no jog command", self.dialect.name);
            return Err(DeviceError::Unsupported(message));
        };
        let stops = self.stops.load(Ordering::Acquire);
        let values = lengths([("velocity", velocity.0)]);
        self.start(jog, &values, MotionState::Jogging, stops).await
    }

    async fn halt(&self) -> DeviceResult<()> {
        let stops = self.stops.fetch_add(1, Ordering::AcqRel) + 1;
        {
            let mut link = self.link.lock().await;
            let stop = &self.dialect.commands.stop;
            self.exchange(&mut link, &stop.send, stop.reply.as_ref(), &Values::new())
                .await?;
            // Still, unless the status says otherwise.
            self.set_doing(MotionState::Idle);
        }
        // A stop that another overtakes has still stopped the drive.
        let deadline = self.clock.now() + Duration::from_secs(5);
        match self.settle(stops, deadline, false).await {
            Ok(_) | Err(DeviceError::Stopped) => Ok(()),
            Err(error) => Err(error),
        }
    }

    async fn home_drive(&self) -> DeviceResult<FocusStatus> {
        let Some(home) = &self.dialect.commands.home else {
            let message = format!("{} has no home command", self.dialect.name);
            return Err(DeviceError::Unsupported(message));
        };
        let stops = self.stops.load(Ordering::Acquire);
        self.start(home, &Values::new(), MotionState::Homing, stops)
            .await?;
        let wait = self.dialect.timeouts.home.to_duration().unwrap_or_default();
        let done = self.dialect.motion.done == Done::Reply;
        let mut status = self.settle(stops, self.clock.now() + wait, done).await?;
        self.homed.store(true, Ordering::Relaxed);
        if !self.dialect.reports("homed") {
            status.homed = true;
            self.status.send_replace(status);
        }
        Ok(status)
    }
}

impl<T: LineTransport> Device for AsciiFocusDrive<T> {
    fn info(&self) -> &DeviceInfo {
        &self.info
    }
}

impl<T: LineTransport> FocusDrive for AsciiFocusDrive<T> {
    fn move_to(
        &self,
        target: Micrometers,
        opts: MoveOpts,
    ) -> BoxFuture<'_, DeviceResult<FocusStatus>> {
        Box::pin(self.inner.go(target, opts))
    }

    fn move_by(
        &self,
        delta: Micrometers,
        opts: MoveOpts,
    ) -> BoxFuture<'_, DeviceResult<FocusStatus>> {
        let from = self.inner.status.borrow().position;
        Box::pin(self.inner.go(from + delta, opts))
    }

    fn jog(&self, velocity: MicrometersPerSecond) -> BoxFuture<'_, DeviceResult<()>> {
        Box::pin(self.inner.jog_at(velocity))
    }

    fn stop(&self) -> BoxFuture<'_, DeviceResult<()>> {
        Box::pin(self.inner.halt())
    }

    fn home(&self) -> BoxFuture<'_, DeviceResult<FocusStatus>> {
        Box::pin(self.inner.home_drive())
    }

    fn status(&self) -> watch::Receiver<FocusStatus> {
        self.inner.status.subscribe()
    }

    fn limits(&self) -> Travel {
        self.inner.travel()
    }
}
