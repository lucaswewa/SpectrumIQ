//! Running a plan: its dataset, its steps, and how it ends.

use std::collections::{HashSet, VecDeque};
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use siq_buffer::Policy;
use siq_devices::{Approach, DeviceError, MoveOpts, Xyz};
use siq_storage::{
    Catalog, Dataset, DatasetKind, DatasetWriter, FrameKey, NewDataset, Snapshot, StorageError,
    WriterOptions,
};
use siq_types::{DatasetId, Micrometers, Timestamp};

use super::{
    ErrorPolicy, Index, Overrun, Plan, Position, Progress, RunControl, RunEvent, Step, Then,
    TileFocus, count,
};
use crate::DeviceSet;
use crate::autofocus::{self, AutofocusPlan, FittedSurface, FocusSurface};
use crate::routines::channel;
use crate::scan::{self, Grid};

/// The file in a dataset's folder that holds its [`Layout`].
pub const LAYOUT: &str = "layout.json";

/// Where a dataset's places were planned, and how its sequence ran: its tile scans' grids, its
/// positions, its focus maps and its time-lapses' points.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Layout {
    /// The tile scans' grids.
    pub scans: Vec<ScanLayout>,
    /// The multi-positions' positions.
    pub positions: Vec<Placed>,
    /// The focus maps made.
    pub focus_maps: Vec<FocusMap>,
    /// The time-lapses, as they ran.
    pub time_lapses: Vec<TimeLapseRecord>,
}

/// A tile scan's grid, its tiles numbered `p` from `first_p` in their order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ScanLayout {
    /// The first tile's `p`.
    pub first_p: u32,
    /// The grid: each tile's column, row and planned position.
    #[serde(flatten)]
    pub grid: Grid,
}

/// A multi-position's position, where the stage went, and its `p`: none when the tiles of a
/// scan there are numbered instead.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Placed {
    /// Its `p`.
    pub p: Option<u32>,
    /// The position, in stage µm.
    #[serde(flatten)]
    pub position: Position,
}

/// A time-lapse as it ran: the points it ran, and those it skipped.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TimeLapseRecord {
    /// The time between points, in s.
    pub interval_s: f64,
    /// What a late point did.
    pub overrun: Overrun,
    /// The points run.
    pub points: Vec<TimePoint>,
    /// The `t` of each point skipped.
    pub skipped: Vec<u32>,
}

/// A time point run: when it was due, and when it began, in s from its time-lapse's start.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TimePoint {
    /// Its `t`.
    pub t: u32,
    /// When it was due.
    pub due_s: f64,
    /// When it began.
    pub began_s: f64,
}

/// Where a run's dataset goes, and what it records it was made with.
#[derive(Debug, Clone)]
pub struct DatasetTarget {
    /// The data root: the dataset goes in its `datasets/`.
    pub data_root: PathBuf,
    /// The bytes to leave free on the disk.
    pub min_free: u64,
    /// The catalog to list the dataset in, when its frames are written.
    pub catalog: Option<Arc<Catalog>>,
    /// The instrument's configuration.
    pub instrument: Option<Snapshot>,
}

/// How a run ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// Every step was done, or skipped.
    Finished,
    /// It was cancelled.
    Cancelled,
    /// A step failed, and its policy ended the run.
    Failed,
}

/// What a run did.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RunReport {
    /// Its dataset.
    pub dataset: DatasetId,
    /// The dataset's folder.
    pub path: PathBuf,
    /// How it ended.
    pub outcome: Outcome,
    /// Why it failed, if it did.
    pub error: Option<String>,
    /// The steps done, and in the plan.
    pub steps: [u64; 2],
    /// The frames recorded.
    pub frames: u64,
}

/// Why a run didn't start.
#[derive(Debug, thiserror::Error)]
pub enum RunError {
    /// The plan can't be run.
    #[error("the plan can't be run: {0}")]
    Invalid(String),
    /// Its dataset couldn't be made, or recorded to.
    #[error(transparent)]
    Storage(#[from] StorageError),
}

/// A focus map as made: where focus was found, `[x, y, z]`, where there was nothing to focus on,
/// and the surface fitted.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FocusMap {
    /// The places focused, and the focus there.
    pub found: Vec<[f64; 3]>,
    /// The places with nothing to focus on.
    pub missed: Vec<[f64; 2]>,
    /// The surface, and its residuals.
    pub fitted: FittedSurface,
}

/// Why the steps stopped.
enum Stop {
    Cancelled,
    Failed(String),
}

type Steps<'a> = Pin<Box<dyn Future<Output = Result<(), Stop>> + Send + 'a>>;

/// A run under way: the indices in force, and how far it has gone.
struct Runner<'a> {
    devices: &'a DeviceSet,
    control: &'a RunControl,
    observe: &'a (dyn Fn(RunEvent) + Send + Sync),
    /// The keys of the frames snapped, in order, for the writer.
    keys: Arc<Mutex<VecDeque<FrameKey>>>,
    /// Every key taken: a frame has one of its own.
    taken: HashSet<FrameKey>,
    /// The focus the last autofocus found.
    focused: Option<f64>,
    /// The focus surface in force, and every focus map made.
    surface: Option<FocusSurface>,
    maps: Vec<FocusMap>,
    /// Every time-lapse run.
    lapses: Vec<TimeLapseRecord>,
    /// `t`, `z` and `p`.
    indices: [u32; 3],
    channel: u16,
    done: u64,
    total: u64,
    began: Timestamp,
}

/// Runs `plan` on `devices`, its dataset in `target`, as `control` asks, telling `observe` what
/// happens. It ends when the plan does, when it's cancelled, or when a failed step's policy
/// says so: the report says which. Once the dataset is made, it is always reported.
pub async fn run(
    devices: &DeviceSet,
    target: &DatasetTarget,
    plan: &Plan,
    control: &RunControl,
    observe: &(dyn Fn(RunEvent) + Send + Sync),
) -> Result<RunReport, RunError> {
    plan.check().map_err(RunError::Invalid)?;
    let mut layout = Layout::default();
    let steps = resolve(devices, &plan.steps, &mut layout)?;
    validate(devices, &steps)?;
    let stacks = contains(&steps, &|step| matches!(step, Step::ZStack { .. }));
    let times = |step: &Step| matches!(step, Step::TimeLapse { .. } | Step::Repeat { .. });
    let lapses = contains(&steps, &times);
    let kind = match (layout.scans.is_empty(), layout.positions.is_empty()) {
        (false, _) => DatasetKind::TileScan,
        (true, true) if stacks && !lapses => DatasetKind::ZStack,
        _ => DatasetKind::Sequence,
    };
    let mut new = NewDataset::new(kind, &plan.name);
    new.instrument = target.instrument.clone();
    let calibration = devices.acquisition.context().calibration;
    let record = calibration.and_then(|id| devices.calibrations.get(id));
    let snapshot = record.map(|record| Snapshot::of(&record)).transpose();
    new.calibration = snapshot.map_err(|error| RunError::Invalid(error.to_string()))?;
    new.plan = Some(serde_json::to_value(plan).map_err(|e| RunError::Invalid(e.to_string()))?);
    let dataset = Dataset::create(&target.data_root, new)?;

    let keys = Arc::new(Mutex::new(VecDeque::new()));
    let frames = devices
        .acquisition
        .subscribe("workflow", Policy::Lossless, |meta| meta.flags.save);
    let mut options = WriterOptions::new(target.min_free);
    options.catalog = target.catalog.clone();
    let queued = Arc::clone(&keys);
    options.key = Box::new(move |meta| {
        let next = queued
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front();
        next.unwrap_or(FrameKey {
            t: 0,
            c: meta.channel.map_or(0, |channel| channel.0),
            z: 0,
            p: 0,
        })
    });
    let writer = DatasetWriter::start(&dataset, frames, options)?;

    let mut runner = Runner {
        devices,
        control,
        observe,
        keys,
        taken: HashSet::new(),
        focused: None,
        surface: None,
        maps: Vec::new(),
        lapses: Vec::new(),
        indices: [0; 3],
        channel: 0,
        done: 0,
        total: count(&steps),
        began: devices.clock.now(),
    };
    let ended = runner.steps(&steps, plan.on_error).await;
    if ended.is_err() {
        // However it stopped, the stage and focus are still.
        let (stage, focus) = tokio::join!(devices.stage.stop(), devices.focus.stop());
        for (what, result) in [("stage", stage), ("focus", focus)] {
            if let Err(error) = result {
                tracing::warn!(%error, "the {what} didn't stop");
            }
        }
    }
    layout.focus_maps = runner.maps;
    layout.time_lapses = runner.lapses;
    if layout != Layout::default() {
        let path = dataset.path().join(LAYOUT);
        let text =
            serde_json::to_string_pretty(&layout).map_err(|e| RunError::Invalid(e.to_string()))?;
        std::fs::write(&path, text).map_err(|error| StorageError::Io {
            path,
            source: error,
        })?;
    }
    let written = tokio::task::spawn_blocking(move || writer.finish()).await;
    let written = written.map_err(|error| RunError::Invalid(error.to_string()))?;
    let (outcome, error) = match (ended, written.error) {
        (Ok(()), None) => (Outcome::Finished, None),
        (Err(Stop::Cancelled), None) => (Outcome::Cancelled, None),
        (Err(Stop::Failed(why)), _) => (Outcome::Failed, Some(why)),
        (_, Some(error)) => (Outcome::Failed, Some(error.to_string())),
    };
    Ok(RunReport {
        dataset: dataset.manifest().id,
        path: dataset.path().to_owned(),
        outcome,
        error,
        steps: [runner.done, runner.total],
        frames: written.stats.frames,
    })
}

/// Whether `steps`, or the steps within them, have one that is `wanted`.
fn contains(steps: &[Step], wanted: &dyn Fn(&Step) -> bool) -> bool {
    steps.iter().any(|step| {
        wanted(step)
            || match step {
                Step::Repeat { steps, .. }
                | Step::Policy { steps, .. }
                | Step::ZStack { steps, .. }
                | Step::MultiPosition { steps, .. }
                | Step::TimeLapse { steps, .. } => contains(steps, wanted),
                _ => false,
            }
    })
}

/// `steps` with each tile scan made into its moves and snaps, by the stage–camera calibration in
/// force and the camera's region, and each multi-position into its moves and steps at each
/// position. `p` numbers every position and tile in order, and each grid and position goes in
/// `layout`.
pub fn resolve(
    devices: &DeviceSet,
    steps: &[Step],
    layout: &mut Layout,
) -> Result<Vec<Step>, RunError> {
    resolve_about(devices, steps, [0.0, 0.0], &mut 0, layout)
}

/// [`resolve`], the places of `steps` about `[ox, oy]`, and `p` from `next_p`.
fn resolve_about(
    devices: &DeviceSet,
    steps: &[Step],
    [ox, oy]: [f64; 2],
    next_p: &mut u32,
    layout: &mut Layout,
) -> Result<Vec<Step>, RunError> {
    // What a z-stack or a position takes, if not given: a frame.
    let frame = |steps: &[Step]| match steps.is_empty() {
        true => vec![Step::Snap],
        false => steps.to_vec(),
    };
    let mut out = Vec::with_capacity(steps.len());
    for step in steps {
        match step {
            Step::TileScan {
                region,
                overlap,
                order,
                channels,
                settle_ms,
                focus,
                z_stack,
            } => {
                let calibration = devices.acquisition.context().calibration;
                let record = calibration.and_then(|id| devices.calibrations.get(id));
                let why =
                    "a tile scan needs the objective's stage–camera calibration: calibrate it";
                let record = record.ok_or_else(|| RunError::Invalid(why.to_owned()))?;
                let roi = devices.camera.settings().roi;
                let field = [f64::from(roi.width), f64::from(roi.height)];
                let region = region.shifted(ox, oy);
                let grid = scan::plan(&region, &record.affine(), field, *overlap, *order)
                    .map_err(|error| RunError::Invalid(error.to_string()))?;
                let first_p = *next_p;
                if let TileFocus::Fixed { z } = *focus {
                    out.push(Step::Focus { z });
                }
                if let [id] = channels[..] {
                    out.push(Step::Channel { id });
                }
                for tile in &grid.tiles {
                    out.push(Step::Index {
                        index: Index::P,
                        value: *next_p,
                    });
                    *next_p += 1;
                    let (x, y) = (tile.x, tile.y);
                    out.push(Step::MoveTo { x, y, z: None });
                    if *settle_ms > 0.0 {
                        out.push(Step::Wait { ms: *settle_ms });
                    }
                    match *focus {
                        TileFocus::Surface => out.push(Step::FocusOnSurface { x, y }),
                        TileFocus::Autofocus { range_um } => out.push(Step::Autofocus { range_um }),
                        TileFocus::None | TileFocus::Fixed { .. } => {}
                    }
                    let snaps = match channels.len() {
                        0 | 1 => vec![Step::Snap],
                        _ => channels
                            .iter()
                            .flat_map(|&id| [Step::Channel { id }, Step::Snap])
                            .collect(),
                    };
                    match z_stack {
                        Some(planes) => out.push(Step::ZStack {
                            planes: *planes,
                            steps: snaps,
                        }),
                        None => out.extend(snaps),
                    }
                }
                layout.scans.push(ScanLayout { first_p, grid });
            }
            Step::MultiPosition {
                positions,
                autofocus,
                steps,
            } => {
                for position in positions {
                    let (x, y) = (position.x + ox, position.y + oy);
                    let before = *next_p;
                    let inner = resolve_about(devices, &frame(steps), [x, y], next_p, layout)?;
                    // Numbered itself unless the tiles of a scan there are.
                    let p = (*next_p == before).then(|| {
                        *next_p += 1;
                        before
                    });
                    if let Some(value) = p {
                        out.push(Step::Index {
                            index: Index::P,
                            value,
                        });
                    }
                    out.push(Step::MoveTo {
                        x,
                        y,
                        z: position.z,
                    });
                    out.extend(position.focus.map(|z| Step::Focus { z }));
                    out.extend(autofocus.map(|range_um| Step::Autofocus { range_um }));
                    out.extend(inner);
                    let position = Position {
                        x,
                        y,
                        ..position.clone()
                    };
                    layout.positions.push(Placed { p, position });
                }
            }
            Step::ZStack { planes, steps } => out.push(Step::ZStack {
                planes: *planes,
                steps: resolve_about(devices, &frame(steps), [ox, oy], next_p, layout)?,
            }),
            Step::TimeLapse {
                interval_s,
                count,
                duration_s,
                overrun,
                steps,
            } => out.push(Step::TimeLapse {
                interval_s: *interval_s,
                count: *count,
                duration_s: *duration_s,
                overrun: *overrun,
                steps: resolve_about(devices, &frame(steps), [ox, oy], next_p, layout)?,
            }),
            Step::Repeat {
                count,
                index,
                steps,
            } => out.push(Step::Repeat {
                count: *count,
                index: *index,
                steps: resolve_about(devices, steps, [ox, oy], next_p, layout)?,
            }),
            Step::Policy { on_error, steps } => out.push(Step::Policy {
                on_error: *on_error,
                steps: resolve_about(devices, steps, [ox, oy], next_p, layout)?,
            }),
            Step::MoveTo { x, y, z } => out.push(Step::MoveTo {
                x: x + ox,
                y: y + oy,
                z: *z,
            }),
            Step::FocusMap {
                places,
                surface,
                range_um,
            } => out.push(Step::FocusMap {
                places: places.iter().map(|[x, y]| [x + ox, y + oy]).collect(),
                surface: *surface,
                range_um: *range_um,
            }),
            Step::FocusOnSurface { x, y } => out.push(Step::FocusOnSurface {
                x: x + ox,
                y: y + oy,
            }),
            step => out.push(step.clone()),
        }
    }
    Ok(out)
}

impl Plan {
    /// Refuses a plan that couldn't run on any instrument: no name or no steps, a wait or an
    /// autofocus of no length, a focus map of fewer than three places, a focus on the surface
    /// before any map, a z-stack of no planes or too many, a time-lapse of no points, or a
    /// multi-position of none.
    pub fn check(&self) -> Result<(), String> {
        if self.name.trim().is_empty() || self.steps.is_empty() {
            return Err("it needs a name, and a step".to_owned());
        }
        check(&self.steps, &mut false)
    }
}

/// [`Plan::check`] of `steps`, `mapped` once a focus map comes before.
fn check(steps: &[Step], mapped: &mut bool) -> Result<(), String> {
    let range = |range_um: f64| match range_um.is_finite() && range_um > 0.0 {
        true => Ok(()),
        false => Err(format!("an autofocus over {range_um} µm")),
    };
    let planes = |planes: &super::ZPlanes| {
        let n = planes.range_um / planes.step_um;
        let fits = planes.step_um > 0.0 && n.is_finite() && n >= 0.0;
        match fits && n + 1.0 <= f64::from(super::MAX_PLANES) {
            true => Ok(()),
            false => Err(format!(
                "a z-stack {} µm across in steps of {} µm: from 1 to {} planes",
                planes.range_um,
                planes.step_um,
                super::MAX_PLANES
            )),
        }
    };
    let surface = |mapped: bool| match mapped {
        true => Ok(()),
        false => Err("a focus on the surface before any focus map".to_owned()),
    };
    for step in steps {
        match step {
            Step::Wait { ms } if !ms.is_finite() || *ms < 0.0 => {
                return Err(format!("a wait of {ms} ms"));
            }
            Step::Autofocus { range_um } => range(*range_um)?,
            Step::FocusMap {
                places, range_um, ..
            } => {
                if places.len() < 3 {
                    return Err(format!("a focus map of {} places", places.len()));
                }
                range(*range_um)?;
                *mapped = true;
            }
            Step::FocusOnSurface { .. } => surface(*mapped)?,
            Step::TileScan { focus, z_stack, .. } => {
                match focus {
                    TileFocus::Autofocus { range_um } => range(*range_um)?,
                    TileFocus::Surface => surface(*mapped)?,
                    TileFocus::None | TileFocus::Fixed { .. } => {}
                }
                z_stack.as_ref().map_or(Ok(()), planes)?;
            }
            Step::ZStack { planes: p, steps } => {
                planes(p)?;
                check(steps, mapped)?;
            }
            Step::MultiPosition {
                positions,
                autofocus,
                steps,
            } => {
                if positions.is_empty() {
                    return Err("a multi-position of no positions".to_owned());
                }
                autofocus.map_or(Ok(()), range)?;
                check(steps, mapped)?;
            }
            Step::TimeLapse {
                interval_s,
                count,
                duration_s,
                steps,
                ..
            } => {
                let duration = duration_s.is_none_or(|d| d.is_finite() && d > 0.0);
                let points = Step::points(*count, *duration_s, *interval_s);
                if !(interval_s.is_finite() && *interval_s > 0.0 && duration && points > 0) {
                    return Err(format!(
                        "a time-lapse every {interval_s} s: it needs an interval, and a count \
                         or a duration"
                    ));
                }
                check(steps, mapped)?;
            }
            Step::Repeat { steps, .. } | Step::Policy { steps, .. } => check(steps, mapped)?,
            _ => {}
        }
    }
    Ok(())
}

/// Refuses a plan, its `steps` resolved, that can't run on `devices`: one with a channel they
/// haven't, or with no step of a device's.
fn validate(devices: &DeviceSet, steps: &[Step]) -> Result<(), RunError> {
    fn channels(devices: &DeviceSet, steps: &[Step]) -> Result<(), String> {
        for step in steps {
            match step {
                Step::Channel { id } if !devices.channels.iter().any(|c| c.id == *id) => {
                    return Err(format!("there is no channel {id}"));
                }
                Step::Repeat { steps, .. }
                | Step::Policy { steps, .. }
                | Step::ZStack { steps, .. }
                | Step::TimeLapse { steps, .. } => channels(devices, steps)?,
                _ => {}
            }
        }
        Ok(())
    }
    channels(devices, steps).map_err(RunError::Invalid)?;
    if count(steps) == 0 {
        return Err(RunError::Invalid("it needs a name, and a step".to_owned()));
    }
    Ok(())
}

impl Runner<'_> {
    /// Runs `steps`, a failed one doing as `policy` says.
    fn steps<'s>(&'s mut self, steps: &'s [Step], policy: ErrorPolicy) -> Steps<'s> {
        Box::pin(async move {
            for step in steps {
                match step {
                    Step::Repeat {
                        count,
                        index,
                        steps,
                    } => {
                        // Its index counts within it, and is as it was after it.
                        let before = self.indices;
                        for time in 0..*count {
                            if let Some(index) = index {
                                self.indices[*index as usize] = time;
                            }
                            self.steps(steps, policy).await?;
                        }
                        self.indices = before;
                    }
                    Step::Policy { on_error, steps } => self.steps(steps, *on_error).await?,
                    Step::Index { index, value } => self.indices[*index as usize] = *value,
                    Step::ZStack { planes, steps } => {
                        // About where the focus is; its z counts within it.
                        let middle = self.devices.focus.status().borrow().position.0;
                        let before = self.indices;
                        for (z, offset) in planes.planes() {
                            self.indices[Index::Z as usize] = z;
                            self.step(&Step::Focus { z: middle + offset }, policy)
                                .await?;
                            self.steps(steps, policy).await?;
                        }
                        self.indices = before;
                        self.step(&Step::Focus { z: middle }, policy).await?;
                    }
                    Step::TimeLapse {
                        interval_s,
                        count,
                        duration_s,
                        overrun,
                        steps,
                    } => {
                        let points = Step::points(*count, *duration_s, *interval_s);
                        self.time_lapse(*interval_s, points, *overrun, steps, policy)
                            .await?;
                    }
                    Step::FocusMap {
                        places,
                        surface,
                        range_um,
                    } => self.focus_map(places, *surface, *range_um, policy).await?,
                    step => self.step(step, policy).await?,
                }
            }
            Ok(())
        })
    }

    /// Runs one device step once the run may go on, trying it again or skipping it as `policy`
    /// says if it fails.
    async fn step(&mut self, step: &Step, policy: ErrorPolicy) -> Result<(), Stop> {
        if self.control.command() == super::Command::Pause {
            (self.observe)(RunEvent::Paused);
            let go_on = self.control.unpaused().await;
            if go_on {
                (self.observe)(RunEvent::Resumed);
            }
        }
        if self.control.command() == super::Command::Cancel {
            return Err(Stop::Cancelled);
        }
        let what = self.describe(step);
        for attempt in 1.. {
            let error = match self.act(step).await {
                Ok(()) => break,
                Err(Stop::Cancelled) => return Err(Stop::Cancelled),
                Err(Stop::Failed(error)) => error,
            };
            let then = match (attempt <= policy.retries, policy.skip) {
                (true, _) => Then::Retry,
                (false, true) => Then::Skip,
                (false, false) => Then::Stop,
            };
            let step = what.clone();
            let problem = RunEvent::Problem {
                step,
                attempt,
                error: error.clone(),
                then,
            };
            (self.observe)(problem);
            match then {
                Then::Retry => continue,
                Then::Skip => break,
                Then::Stop => return Err(Stop::Failed(format!("{what}: {error}"))),
            }
        }
        self.done += 1;
        let elapsed = self.devices.clock.now().saturating_since(self.began);
        let left = (self.total - self.done) as f64;
        let progress = Progress {
            done: self.done,
            total: self.total,
            eta_s: Some(elapsed.as_secs_f64() / self.done as f64 * left),
            step: what,
        };
        (self.observe)(RunEvent::Progress { progress });
        Ok(())
    }

    /// What `step` does, for the progress and the problems.
    fn describe(&self, step: &Step) -> String {
        let [t, z, p] = self.indices;
        match step {
            Step::MoveTo { x, y, z: Some(h) } => format!("move to ({x}, {y}, {h}) µm"),
            Step::MoveTo { x, y, z: None } => format!("move to ({x}, {y}) µm"),
            Step::Focus { z } => format!("focus to {z} µm"),
            Step::Channel { id } => format!("channel {id}"),
            Step::Snap => format!("snap t{t} c{} z{z} p{p}", self.channel),
            Step::Wait { ms } => format!("wait {ms} ms"),
            Step::Autofocus { range_um } => format!("autofocus over {range_um} µm"),
            Step::FocusOnSurface { x, y } => format!("focus on the surface at ({x}, {y}) µm"),
            Step::Repeat { .. }
            | Step::Policy { .. }
            | Step::Index { .. }
            | Step::TileScan { .. }
            | Step::FocusMap { .. }
            | Step::ZStack { .. }
            | Step::MultiPosition { .. }
            | Step::TimeLapse { .. } => String::new(),
        }
    }

    /// Does `step` once. A move is abandoned if the run is cancelled meanwhile; a channel, a
    /// capture and a wait finish first.
    async fn act(&mut self, step: &Step) -> Result<(), Stop> {
        let devices = self.devices;
        let failed = |error: DeviceError| Stop::Failed(error.to_string());
        let opts = MoveOpts {
            speed: None,
            approach: Approach::Unidirectional,
        };
        match step {
            Step::MoveTo { x, y, z } => {
                let at = devices.stage.status().borrow().position;
                let target = Xyz {
                    x: Micrometers(*x),
                    y: Micrometers(*y),
                    z: z.map_or(at.z, Micrometers),
                };
                let moved = self
                    .unless_cancelled(devices.stage.move_to(target, opts))
                    .await?;
                moved.map(drop).map_err(failed)
            }
            Step::Focus { z } => {
                let focus = devices.focus.move_to(Micrometers(*z), opts);
                self.unless_cancelled(focus)
                    .await?
                    .map(drop)
                    .map_err(failed)
            }
            Step::Channel { id } => {
                let found = devices.channels.iter().find(|channel| channel.id == *id);
                let config = found.ok_or_else(|| Stop::Failed(format!("no channel {id}")))?;
                let applied = channel::apply(devices, config).await;
                applied.map_err(|error| Stop::Failed(error.to_string()))?;
                self.channel = id.0;
                Ok(())
            }
            Step::Snap => {
                let [t, z, p] = self.indices;
                let c = self.channel;
                let key = FrameKey { t, c, z, p };
                if !self.taken.insert(key) {
                    let why = format!("a frame was already taken at {key}: number the snaps");
                    return Err(Stop::Failed(why));
                }
                let keys = &self.keys;
                let queue = || keys.lock().unwrap_or_else(PoisonError::into_inner);
                queue().push_back(key);
                match devices.acquisition.snap(true).await {
                    Ok(_) => {
                        (self.observe)(RunEvent::Frame { t, c, z, p });
                        Ok(())
                    }
                    Err(error) => {
                        queue().pop_back();
                        self.taken.remove(&key);
                        Err(failed(error))
                    }
                }
            }
            Step::Wait { ms } => {
                tokio::time::sleep(Duration::from_secs_f64(ms / 1000.0)).await;
                Ok(())
            }
            Step::Autofocus { range_um } => {
                let plan = AutofocusPlan {
                    range_um: *range_um,
                    ..AutofocusPlan::default()
                };
                let found = autofocus::autofocus(devices, &plan).await;
                let found = found.map_err(|error| Stop::Failed(error.to_string()))?;
                self.focused = found.z;
                match found.z {
                    Some(_) => Ok(()),
                    None => Err(Stop::Failed(format!(
                        "nothing to focus on: the sharpness rose by {:.0} %",
                        found.rise * 100.0
                    ))),
                }
            }
            Step::FocusOnSurface { x, y } => {
                let surface = self.surface.as_ref();
                let why = || Stop::Failed("no focus map has been made".to_owned());
                let z = surface.ok_or_else(why)?.z(*x, *y);
                let moved = devices.focus.move_to(Micrometers(z), opts);
                self.unless_cancelled(moved)
                    .await?
                    .map(drop)
                    .map_err(failed)
            }
            Step::Repeat { .. }
            | Step::Policy { .. }
            | Step::Index { .. }
            | Step::TileScan { .. }
            | Step::FocusMap { .. }
            | Step::ZStack { .. }
            | Step::MultiPosition { .. }
            | Step::TimeLapse { .. } => Ok(()),
        }
    }

    /// Autofocuses at each of `places`, a place with nothing to focus on left out, and fits the
    /// surface in force to the rest.
    async fn focus_map(
        &mut self,
        places: &[[f64; 2]],
        kind: autofocus::SurfaceKind,
        range_um: f64,
        policy: ErrorPolicy,
    ) -> Result<(), Stop> {
        let (mut found, mut missed) = (Vec::new(), Vec::new());
        let lenient = ErrorPolicy {
            skip: true,
            ..policy
        };
        for &[x, y] in places {
            self.step(&Step::MoveTo { x, y, z: None }, policy).await?;
            self.focused = None;
            self.step(&Step::Autofocus { range_um }, lenient).await?;
            match self.focused {
                Some(z) => found.push([x, y, z]),
                None => missed.push([x, y]),
            }
        }
        let fitted = autofocus::fit_surface(&found, kind).map_err(|error| {
            Stop::Failed(format!(
                "the focus map found focus at {} places: {error}",
                found.len()
            ))
        })?;
        self.surface = Some(fitted.surface.clone());
        self.maps.push(FocusMap {
            found,
            missed,
            fitted,
        });
        Ok(())
    }

    /// Runs `steps` at `points` time points `interval_s` apart, numbering them `t`, each when it
    /// is due; a late one as `overrun` says.
    async fn time_lapse(
        &mut self,
        interval_s: f64,
        points: u32,
        overrun: Overrun,
        steps: &[Step],
        policy: ErrorPolicy,
    ) -> Result<(), Stop> {
        let start = tokio::time::Instant::now();
        let since = move || start.elapsed().as_secs_f64();
        let mut record = TimeLapseRecord {
            interval_s,
            overrun,
            points: Vec::new(),
            skipped: Vec::new(),
        };
        let (before, mut shift) = (self.indices, 0.0);
        let mut ran = Ok(());
        for t in 0..points {
            let due_s = shift + f64::from(t) * interval_s;
            ran = self.until(start + Duration::from_secs_f64(due_s)).await;
            if ran.is_err() {
                break;
            }
            let late = since() - due_s;
            match overrun {
                Overrun::Skip if late >= interval_s => {
                    record.skipped.push(t);
                    self.total = self.total.saturating_sub(count(steps));
                    continue;
                }
                Overrun::Shift if late > 0.0 => shift += late,
                _ => {}
            }
            self.indices[Index::T as usize] = t;
            let began_s = since();
            record.points.push(TimePoint { t, due_s, began_s });
            ran = self.steps(steps, policy).await;
            if ran.is_err() {
                break;
            }
        }
        self.indices = before;
        self.lapses.push(record);
        ran
    }

    /// Waits until `deadline`, then while the run is paused; a cancel stops it.
    async fn until(&self, deadline: tokio::time::Instant) -> Result<(), Stop> {
        self.unless_cancelled(tokio::time::sleep_until(deadline))
            .await?;
        if self.control.command() == super::Command::Pause {
            (self.observe)(RunEvent::Paused);
            if !self.control.unpaused().await {
                return Err(Stop::Cancelled);
            }
            (self.observe)(RunEvent::Resumed);
        }
        Ok(())
    }

    /// `operation`'s result, unless the run is cancelled first.
    async fn unless_cancelled<T>(&self, operation: impl Future<Output = T>) -> Result<T, Stop> {
        tokio::select! {
            done = operation => Ok(done),
            () = self.control.cancelled() => Err(Stop::Cancelled),
        }
    }
}
