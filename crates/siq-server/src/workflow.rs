//! The `workflow` Thing (ADR-0056): acquisition plans run by `siq-workflow`'s engine (ADR-0052,
//! ADR-0055) for the UI and for scripts, and the saved positions.
//!
//! - **Runs:** `run` takes any plan; `tile_scan`, `z_stack`, `multi_position` and `time_lapse`
//!   each make a plan of their shape. A plan that couldn't run on any instrument is refused with
//!   422, as its input doesn't validate; one that can't run on this one, for want of a channel or
//!   a calibration, fails its invocation before anything moves.
//! - **One at a time:** a run holds the global lock throughout, so a manual move during one is
//!   refused as `GlobalLockBusy`. `pause` and `resume` don't take it.
//! - **Cancelling** the invocation (`DELETE /action_invocations/{id}`) cancels the run: a move
//!   under way is abandoned, a capture finishes, then the stage and focus halt. Its dataset keeps
//!   what was taken.
//! - **What's observed:** `state` and `progress`, each frame as `tile_acquired`, and each run's
//!   end as `finished`.
//! - **Before a tile scan:** `field`, the frame about where the stage is, and
//!   `estimate_tile_scan`, its tiles, time and bytes, and what may go wrong (ADR-0057).
//! - **Saved positions,** in `positions.json` in the settings folder: `positions`, and
//!   `save_position`, `rename_position` and `remove_position`. A multi-position given none visits
//!   them.
//! - **When a run's dataset is complete,** however it ended, the instrument's
//!   [`DatasetHook`](crate::DatasetHook)s are told.
//!
//! A plan's steps nest, which a Thing Description's schemas can't: the actions' schemas show
//! `steps` as objects, and `plan_schema` is the plan's whole JSON Schema.

use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use anyhow::Context;
use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use siq_storage::{Snapshot, free_space};
use siq_types::{ChannelId, DatasetId};
use siq_workflow::autofocus::SurfaceKind;
use siq_workflow::engine::{
    DatasetTarget, ErrorPolicy, Outcome, Overrun, Plan, Position, Progress, RunControl, RunEvent,
    Step, TileFocus, ZPlanes, run,
};
use siq_workflow::scan::{self, Focusing, Order, Region, Tile, Timing};
use teta_wot::prelude::{ActionCtx, ActionError, Event, Prop, Thing, ThingCtx, thing_impl};

use crate::Instrument;
use crate::things::{Bound, bind, bound};

/// What the workflow is doing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RunState {
    /// No run is under way.
    #[default]
    Idle,
    /// A run is under way.
    Running,
    /// A run is paused before its next step.
    Paused,
}

/// A frame taken, at its indices.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Acquired {
    /// The time point.
    pub t: u32,
    /// The channel.
    pub c: u16,
    /// The focus plane.
    pub z: u32,
    /// The stage position.
    pub p: u32,
}

/// How a run ended.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Finished {
    /// The plan's name.
    pub name: String,
    /// Its dataset.
    pub dataset: DatasetId,
    /// How it ended.
    pub outcome: Outcome,
    /// Why it failed.
    pub error: Option<String>,
    /// The steps done, of the plan's.
    pub steps: [u64; 2],
    /// The frames written.
    pub frames: u64,
}

/// Steps of a plan, as `siq-workflow`'s `Step`: see `plan_schema`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Steps(pub Vec<Step>);

impl JsonSchema for Steps {
    fn schema_name() -> Cow<'static, str> {
        "Steps".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "array",
            "items": { "type": "object" },
            "description": "Steps of a plan, each with its `step`: the `plan_schema` property has them."
        })
    }
}

/// What a request becomes: a plan, checked.
pub trait Request {
    /// Its plan, a multi-position given none visiting `saved`.
    fn plan(&self, saved: &[Position]) -> Plan;
}

/// A request, its plan checked as it is read: a plan that couldn't run is a 422. A
/// multi-position given no positions is checked as if one were saved.
#[derive(Debug, Clone)]
pub struct Requested<R>(R);

impl<'de, R: Deserialize<'de> + Request> Deserialize<'de> for Requested<R> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let request = R::deserialize(deserializer)?;
        let stand_in = Position {
            name: "saved".to_owned(),
            x: 0.0,
            y: 0.0,
            z: None,
            focus: None,
        };
        let checked = request.plan(&[stand_in]).check();
        checked.map_err(serde::de::Error::custom)?;
        Ok(Self(request))
    }
}

impl<R: Serialize> Serialize for Requested<R> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.serialize(serializer)
    }
}

impl<R: JsonSchema> JsonSchema for Requested<R> {
    fn inline_schema() -> bool {
        R::inline_schema()
    }

    fn schema_name() -> Cow<'static, str> {
        R::schema_name()
    }

    fn schema_id() -> Cow<'static, str> {
        R::schema_id()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        R::json_schema(generator)
    }
}

/// Any plan.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PlanRequest {
    /// The dataset's name.
    pub name: String,
    /// The steps.
    pub steps: Steps,
    /// What a failed step does: by default, it ends the run.
    #[serde(default)]
    pub on_error: ErrorPolicy,
}

impl Request for PlanRequest {
    fn plan(&self, _: &[Position]) -> Plan {
        Plan {
            name: self.name.clone(),
            steps: self.steps.0.clone(),
            on_error: self.on_error,
        }
    }
}

/// A focus map made before a tile scan.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct FocusMapRequest {
    /// The places, `[x, y]` in stage µm: three or more.
    pub places: Vec<[f64; 2]>,
    /// The surface: a thin-plate spline if not given.
    #[serde(default)]
    pub surface: SurfaceKind,
    /// Each autofocus's range, in µm: 50 if not given.
    #[serde(default = "range")]
    pub range_um: f64,
}

fn range() -> f64 {
    50.0
}

fn overlap() -> f64 {
    0.1
}

/// A tile scan, after a focus map if one is given (ADR-0053).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TileScanRequest {
    /// The dataset's name.
    pub name: String,
    /// The region, in stage µm.
    pub region: Region,
    /// The overlap between neighbours, of the frame: 0.1 if not given.
    #[serde(default = "overlap")]
    pub overlap: f64,
    /// The order the tiles are visited in: a snake if not given.
    #[serde(default)]
    pub order: Order,
    /// The channels, in order: the one in use if none.
    #[serde(default)]
    pub channels: Vec<ChannelId>,
    /// How long the stage settles after each move, in ms.
    #[serde(default)]
    pub settle_ms: f64,
    /// How each tile is focused: as the focus is, if not given; `surface` with a focus map.
    #[serde(default)]
    pub focus: TileFocus,
    /// A z-stack at each tile, in place of one frame.
    #[serde(default)]
    pub z_stack: Option<ZPlanes>,
    /// A focus map made first.
    #[serde(default)]
    pub focus_map: Option<FocusMapRequest>,
}

impl Request for TileScanRequest {
    fn plan(&self, _: &[Position]) -> Plan {
        let map = self.focus_map.iter().map(|map| Step::FocusMap {
            places: map.places.clone(),
            surface: map.surface,
            range_um: map.range_um,
        });
        let scan = Step::TileScan {
            region: self.region.clone(),
            overlap: self.overlap,
            order: self.order,
            channels: self.channels.clone(),
            settle_ms: self.settle_ms,
            focus: self.focus,
            z_stack: self.z_stack,
        };
        Plan {
            name: self.name.clone(),
            steps: map.chain([scan]).collect(),
            on_error: ErrorPolicy::default(),
        }
    }
}

/// A z-stack where the stage is, about where the focus is (ADR-0055).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ZStackRequest {
    /// The dataset's name.
    pub name: String,
    /// The planes.
    pub planes: ZPlanes,
    /// What to take at each: a frame if none.
    #[serde(default)]
    pub steps: Steps,
}

impl Request for ZStackRequest {
    fn plan(&self, _: &[Position]) -> Plan {
        Plan {
            name: self.name.clone(),
            steps: vec![Step::ZStack {
                planes: self.planes,
                steps: self.steps.0.clone(),
            }],
            on_error: ErrorPolicy::default(),
        }
    }
}

/// A multi-position: the saved positions if none are given (ADR-0055).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MultiPositionRequest {
    /// The dataset's name.
    pub name: String,
    /// The positions: the saved ones if none.
    #[serde(default)]
    pub positions: Vec<Position>,
    /// An autofocus at each, over this range in µm, if given.
    #[serde(default)]
    pub autofocus: Option<f64>,
    /// What to take at each: a frame if none.
    #[serde(default)]
    pub steps: Steps,
}

impl Request for MultiPositionRequest {
    fn plan(&self, saved: &[Position]) -> Plan {
        let positions = match self.positions.is_empty() {
            true => saved.to_vec(),
            false => self.positions.clone(),
        };
        Plan {
            name: self.name.clone(),
            steps: vec![Step::MultiPosition {
                positions,
                autofocus: self.autofocus,
                steps: self.steps.0.clone(),
            }],
            on_error: ErrorPolicy::default(),
        }
    }
}

/// A time-lapse where the stage is (ADR-0055).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TimeLapseRequest {
    /// The dataset's name.
    pub name: String,
    /// The time between points' starts, in s.
    pub interval_s: f64,
    /// How many points.
    #[serde(default)]
    pub count: Option<u32>,
    /// How long, in s.
    #[serde(default)]
    pub duration_s: Option<f64>,
    /// What a late point does: skipped once its interval has passed, if not given.
    #[serde(default)]
    pub overrun: Overrun,
    /// What to do at each point: a frame if none.
    #[serde(default)]
    pub steps: Steps,
}

impl Request for TimeLapseRequest {
    fn plan(&self, _: &[Position]) -> Plan {
        Plan {
            name: self.name.clone(),
            steps: vec![Step::TimeLapse {
                interval_s: self.interval_s,
                count: self.count,
                duration_s: self.duration_s,
                overrun: self.overrun,
                steps: self.steps.0.clone(),
            }],
            on_error: ErrorPolicy::default(),
        }
    }
}

/// What a tile scan would be.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ScanEstimate {
    /// The tiles, in their order.
    pub tiles: Vec<Tile>,
    /// A frame's corners about its tile's centre, in µm.
    pub frame: [[f64; 2]; 4],
    /// The frames.
    pub frames: usize,
    /// How long it takes, in s.
    pub seconds: f64,
    /// The frames' bytes on disk.
    pub bytes: u64,
    /// The bytes free on the data root's disk, if known.
    pub free_bytes: Option<u64>,
    /// Whether the frames fit there, leaving the bytes kept free.
    pub fits: bool,
    /// What may go wrong.
    pub warnings: Vec<String>,
}

/// The calibration in force, and the frame in sensor pixels.
fn optics(instrument: &Instrument) -> Option<(siq_types::Affine2, [f64; 2])> {
    let calibration = instrument.acquisition.context().calibration;
    let record = calibration.and_then(|id| instrument.calibrations.get(id))?;
    let roi = instrument.camera.settings().roi;
    Some((
        record.affine(),
        [f64::from(roi.width), f64::from(roi.height)],
    ))
}

/// What `request` would take on `instrument`.
fn estimate(instrument: &Instrument, request: &TileScanRequest) -> Result<ScanEstimate, String> {
    let why = "a tile scan needs the objective's stage–camera calibration: calibrate it";
    let (l, field) = optics(instrument).ok_or(why)?;
    let grid = scan::plan(&request.region, &l, field, request.overlap, request.order);
    let grid = grid.map_err(|error| error.to_string())?;
    let frame = scan::frame(&l, field).ok_or("the calibration isn't invertible")?;
    let config = &instrument.config;
    let settings = instrument.camera.settings();
    let exposures = match request.channels.is_empty() {
        true => vec![settings.exposure.0 / 1000.0],
        false => request
            .channels
            .iter()
            .map(|id| {
                let found = config.channels.iter().find(|channel| channel.id == *id);
                let channel = found.ok_or_else(|| format!("there is no channel {id}"))?;
                Ok(channel.exposure.0 / 1000.0)
            })
            .collect::<Result<_, String>>()?,
    };
    let binning = settings.binning;
    let pixels = (field[0] as u64 / u64::from(binning.horizontal.get()))
        * (field[1] as u64 / u64::from(binning.vertical.get()));
    let focusing = Focusing {
        planes: request.z_stack.map_or(1, |planes| planes.planes().len()),
        autofocus: matches!(request.focus, TileFocus::Autofocus { .. }),
        map_places: request.focus_map.as_ref().map_or(0, |map| map.places.len()),
    };
    let mut timing = Timing::default();
    if let Some(kinematics) = instrument.stage.kinematics() {
        timing.speed_um_s = kinematics.speed_um_s;
        timing.acceleration_um_s2 = kinematics.acceleration_um_s2;
    }
    let at = instrument.stage.status().borrow().position;
    let settle_s = request.settle_ms / 1000.0;
    let (start, bytes) = ([at.x.0, at.y.0], pixels * 2);
    let estimated = scan::estimate(
        &grid, start, settle_s, &exposures, bytes, &focusing, &timing,
    );
    let free_bytes = free_space(instrument.catalog.data_root()).ok();
    let keep = (config.server.min_free_gb * 1e9) as u64;
    let fits = free_bytes.is_none_or(|free| estimated.fits(free, keep));
    let gb = |bytes: u64| bytes as f64 / 1e9;
    let mut warnings = Vec::new();
    if let (false, Some(free)) = (fits, free_bytes) {
        warnings.push(format!(
            "the frames take {:.1} GB; the disk has {:.1} GB free, {:.1} GB of it kept free",
            gb(estimated.bytes),
            gb(free),
            gb(keep)
        ));
    }
    if bytes as usize > instrument.slot_bytes {
        warnings.push("a frame is larger than the frame pool's slots".to_owned());
    }
    let travel = instrument.stage.limits();
    let (low, high) = (travel.min, travel.max);
    let outside = grid.tiles.iter().filter(|tile| {
        !(low.x.0..=high.x.0).contains(&tile.x) || !(low.y.0..=high.y.0).contains(&tile.y)
    });
    match outside.count() {
        0 => {}
        n => warnings.push(format!("{n} tiles are outside the stage's travel")),
    }
    if let Some(planes) = request.z_stack {
        let focus = instrument.focus.status().borrow().position.0;
        let travel = instrument.focus.limits();
        let half = planes.range_um / 2.0;
        if focus - half < travel.min.0 || focus + half > travel.max.0 {
            warnings.push("the z-stack leaves the focus drive's travel".to_owned());
        }
    }
    if request.overlap < 0.05 {
        warnings.push("under 5 % overlap leaves stitching little to register".to_owned());
    }
    Ok(ScanEstimate {
        tiles: grid.tiles,
        frame,
        frames: estimated.frames,
        seconds: estimated.seconds,
        bytes: estimated.bytes,
        free_bytes,
        fits,
        warnings,
    })
}

/// The saved positions, in a file of the settings folder.
struct Saved {
    path: PathBuf,
    positions: Mutex<Vec<Position>>,
}

impl Saved {
    /// The positions saved in `folder`'s `positions.json`: none if there is none.
    fn open(folder: &Path) -> anyhow::Result<Self> {
        let path = folder.join("positions.json");
        let positions = match std::fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text).with_context(|| path.display().to_string())?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(error) => return Err(error).context(path.display().to_string()),
        };
        Ok(Self {
            path,
            positions: Mutex::new(positions),
        })
    }

    fn list(&self) -> Vec<Position> {
        self.lock().clone()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<Position>> {
        self.positions
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// The positions changed by `change`, then written: a new file in place of the old.
    fn change(
        &self,
        change: impl FnOnce(&mut Vec<Position>) -> Result<(), String>,
    ) -> Result<Vec<Position>, ActionError> {
        let mut positions = self.lock();
        let mut changed = positions.clone();
        change(&mut changed).map_err(ActionError::handled)?;
        let text = serde_json::to_string_pretty(&changed)?;
        if let Some(folder) = self.path.parent() {
            std::fs::create_dir_all(folder)?;
        }
        let new = self.path.with_extension("json.new");
        std::fs::write(&new, text)?;
        std::fs::rename(&new, &self.path)?;
        *positions = changed;
        Ok(positions.clone())
    }
}

/// The workflows: plans run on the instrument, and the saved positions.
#[derive(Thing)]
pub struct Workflow {
    /// What the workflow is doing.
    #[property(readonly, default = RunState::Idle)]
    state: Prop<RunState>,
    /// How far the run under way, or the last, has gone.
    #[property(readonly, default = None)]
    progress: Prop<Option<Progress>>,
    /// The saved positions.
    #[property(readonly, default = Vec::new())]
    positions: Prop<Vec<Position>>,
    /// A frame was taken, for the dataset at these indices.
    #[event]
    tile_acquired: Event<Acquired>,
    /// A run ended.
    #[event]
    finished: Event<Finished>,
    instrument: Bound,
    saved: std::sync::OnceLock<Saved>,
    control: Mutex<Option<RunControl>>,
}

impl Workflow {
    fn saved(&self) -> &Saved {
        self.saved
            .get()
            .expect("a Thing is used only once it has started")
    }

    /// The run under way.
    fn under_way(&self) -> Result<RunControl, ActionError> {
        let control = self.control.lock().unwrap_or_else(PoisonError::into_inner);
        control
            .clone()
            .ok_or_else(|| ActionError::handled("no run is under way"))
    }

    /// Runs `request`'s plan, reporting as it goes, until it ends or the invocation is
    /// cancelled.
    async fn execute(
        &self,
        ctx: &ActionCtx,
        request: &impl Request,
    ) -> Result<Finished, ActionError> {
        let plan = request.plan(&self.saved().list());
        let instrument = bound(&self.instrument);
        let config = &instrument.config;
        let target = DatasetTarget {
            data_root: instrument.catalog.data_root().to_owned(),
            min_free: (config.server.min_free_gb * 1e9) as u64,
            catalog: Some(instrument.catalog.clone()),
            instrument: Snapshot::of(config).ok(),
        };
        let control = RunControl::new();
        *self.control.lock().unwrap_or_else(PoisonError::into_inner) = Some(control.clone());
        let _ = (self.state.set(RunState::Running), self.progress.set(None));
        let observe = |event: RunEvent| match event {
            RunEvent::Progress { progress } => {
                let _ = self.progress.set(Some(progress));
            }
            RunEvent::Frame { t, c, z, p } => self.tile_acquired.emit(Acquired { t, c, z, p }),
            RunEvent::Paused => {
                let _ = self.state.set(RunState::Paused);
            }
            RunEvent::Resumed => {
                let _ = self.state.set(RunState::Running);
            }
            RunEvent::Problem {
                step, error, then, ..
            } => tracing::warn!(%step, %error, ?then, "a step of {} failed", plan.name),
        };
        let devices = instrument.devices();
        let running = run(&devices, &target, &plan, &control, &observe);
        tokio::pin!(running);
        let mut cancelled = None;
        let ran = tokio::select! {
            report = &mut running => report,
            cancel = ctx.cancelled() => {
                cancelled = Some(cancel);
                control.cancel();
                running.await
            }
        };
        *self.control.lock().unwrap_or_else(PoisonError::into_inner) = None;
        let _ = self.state.set(RunState::Idle);
        let report = ran.map_err(ActionError::handled)?;
        for hook in instrument.hooks() {
            hook.on_dataset_complete(instrument, &report);
        }
        let finished = Finished {
            name: plan.name.clone(),
            dataset: report.dataset,
            outcome: report.outcome,
            error: report.error.clone(),
            steps: report.steps,
            frames: report.frames,
        };
        self.finished.emit(finished.clone());
        match (report.outcome, cancelled) {
            (Outcome::Finished, _) => Ok(finished),
            (Outcome::Cancelled, Some(cancel)) => Err(cancel.into()),
            _ => Err(ActionError::handled(report.error.unwrap_or_default())),
        }
    }
}

#[thing_impl]
impl Workflow {
    #[on_start]
    async fn start(&self, ctx: ThingCtx) -> anyhow::Result<()> {
        bind(&self.instrument, &ctx)?;
        let saved = Saved::open(&bound(&self.instrument).config.server.settings)?;
        self.positions.set(saved.list())?;
        let _ = self.saved.set(saved);
        Ok(())
    }

    /// A frame's corners about where the stage is, in µm, by the calibration in force and the
    /// camera's region: `null` without a calibration.
    #[property(global_lock = false)]
    async fn field(&self) -> Option<[[f64; 2]; 4]> {
        let (l, field) = optics(bound(&self.instrument))?;
        scan::frame(&l, field)
    }

    /// What `tile_scan` would do with this request, without doing it: its tiles in their order,
    /// a frame's corners about a tile, its time and bytes, whether the disk holds them, and what
    /// may go wrong. It doesn't wait for the global lock.
    #[action(global_lock = false)]
    async fn estimate_tile_scan(
        &self,
        #[input] request: Requested<TileScanRequest>,
    ) -> Result<ScanEstimate, ActionError> {
        estimate(bound(&self.instrument), &request.0).map_err(ActionError::handled)
    }

    /// The JSON Schema of a plan: its steps, which nest.
    #[property(global_lock = false)]
    async fn plan_schema(&self) -> serde_json::Value {
        serde_json::to_value(schemars::schema_for!(Plan)).unwrap_or_default()
    }

    /// Runs a plan of any shape: its dataset named `name`, its `steps` as `plan_schema` has them.
    /// It returns how the run ended; cancelled, it ends as cancelled, its dataset kept.
    #[action]
    async fn run(
        &self,
        ctx: ActionCtx,
        #[input] request: Requested<PlanRequest>,
    ) -> Result<Finished, ActionError> {
        self.execute(&ctx, &request.0).await
    }

    /// Covers `region` with tiles, after a focus map if given: its tiles focused as `focus` says,
    /// a frame in each channel at each, or a z-stack. It needs the stage–camera calibration.
    #[action]
    async fn tile_scan(
        &self,
        ctx: ActionCtx,
        #[input] request: Requested<TileScanRequest>,
    ) -> Result<Finished, ActionError> {
        self.execute(&ctx, &request.0).await
    }

    /// Takes a z-stack where the stage is, about where the focus is.
    #[action]
    async fn z_stack(
        &self,
        ctx: ActionCtx,
        #[input] request: Requested<ZStackRequest>,
    ) -> Result<Finished, ActionError> {
        self.execute(&ctx, &request.0).await
    }

    /// Visits each position, or each saved one if none are given, and takes its steps there.
    #[action]
    async fn multi_position(
        &self,
        ctx: ActionCtx,
        #[input] request: Requested<MultiPositionRequest>,
    ) -> Result<Finished, ActionError> {
        self.execute(&ctx, &request.0).await
    }

    /// Takes its steps at time points `interval_s` apart, where the stage is.
    #[action]
    async fn time_lapse(
        &self,
        ctx: ActionCtx,
        #[input] request: Requested<TimeLapseRequest>,
    ) -> Result<Finished, ActionError> {
        self.execute(&ctx, &request.0).await
    }

    /// Pauses the run under way before its next step. It doesn't wait for the global lock.
    #[action(global_lock = false)]
    async fn pause(&self) -> Result<(), ActionError> {
        self.under_way()?.pause();
        Ok(())
    }

    /// Lets a paused run go on. It doesn't wait for the global lock.
    #[action(global_lock = false)]
    async fn resume(&self) -> Result<(), ActionError> {
        self.under_way()?.resume();
        Ok(())
    }

    /// Saves a position named `name`: where the stage and the focus are, unless given. A name is
    /// saved once.
    #[action(global_lock = false)]
    async fn save_position(
        &self,
        name: String,
        #[param(default)] x: Option<f64>,
        #[param(default)] y: Option<f64>,
        #[param(default)] z: Option<f64>,
        #[param(default)] focus: Option<f64>,
    ) -> Result<Vec<Position>, ActionError> {
        let instrument = bound(&self.instrument);
        let at = instrument.stage.status().borrow().position;
        let focused = instrument.focus.status().borrow().position.0;
        let position = Position {
            name: name.trim().to_owned(),
            x: x.unwrap_or(at.x.0),
            y: y.unwrap_or(at.y.0),
            z: Some(z.unwrap_or(at.z.0)),
            focus: Some(focus.unwrap_or(focused)),
        };
        let saved = self.saved().change(|positions| {
            if position.name.is_empty() || positions.iter().any(|p| p.name == position.name) {
                return Err(format!(
                    "a position named {:?} can't be saved",
                    position.name
                ));
            }
            positions.push(position);
            Ok(())
        })?;
        self.positions.set(saved.clone())?;
        Ok(saved)
    }

    /// Renames the saved position `name` to `to`.
    #[action(global_lock = false)]
    async fn rename_position(
        &self,
        name: String,
        to: String,
    ) -> Result<Vec<Position>, ActionError> {
        let to = to.trim().to_owned();
        let saved = self.saved().change(|positions| {
            if to.is_empty() || positions.iter().any(|p| p.name == to) {
                return Err(format!("a position can't be named {to:?}"));
            }
            let found = positions.iter_mut().find(|p| p.name == name);
            found
                .ok_or_else(|| format!("no position is named {name:?}"))?
                .name = to;
            Ok(())
        })?;
        self.positions.set(saved.clone())?;
        Ok(saved)
    }

    /// Removes the saved position `name`.
    #[action(global_lock = false)]
    async fn remove_position(&self, name: String) -> Result<Vec<Position>, ActionError> {
        let saved = self.saved().change(|positions| {
            let before = positions.len();
            positions.retain(|p| p.name != name);
            match positions.len() < before {
                true => Ok(()),
                false => Err(format!("no position is named {name:?}")),
            }
        })?;
        self.positions.set(saved.clone())?;
        Ok(saved)
    }
}
