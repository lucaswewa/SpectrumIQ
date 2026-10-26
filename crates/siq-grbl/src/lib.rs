//! A GRBL and grblHAL stage driver (REQ-STG-001, ADR-0029): a `siq-devices` [`Stage`] over a
//! serial [`LineTransport`].
//!
//! - **Connecting:** a soft reset, the banner, and `$$` for the axes' rates, accelerations and
//!   travel. Then `$X` if the controller is locked and the options allow it, and `G21 G90`.
//! - **Moves complete with `G4 P0`:** its `ok` comes once the motion has finished. A fresh `Idle`
//!   status then confirms it. The `ok` must come within a timeout from the distance and speed.
//! - **A backlash-consistent approach** overshoots, so that each axis arrives from its configured
//!   side (REQ-STG-005).
//! - **`stop`** holds, then resets once still, so that GRBL keeps the position, and re-syncs. A
//!   reset in motion would lose the position: the status would then say the stage isn't homed.
//! - **Status** is polled on the clock, and published on a watch (REQ-STG-004).
//! - **Positions** are µm in the stage frame: machine millimetres × 1000.
//!
//! The stage is open-loop: its positions are where the controller has stepped it, not measured.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use siq_config::model::{GrblMotion, GrblStageConfig, SimStageConfig};
use siq_devices::{
    Approach, BoxFuture, Device, DeviceError, DeviceInfo, DeviceResult, Kinematics, MotionState,
    MoveOpts, Stage, StageStatus, Travel, Xyz, watch,
};
use siq_serial::{LineTransport, SerialError};
use siq_types::{Clock, Hertz, MicrometersPerSecond};
use tokio::sync::{mpsc, oneshot};

mod link;
pub mod protocol;

use link::{Line, Shared, Urgent};

/// How the driver works the stage.
#[derive(Debug, Clone, PartialEq)]
pub struct GrblOptions {
    /// How often it asks for the status.
    pub poll: Hertz,
    /// How it moves: unlocking, and the backlash-consistent approach.
    pub motion: GrblMotion,
}

impl From<&GrblStageConfig> for GrblOptions {
    fn from(config: &GrblStageConfig) -> Self {
        Self {
            poll: config.poll_hz,
            motion: config.motion.clone(),
        }
    }
}

impl From<&SimStageConfig> for GrblOptions {
    fn from(config: &SimStageConfig) -> Self {
        Self {
            poll: config.poll_hz,
            motion: config.motion.clone(),
        }
    }
}

/// The axes as the controller's `$$` describes them.
#[derive(Debug, Clone, Copy)]
struct Axes {
    /// The maximum rates, `$11x`, in mm/min.
    rate: [f64; 3],
    /// The accelerations, `$12x`, in mm/s².
    accel: [f64; 3],
    /// The travel, `$13x`, in mm: from minus it to 0.
    travel: [f64; 3],
    /// The homing seek rate, `$25`, in mm/min.
    seek: f64,
}

/// Where the driver last sent the stage, if it knows, and the direction each axis last moved in:
/// 1, −1, or 0 for unknown.
#[derive(Debug, Clone, Copy, Default)]
struct Commanded {
    at: Option<[f64; 3]>,
    heading: [f64; 3],
}

/// How long the controller has to answer a command, or to send a status, in real time.
const PATIENCE: Duration = Duration::from_secs(5);

/// How long the port must be quiet after the banner before the handshake goes on.
const QUIET: Duration = Duration::from_millis(250);

/// A GRBL or grblHAL stage.
pub struct GrblStage {
    info: DeviceInfo,
    options: GrblOptions,
    axes: Axes,
    clock: Arc<dyn Clock>,
    shared: Arc<Shared>,
    lines: mpsc::UnboundedSender<Line>,
    urgent: mpsc::UnboundedSender<Urgent>,
    commanded: Mutex<Commanded>,
}

/// Sends `line` during the handshake, and gives back the lines before its `ok`.
async fn command<T: LineTransport>(transport: &mut T, line: &str) -> DeviceResult<Vec<String>> {
    transport.write_line(line).await.map_err(link_failed)?;
    let mut before = Vec::new();
    loop {
        let reply = transport.read_line(PATIENCE).await.map_err(link_failed)?;
        match reply.strip_prefix("error:") {
            _ if reply == "ok" => return Ok(before),
            Some(code) => return Err(protocol::error(code.parse().unwrap_or(0))),
            None => before.push(reply),
        }
    }
}

fn link_failed(error: SerialError) -> DeviceError {
    match error {
        SerialError::Timeout => DeviceError::Timeout("the stage's controller didn't answer".into()),
        other => DeviceError::Disconnected(format!("the stage's link: {other}")),
    }
}

fn gone() -> DeviceError {
    DeviceError::Disconnected("the stage's link task has stopped".into())
}

/// Millimetres from the stage frame's µm.
fn mm(position: Xyz) -> [f64; 3] {
    [position.x.0, position.y.0, position.z.0].map(|um| um / 1000.0)
}

impl GrblStage {
    /// Connects to the controller over `transport`, and starts polling it. Its timing is on
    /// `clock`, which also timestamps the status.
    pub async fn connect<T: LineTransport + 'static>(
        mut transport: T,
        options: GrblOptions,
        clock: Arc<dyn Clock>,
    ) -> DeviceResult<Self> {
        // A reset puts the controller in a known state. Its banner comes once the reset is done,
        // after whatever the port showed on opening; a board that restarts on opening is slower.
        transport.write_realtime(0x18).await.map_err(link_failed)?;
        let mut banner = None;
        let banner = loop {
            let wait = if banner.is_some() { QUIET } else { PATIENCE };
            match transport.read_line(wait).await {
                Ok(line) if line.starts_with("Grbl") => banner = Some(line),
                Ok(_) => {}
                Err(SerialError::Timeout) if banner.is_some() => break banner.unwrap_or_default(),
                Err(error) => return Err(link_failed(error)),
            }
        };
        let mut settings = HashMap::new();
        for line in command(&mut transport, "$$").await? {
            settings.extend(protocol::parse_setting(&line));
        }
        let setting = |base: u16| -> DeviceResult<[f64; 3]> {
            let missing = || DeviceError::Fault(format!("the controller's $$ has no ${base}"));
            let value = |axis: u16| settings.get(&(base + axis)).copied().ok_or_else(missing);
            Ok([value(0)?, value(1)?, value(2)?])
        };
        let axes = Axes {
            rate: setting(110)?,
            accel: setting(120)?,
            travel: setting(130)?,
            seek: settings.get(&25).copied().unwrap_or(500.0),
        };
        // `G21` sets the units of moves, but `$13` those of the status.
        if settings.get(&13) == Some(&1.0) {
            let message = "the controller reports in inches: set $13=0";
            return Err(DeviceError::InvalidArgument(message.into()));
        }

        let shared = Arc::new(Shared {
            status: watch::Sender::new(StageStatus {
                position: Xyz::default(),
                state: MotionState::Idle,
                homed: false,
                ts: clock.now(),
            }),
            homed: AtomicBool::new(false),
            alarm: AtomicU32::new(0),
            resets: AtomicU64::new(0),
        });
        let (lines, line_queue) = mpsc::unbounded_channel();
        let (urgent, urgent_queue) = mpsc::unbounded_channel();
        let poll = Duration::from_secs_f64(1.0 / options.poll.0.max(0.1));
        let link = link::run(
            transport,
            shared.clone(),
            clock.clone(),
            poll,
            line_queue,
            urgent_queue,
        );
        tokio::spawn(link);
        let vendor = if banner.contains("GrblHAL") {
            "grblHAL"
        } else {
            "GRBL"
        };
        let info = DeviceInfo {
            firmware: Some(banner),
            ..DeviceInfo::new(vendor, "stage controller")
        };
        let stage = Self {
            info,
            options,
            axes,
            clock,
            shared,
            lines,
            urgent,
            commanded: Mutex::default(),
        };
        stage.resync().await?;
        Ok(stage)
    }

    fn commanded(&self) -> MutexGuard<'_, Commanded> {
        self.commanded
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// The resets sent so far: lines queued under an older count aren't sent.
    fn epoch(&self) -> u64 {
        self.shared.resets.load(Ordering::Acquire)
    }

    /// Sends `text`, and waits for its `ok` for `within` on the clock.
    async fn line(&self, text: String, epoch: u64, within: Duration) -> DeviceResult<()> {
        let (reply, answer) = oneshot::channel();
        let deadline = self.clock.now() + within;
        let line = Line {
            text,
            epoch,
            deadline,
            reply,
        };
        self.lines.send(line).map_err(|_| gone())?;
        answer.await.map_err(|_| gone())?
    }

    fn send_urgent(&self, urgent: Urgent) -> DeviceResult<()> {
        self.urgent.send(urgent).map_err(|_| gone())
    }

    /// A status report asked for now: the first to arrive from now on, timestamped from now on.
    async fn fresh_status(&self) -> DeviceResult<StageStatus> {
        let since = self.clock.now();
        let mut status = self.shared.status.subscribe();
        self.send_urgent(Urgent::Poll)?;
        let fresh = async {
            loop {
                status.changed().await.map_err(|_| gone())?;
                let report = *status.borrow_and_update();
                if report.ts >= since {
                    return Ok(report);
                }
            }
        };
        let missing = || DeviceError::Timeout("the stage's controller sent no status".into());
        tokio::time::timeout(PATIENCE, fresh)
            .await
            .map_err(|_| missing())?
    }

    /// Polls until the stage is in a state `done` accepts, and fails on an alarm.
    async fn until(&self, done: fn(MotionState) -> bool) -> DeviceResult<StageStatus> {
        loop {
            let status = self.fresh_status().await?;
            match status.state {
                state if done(state) => return Ok(status),
                MotionState::Alarm(code) => return Err(DeviceError::Alarm(code)),
                _ => tokio::time::sleep(Duration::from_millis(20)).await,
            }
        }
    }

    async fn idle(&self) -> DeviceResult<StageStatus> {
        self.until(|state| state == MotionState::Idle).await
    }

    /// Where the stage was last sent, or, when a jog or a stop has left that unknown, where it
    /// reports.
    async fn position(&self) -> DeviceResult<[f64; 3]> {
        let at = self.commanded().at;
        match at {
            Some(at) => Ok(at),
            None => Ok(mm(self.fresh_status().await?.position)),
        }
    }

    /// Whether `point` (mm) is within the travel.
    fn within(&self, point: [f64; 3]) -> bool {
        (0..3).all(|i| (-self.axes.travel[i] - 1e-9..=1e-9).contains(&point[i]))
    }

    /// How long moves along `path` (mm) at `speed` (mm/s) may take: twice their time at the
    /// slowest axis's rate and acceleration, and 2 s more.
    fn allowance(&self, path: &[[f64; 3]], speed: f64) -> Duration {
        let slowest = |values: [f64; 3]| values.into_iter().fold(f64::INFINITY, f64::min);
        let accel = slowest(self.axes.accel).max(1e-3);
        let speed = speed.min(slowest(self.axes.rate) / 60.0).max(1e-3);
        let length =
            |a: &[f64; 3], b: &[f64; 3]| (0..3).map(|i| (b[i] - a[i]).powi(2)).sum::<f64>().sqrt();
        let length: f64 = path.windows(2).map(|leg| length(&leg[0], &leg[1])).sum();
        let ramps = 2.0 * speed / accel * (path.len() - 1) as f64;
        Duration::from_secs_f64(2.0 * (length / speed + ramps) + 2.0)
    }

    async fn go(&self, target: Xyz, opts: MoveOpts) -> DeviceResult<StageStatus> {
        let epoch = self.epoch();
        let to = mm(target);
        if !self.within(to) {
            let message = format!("{target:?} is outside the stage's travel");
            return Err(DeviceError::InvalidArgument(message));
        }
        let from = self.position().await?;
        let heading = self.commanded().heading;
        let mut path = vec![from];
        if opts.approach == Approach::Unidirectional {
            // An axis that would arrive the wrong way, or may have, first goes past the target.
            let overshoot = self.options.motion.overshoot.0 / 1000.0;
            let side = self
                .options
                .motion
                .approach
                .map(|d| if d < 0 { -1.0 } else { 1.0 });
            let arrives = |i: usize| match to[i] == from[i] {
                true => heading[i],
                false => (to[i] - from[i]).signum(),
            };
            if (0..3).any(|i| arrives(i) != side[i]) {
                let travel = self.axes.travel;
                path.push(std::array::from_fn(|i| match arrives(i) == side[i] {
                    true => to[i],
                    false => (to[i] - side[i] * overshoot).clamp(-travel[i], 0.0),
                }));
            }
        }
        path.push(to);
        let speed = opts.speed.map(|speed| speed.0 / 1000.0);
        let within = self.allowance(&path, speed.unwrap_or(f64::INFINITY));
        for leg in path.windows(2) {
            let [x, y, z] = leg[1];
            let text = match speed {
                Some(speed) => format!("G1 X{x:.4} Y{y:.4} Z{z:.4} F{:.1}", speed * 60.0),
                None => format!("G0 X{x:.4} Y{y:.4} Z{z:.4}"),
            };
            self.line(text, epoch, PATIENCE).await?;
            let mut commanded = self.commanded();
            commanded.at = Some(leg[1]);
            for i in (0..3).filter(|&i| leg[1][i] != leg[0][i]) {
                commanded.heading[i] = (leg[1][i] - leg[0][i]).signum();
            }
        }
        // G4 P0 is answered once the motion before it has finished.
        self.line("G4 P0".into(), epoch, within).await?;
        let status = self.idle().await?;
        match self.epoch() == epoch {
            true => Ok(status),
            false => Err(DeviceError::Stopped),
        }
    }

    async fn halt(&self) -> DeviceResult<()> {
        // A feed hold stops the axes under control, and cancels a jog. A reset once they are
        // still keeps the position, and clears what was planned.
        self.send_urgent(Urgent::Byte(b'!'))?;
        let still = |state| matches!(state, MotionState::Holding | MotionState::Idle);
        match self.until(still).await {
            Ok(_) | Err(DeviceError::Alarm(_)) => {}
            Err(error) => return Err(error),
        }
        let (reply, reset) = oneshot::channel();
        self.send_urgent(Urgent::Reset(reply))?;
        let banner = tokio::time::timeout(PATIENCE, reset).await;
        let missing = || DeviceError::Timeout("the stage's controller didn't restart".into());
        banner.map_err(|_| missing())?.map_err(|_| gone())?;
        self.resync().await.map(|_| ())
    }

    /// After a reset or homing: unlocks if it may, sets the modes again, and takes the position
    /// as the controller reports it.
    async fn resync(&self) -> DeviceResult<StageStatus> {
        let epoch = self.epoch();
        let locked = matches!(self.fresh_status().await?.state, MotionState::Alarm(_));
        if locked && self.options.motion.unlock {
            self.line("$X".into(), epoch, PATIENCE).await?;
            self.shared.alarm.store(0, Ordering::Relaxed);
        }
        if !locked || self.options.motion.unlock {
            self.line("G21 G90".into(), epoch, PATIENCE).await?;
        }
        let status = self.fresh_status().await?;
        let at = Some(mm(status.position));
        *self.commanded() = Commanded {
            at,
            heading: [0.0; 3],
        };
        Ok(status)
    }

    async fn jog_at(&self, velocity: Xyz<MicrometersPerSecond>) -> DeviceResult<()> {
        let v = [velocity.x.0, velocity.y.0, velocity.z.0].map(|v| v / 1000.0);
        let speed = v.iter().map(|v| v * v).sum::<f64>().sqrt();
        // A jog cancel stops a jog already running; a new one starts once it has.
        self.send_urgent(Urgent::Byte(0x85))?;
        self.commanded().at = None;
        if speed == 0.0 {
            return Ok(());
        }
        let epoch = self.epoch();
        let from = mm(self.idle().await?.position);
        // A jog runs until it is cancelled, or to the edge of the travel in its direction.
        let travel = self.axes.travel;
        let room = |i: usize| match v[i] > 0.0 {
            true => -from[i],
            false => from[i] + travel[i],
        };
        let reach = (0..3)
            .filter(|&i| v[i] != 0.0)
            .map(|i| room(i) / (v[i].abs() / speed))
            .fold(f64::INFINITY, f64::min)
            .max(0.0);
        let [x, y, z]: [f64; 3] = std::array::from_fn(|i| from[i] + v[i] / speed * reach);
        let feed = speed * 60.0;
        let text = format!("$J=G90 X{x:.4} Y{y:.4} Z{z:.4} F{feed:.1}");
        self.line(text, epoch, PATIENCE).await?;
        let mut commanded = self.commanded();
        for i in (0..3).filter(|&i| v[i] != 0.0) {
            commanded.heading[i] = v[i].signum();
        }
        Ok(())
    }

    async fn home_axes(&self) -> DeviceResult<StageStatus> {
        let epoch = self.epoch();
        // Homing seeks the switches over the whole travel, then finds them again slowly.
        let travel: f64 = self.axes.travel.iter().sum();
        let seek = self.axes.seek.max(1.0) / 60.0;
        let within = Duration::from_secs_f64(2.0 * travel / seek + 30.0);
        self.line("$H".into(), epoch, within).await?;
        self.shared.homed.store(true, Ordering::Relaxed);
        self.shared.alarm.store(0, Ordering::Relaxed);
        self.resync().await
    }
}

impl Device for GrblStage {
    fn info(&self) -> &DeviceInfo {
        &self.info
    }
}

impl Stage for GrblStage {
    fn move_to(&self, target: Xyz, opts: MoveOpts) -> BoxFuture<'_, DeviceResult<StageStatus>> {
        Box::pin(self.go(target, opts))
    }

    fn move_by(&self, delta: Xyz, opts: MoveOpts) -> BoxFuture<'_, DeviceResult<StageStatus>> {
        Box::pin(async move {
            let [x, y, z] = self.position().await?.map(|mm| mm * 1000.0);
            self.go(Xyz::from_um(x, y, z) + delta, opts).await
        })
    }

    fn jog(&self, velocity: Xyz<MicrometersPerSecond>) -> BoxFuture<'_, DeviceResult<()>> {
        Box::pin(self.jog_at(velocity))
    }

    fn stop(&self) -> BoxFuture<'_, DeviceResult<()>> {
        Box::pin(self.halt())
    }

    fn home(&self) -> BoxFuture<'_, DeviceResult<StageStatus>> {
        Box::pin(self.home_axes())
    }

    fn status(&self) -> watch::Receiver<StageStatus> {
        self.shared.status.subscribe()
    }

    fn limits(&self) -> Travel<Xyz> {
        let [x, y, z] = self.axes.travel.map(|mm| -mm * 1000.0);
        Travel {
            min: Xyz::from_um(x, y, z),
            max: Xyz::from_um(0.0, 0.0, 0.0),
        }
    }

    /// The slower of x's and y's `$11x` and `$12x`.
    fn kinematics(&self) -> Option<Kinematics> {
        let [rate, accel] = [self.axes.rate, self.axes.accel].map(|axes| axes[0].min(axes[1]));
        Some(Kinematics {
            speed_um_s: rate / 60.0 * 1000.0,
            acceleration_um_s2: accel * 1000.0,
        })
    }
}
