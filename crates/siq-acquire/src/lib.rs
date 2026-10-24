//! The acquisition engine (REQ-ACQ-004–006, ADR-0031): it owns the camera, the frame pool and the
//! fan-out, and gives every frame its pose and context.
//!
//! - **Modes:** live, free-running; snaps, software-triggered once the stage and focus are still;
//!   and sequences of snaps, with a hook after each frame. A change of region or binning stops the
//!   camera, reconfigures it and starts it again.
//! - **Frame numbers go on across restarts,** so a gap always means frames were lost.
//! - **Every frame is stamped** with the [`AcquisitionContext`] and the pose in force at its
//!   mid-exposure: the context from the last changes, and the pose from the [`PoseTimeline`].
//! - **Some frames wait for their pose:** a frame to be captured, a snap and a sequence's frame are
//!   held until the timeline has a reading after them, for up to [`POSE_WAIT`], so that their pose
//!   is final. A live frame doesn't wait. Frames are published in order.
//! - **[`Acquisition::capture_next`]** hands over the next frames, and marks them for the recorder.

use std::collections::VecDeque;
use std::future::Future;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::{Duration, Instant};

use siq_buffer::{
    Fanout, FillGuard, FrameLease, FramePool, Policy, PoolConfig, PoolError, PoolStats, Subscriber,
    SubscriberStats,
};
use siq_devices::{
    AxisStatus, Camera, CameraSettings, CameraStats, DeviceError, DeviceResult, FocusStatus,
    FrameSink, StageStatus, TriggerMode, watch,
};
use siq_types::{CalibrationId, ChannelId, Clock, FrameMeta, FrameSeq, ObjectiveId, Timestamp};
use tokio::sync::mpsc;
use tracing::warn;

mod timeline;

pub use timeline::{PoseAt, PoseTimeline};

/// How long, on the clock, a frame waits for a reading after it before it goes with the pose as
/// far as it is known: two readings at the GRBL driver's 5 Hz, and a little more.
pub const POSE_WAIT: Duration = Duration::from_millis(500);

/// How long a snap waits, in real time, for the stage and focus to be still.
const SETTLE: Duration = Duration::from_secs(30);

/// How many readings the timeline keeps of each axis group.
const READINGS: usize = 256;

/// How many context changes the engine keeps, for frames that arrive after a change.
const CONTEXTS: usize = 8;

/// What the frames are taken with, as the UI and the workflows set it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AcquisitionContext {
    /// The channel.
    pub channel: Option<ChannelId>,
    /// The objective.
    pub objective: Option<ObjectiveId>,
    /// The stage–camera calibration in force.
    pub calibration: Option<CalibrationId>,
}

/// What the camera is doing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Mode {
    /// Stopped.
    #[default]
    Stopped,
    /// Free-running, for live view.
    Live,
    /// Waiting for software triggers, for snaps and sequences.
    Triggered,
}

/// What the engine has done, and how its consumers keep up.
#[derive(Debug, Clone, PartialEq)]
pub struct AcquisitionStats {
    /// What the camera is doing.
    pub mode: Mode,
    /// The rate of the last frames, by their mid-exposure times.
    pub fps: f64,
    /// Frames published to the fan-out.
    pub published: u64,
    /// Frames the camera lost for want of a free slot, since the engine was made.
    pub lost_no_slot: u64,
    /// Frames the camera or grabber lost, since the engine was made.
    pub lost_device: u64,
    /// Frames waiting for their pose now.
    pub waiting: usize,
    /// Each consumer's name and statistics: how much it has dropped, and how far behind it has been.
    pub consumers: Vec<(String, SubscriberStats)>,
}

/// A frame waiting to be published.
struct Waiting {
    slot: FillGuard,
    meta: FrameMeta,
    deadline: Timestamp,
    to: Option<mpsc::Sender<FrameLease>>,
}

/// The frames that [`Acquisition::capture_next`] still has to mark.
struct Capture {
    remaining: u32,
    save: bool,
    frames: mpsc::Sender<FrameLease>,
}

/// The frame path's state, locked briefly on the camera's thread.
struct State {
    mode: Mode,
    /// The last context changes, each from when it was set, oldest first.
    contexts: VecDeque<(Timestamp, AcquisitionContext)>,
    /// The number of the camera's frame 0 since it last started.
    base: u64,
    /// The number the next frame will have.
    next: u64,
    waiting: VecDeque<Waiting>,
    capture: Option<Capture>,
    /// The mid-exposure times of the last frames published.
    recent: VecDeque<Timestamp>,
    published: u64,
}

impl State {
    /// The context in force at `t`: the last set at or before it, or the oldest kept.
    fn context_at(&self, t: Timestamp) -> AcquisitionContext {
        let set = self.contexts.iter().rev().find(|(since, _)| *since <= t);
        set.or(self.contexts.front())
            .map(|(_, context)| *context)
            .unwrap_or_default()
    }
}

struct Inner {
    camera: Arc<dyn Camera>,
    pool: FramePool,
    fanout: Fanout,
    clock: Arc<dyn Clock>,
    timeline: PoseTimeline,
    /// Starts, stops, snaps and reconfigurations take turns.
    turn: tokio::sync::Mutex<()>,
    state: Mutex<State>,
    baseline: CameraStats,
}

/// The camera's sink. It holds the engine weakly, so that dropping the engine ends it.
struct Sink(Weak<Inner>);

impl FrameSink for Sink {
    fn frame(&self, slot: FillGuard, meta: FrameMeta) {
        if let Some(inner) = self.0.upgrade() {
            inner.arrive(slot, meta);
        }
    }
}

/// The acquisition engine.
pub struct Acquisition {
    inner: Arc<Inner>,
}

impl Acquisition {
    /// An engine for `camera`, with a pool made from `pool`, whose slots must fit the largest frame
    /// it will take. Its timing is on `clock`, the camera's and the drivers' clock.
    pub fn new(
        camera: Arc<dyn Camera>,
        pool: &PoolConfig,
        clock: Arc<dyn Clock>,
    ) -> Result<Self, PoolError> {
        let pool = FramePool::new(pool, clock.clone(), None)?;
        Ok(Self::with_pool(camera, pool, clock))
    }

    /// An engine for `camera` with `pool`, made by the caller: with its metrics, say.
    pub fn with_pool(camera: Arc<dyn Camera>, pool: FramePool, clock: Arc<dyn Clock>) -> Self {
        let slots = pool.slots();
        let state = State {
            mode: Mode::Stopped,
            contexts: VecDeque::from([(Timestamp(0), AcquisitionContext::default())]),
            base: 0,
            next: 0,
            waiting: VecDeque::with_capacity(slots),
            capture: None,
            recent: VecDeque::with_capacity(16),
            published: 0,
        };
        let inner = Inner {
            baseline: camera.stats(),
            camera,
            fanout: Fanout::new(&pool),
            pool,
            clock,
            timeline: PoseTimeline::new(READINGS),
            turn: tokio::sync::Mutex::new(()),
            state: Mutex::new(state),
        };
        Self {
            inner: Arc::new(inner),
        }
    }

    /// The pool's slots: free, being filled, and leased.
    pub fn pool_stats(&self) -> PoolStats {
        self.inner.pool.stats()
    }

    /// Follows the stage's status, for the frames' poses.
    pub fn follow_stage(&self, status: watch::Receiver<StageStatus>) {
        self.follow(status, PoseTimeline::push_stage);
    }

    /// Follows the focus drive's status, for the frames' poses.
    pub fn follow_focus(&self, status: watch::Receiver<FocusStatus>) {
        self.follow(status, PoseTimeline::push_focus);
    }

    /// Feeds each status into the timeline, and lets waiting frames go as their poses are decided
    /// or their time is up.
    fn follow<P: Send + Sync + 'static>(
        &self,
        mut status: watch::Receiver<AxisStatus<P>>,
        push: fn(&PoseTimeline, &AxisStatus<P>),
    ) {
        push(&self.inner.timeline, &status.borrow_and_update());
        let inner = Arc::downgrade(&self.inner);
        tokio::spawn(async move {
            loop {
                let changed =
                    tokio::time::timeout(Duration::from_millis(50), status.changed()).await;
                let Some(inner) = inner.upgrade() else { return };
                match changed {
                    Ok(Ok(())) => push(&inner.timeline, &status.borrow_and_update()),
                    Ok(Err(_)) => return,
                    Err(_) => {}
                }
                inner.flush();
            }
        });
    }

    /// A consumer of every published frame its filter takes, kept as `policy` says.
    pub fn subscribe(
        &self,
        name: impl Into<String>,
        policy: Policy,
        filter: impl Fn(&FrameMeta) -> bool + Send + Sync + 'static,
    ) -> Subscriber {
        self.inner.fanout.subscribe(name, policy, filter)
    }

    /// Starts live view: the camera free-running.
    pub async fn start_live(&self) -> DeviceResult<()> {
        let _turn = self.inner.turn.lock().await;
        self.inner.run_in(TriggerMode::FreeRun).await
    }

    /// Stops the camera.
    pub async fn stop(&self) -> DeviceResult<()> {
        let _turn = self.inner.turn.lock().await;
        self.inner.halt().await
    }

    /// Takes one frame once the stage and focus are still, with the camera software-triggered, and
    /// gives it back with its pose decided. It is published too, marked for saving if `save`.
    pub async fn snap(&self, save: bool) -> DeviceResult<FrameLease> {
        let _turn = self.inner.turn.lock().await;
        self.inner.snap(save).await
    }

    /// Takes `n` snaps, and calls `hook` with each frame before the next is taken: to move the
    /// focus for a z-stack, say.
    pub async fn sequence<F, H>(&self, n: u32, save: bool, mut hook: H) -> DeviceResult<()>
    where
        H: FnMut(u32, FrameLease) -> F,
        F: Future<Output = DeviceResult<()>>,
    {
        let _turn = self.inner.turn.lock().await;
        for index in 0..n {
            let frame = self.inner.snap(save).await?;
            hook(index, frame).await?;
        }
        Ok(())
    }

    /// Applies `settings`, keeping the engine's trigger mode. The exposure and gain change at
    /// once; any other change stops the camera, reconfigures it, and starts it again.
    pub async fn configure(&self, settings: CameraSettings) -> DeviceResult<()> {
        let _turn = self.inner.turn.lock().await;
        let inner = &self.inner;
        let settings = CameraSettings {
            trigger: inner.camera.settings().trigger,
            ..settings
        };
        match inner.camera.configure(settings).await {
            Err(DeviceError::Busy(_)) => {
                let trigger = settings.trigger;
                inner.halt().await?;
                inner.camera.configure(settings).await?;
                inner.run_in(trigger).await
            }
            other => other,
        }
    }

    /// Sets what frames exposed from now on are taken with.
    pub fn set_context(&self, context: AcquisitionContext) {
        let now = self.inner.clock.now();
        let mut state = self.inner.state();
        if state.contexts.len() == CONTEXTS {
            state.contexts.pop_front();
        }
        state.contexts.push_back((now, context));
    }

    /// What frames are taken with now.
    pub fn context(&self) -> AcquisitionContext {
        self.inner.state().context_at(Timestamp(u64::MAX))
    }

    /// Hands over the next `n` frames to arrive, once published, and marks them for the recorder
    /// if `save`. It replaces a capture still under way. The frames hold their slots until they
    /// are dropped.
    pub fn capture_next(&self, n: u32, save: bool) -> mpsc::Receiver<FrameLease> {
        self.inner.capture(n, save)
    }

    /// The pose timeline.
    pub fn timeline(&self) -> &PoseTimeline {
        &self.inner.timeline
    }

    /// What the engine has done, and how its consumers keep up.
    pub fn stats(&self) -> AcquisitionStats {
        let inner = &self.inner;
        let (camera, baseline) = (inner.camera.stats(), inner.baseline);
        let state = inner.state();
        let fps = match (state.recent.front(), state.recent.back()) {
            (Some(first), Some(last)) if last > first => {
                (state.recent.len() - 1) as f64 / last.saturating_since(*first).as_secs_f64()
            }
            _ => 0.0,
        };
        AcquisitionStats {
            mode: state.mode,
            fps,
            published: state.published,
            lost_no_slot: camera.lost_no_slot - baseline.lost_no_slot,
            lost_device: camera.lost_device - baseline.lost_device,
            waiting: state.waiting.len(),
            consumers: inner.fanout.stats(),
        }
    }
}

impl Inner {
    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Numbers a frame, stamps its context, and queues it.
    fn arrive(&self, slot: FillGuard, mut meta: FrameMeta) {
        let mut state = self.state();
        meta.seq = FrameSeq(state.base + meta.seq.0);
        state.next = meta.seq.0 + 1;
        let context = state.context_at(meta.host_ts_mid_exposure);
        (meta.channel, meta.objective, meta.calibration) =
            (context.channel, context.objective, context.calibration);
        let mut to = None;
        if let Some(capture) = state.capture.as_mut() {
            capture.remaining -= 1;
            meta.flags.save = capture.save;
            to = Some(capture.frames.clone());
            if capture.remaining == 0 {
                state.capture = None;
            }
        }
        let waits = to.is_some() || state.mode == Mode::Triggered;
        let deadline = match waits {
            true => self.clock.now() + POSE_WAIT,
            false => Timestamp(0),
        };
        state.waiting.push_back(Waiting {
            slot,
            meta,
            deadline,
            to,
        });
        drop(state);
        self.flush();
    }

    /// Stamps and publishes the waiting frames, in order, while the first's pose is decided or
    /// its time is up.
    fn flush(&self) {
        let mut state = self.state();
        while let Some(first) = state.waiting.front() {
            let t = first.meta.host_ts_mid_exposure;
            if self.clock.now() < first.deadline && !self.timeline.decided(t) {
                return;
            }
            let Some(Waiting {
                slot, mut meta, to, ..
            }) = state.waiting.pop_front()
            else {
                return;
            };
            let pose = self.timeline.pose_at(t);
            (meta.pose, meta.pose_quality, meta.flags.moving) =
                (pose.pose, pose.quality, pose.moving);
            // Published under the lock, so that frames go out in order.
            match slot.publish(meta) {
                Ok(lease) => {
                    self.fanout.publish(&lease);
                    if state.recent.len() == state.recent.capacity() {
                        state.recent.pop_front();
                    }
                    state.recent.push_back(t);
                    state.published += 1;
                    if let Some(to) = to {
                        let _ = to.try_send(lease);
                    }
                }
                Err(error) => warn!(%error, "a frame didn't fit its slot"),
            }
        }
    }

    fn capture(&self, n: u32, save: bool) -> mpsc::Receiver<FrameLease> {
        let (frames, receiver) = mpsc::channel(n.max(1) as usize);
        let capture = (n > 0).then_some(Capture {
            remaining: n,
            save,
            frames,
        });
        self.state().capture = capture;
        receiver
    }

    /// Runs the camera with `trigger`, restarting it if it runs otherwise.
    async fn run_in(self: &Arc<Self>, trigger: TriggerMode) -> DeviceResult<()> {
        let mode = match trigger {
            TriggerMode::FreeRun => Mode::Live,
            _ => Mode::Triggered,
        };
        if self.state().mode == mode {
            return Ok(());
        }
        self.halt().await?;
        let settings = CameraSettings {
            trigger,
            ..self.camera.settings()
        };
        self.camera.configure(settings).await?;
        {
            let mut state = self.state();
            (state.base, state.mode) = (state.next, mode);
        }
        let sink = Arc::new(Sink(Arc::downgrade(self)));
        let started = self.camera.start(self.pool.clone(), sink).await;
        if started.is_err() {
            self.state().mode = Mode::Stopped;
        }
        started
    }

    async fn halt(&self) -> DeviceResult<()> {
        if self.state().mode == Mode::Stopped {
            return Ok(());
        }
        self.camera.stop().await?;
        self.state().mode = Mode::Stopped;
        Ok(())
    }

    async fn snap(self: &Arc<Self>, save: bool) -> DeviceResult<FrameLease> {
        self.run_in(TriggerMode::Software).await?;
        let started = Instant::now();
        while !self.timeline.still() {
            if started.elapsed() > SETTLE {
                return Err(DeviceError::Timeout(
                    "the stage and focus didn't come to rest".into(),
                ));
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let mut frames = self.capture(1, save);
        let frame = async {
            self.camera.trigger().await?;
            frames.recv().await.ok_or(DeviceError::Stopped)
        };
        // The exposure, its readout, and the wait for the pose, with room to spare.
        let exposure = self
            .camera
            .settings()
            .exposure
            .to_duration()
            .unwrap_or_default();
        let patience = exposure + POSE_WAIT + Duration::from_secs(5);
        let result = tokio::time::timeout(patience, frame).await;
        if !matches!(result, Ok(Ok(_))) {
            self.state().capture = None;
        }
        result.map_err(|_| DeviceError::Timeout("the snap's frame didn't come".into()))?
    }
}
