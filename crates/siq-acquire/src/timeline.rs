//! The pose timeline: the stage's and the focus drive's recent readings, and the pose they give at
//! a frame's time (ADR-0031).
//!
//! - **Settled:** a reading at or before the time and one after it, both still, at the same
//!   position.
//! - **Interpolated:** between the readings either side, or carried on from the last two.
//!   The uncertainty is the speed times the readings' spacing: how far the axes went between them.
//! - **Unknown:** no reading at or before the time.

use std::collections::VecDeque;
use std::sync::{Mutex, MutexGuard, PoisonError};

use siq_devices::{AxisStatus, FocusStatus, MotionState, StageStatus};
use siq_types::{Micrometers, PoseQuality, StagePose, Timestamp};

/// One reading of an axis group.
#[derive(Debug, Clone, Copy)]
struct Sample<const N: usize> {
    ts: Timestamp,
    position: [f64; N],
    idle: bool,
}

/// What a group's readings say about one time.
#[derive(Debug, Clone, Copy)]
struct Estimate<const N: usize> {
    position: [f64; N],
    settled: bool,
    uncertainty: f64,
    moving: bool,
}

fn distance<const N: usize>(a: &[f64; N], b: &[f64; N]) -> f64 {
    a.iter()
        .zip(b)
        .map(|(a, b)| (b - a).powi(2))
        .sum::<f64>()
        .sqrt()
}

fn seconds(from: Timestamp, to: Timestamp) -> f64 {
    (to.0 as f64 - from.0 as f64) / 1e9
}

/// The recent readings of one group, oldest first.
#[derive(Debug)]
struct Track<const N: usize> {
    samples: VecDeque<Sample<N>>,
}

impl<const N: usize> Track<N> {
    fn push(&mut self, sample: Sample<N>) {
        if self.samples.len() == self.samples.capacity() {
            self.samples.pop_front();
        }
        // Readings come in order, nearly: one timestamped a little earlier goes in its place.
        let at = self.samples.partition_point(|s| s.ts <= sample.ts);
        self.samples.insert(at, sample);
    }

    fn newer_than(&self, t: Timestamp) -> bool {
        self.samples.back().is_some_and(|last| last.ts > t)
    }

    fn still(&self) -> bool {
        self.samples.back().is_none_or(|last| last.idle)
    }

    fn at(&self, t: Timestamp) -> Option<Estimate<N>> {
        let i = self.samples.partition_point(|s| s.ts <= t);
        let a = self.samples.get(i.checked_sub(1)?)?;
        if let Some(b) = self.samples.get(i) {
            let f = seconds(a.ts, t) / seconds(a.ts, b.ts).max(f64::MIN_POSITIVE);
            let gone = distance(&a.position, &b.position);
            let still = a.idle && b.idle && gone == 0.0;
            return Some(Estimate {
                position: std::array::from_fn(|k| {
                    a.position[k] + f * (b.position[k] - a.position[k])
                }),
                settled: still,
                uncertainty: gone,
                moving: !still,
            });
        }
        // After the last reading: carried on at the speed between the last two, if moving.
        let earlier = i.checked_sub(2).and_then(|j| self.samples.get(j));
        let (velocity, spacing) = match earlier {
            Some(e) if !a.idle => {
                let dt = seconds(e.ts, a.ts).max(f64::MIN_POSITIVE);
                (
                    std::array::from_fn(|k| (a.position[k] - e.position[k]) / dt),
                    dt,
                )
            }
            _ => ([0.0; N], 0.0),
        };
        let ahead = seconds(a.ts, t);
        Some(Estimate {
            position: std::array::from_fn(|k| a.position[k] + velocity[k] * ahead),
            settled: false,
            uncertainty: distance(&[0.0; N], &velocity) * spacing,
            moving: !a.idle,
        })
    }
}

/// The pose at a time, as the readings give it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PoseAt {
    /// The pose, if the stage is followed and has a reading. The focus is 0 if it isn't followed.
    pub pose: Option<StagePose>,
    /// How well it is known.
    pub quality: PoseQuality,
    /// Whether an axis was moving.
    pub moving: bool,
}

/// The stage's and focus drive's recent readings.
#[derive(Debug)]
pub struct PoseTimeline {
    stage: Mutex<Option<Track<3>>>,
    focus: Mutex<Option<Track<1>>>,
    capacity: usize,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn sample<P, const N: usize>(status: &AxisStatus<P>, position: [f64; N]) -> Sample<N> {
    let idle = status.state == MotionState::Idle;
    Sample {
        ts: status.ts,
        position,
        idle,
    }
}

impl PoseTimeline {
    /// A timeline that keeps the last `capacity` readings of each group, and follows neither yet.
    pub fn new(capacity: usize) -> Self {
        Self {
            stage: Mutex::new(None),
            focus: Mutex::new(None),
            capacity: capacity.max(2),
        }
    }

    /// Adds a stage reading, and follows the stage from now on.
    pub fn push_stage(&self, status: &StageStatus) {
        let p = status.position;
        let sample = sample(status, [p.x.0, p.y.0, p.z.0]);
        let capacity = self.capacity;
        let mut track = lock(&self.stage);
        track
            .get_or_insert_with(|| Track {
                samples: VecDeque::with_capacity(capacity),
            })
            .push(sample);
    }

    /// Adds a focus reading, and follows the focus drive from now on.
    pub fn push_focus(&self, status: &FocusStatus) {
        let sample = sample(status, [status.position.0]);
        let capacity = self.capacity;
        let mut track = lock(&self.focus);
        track
            .get_or_insert_with(|| Track {
                samples: VecDeque::with_capacity(capacity),
            })
            .push(sample);
    }

    /// Whether every group followed was still at its last reading.
    pub fn still(&self) -> bool {
        let stage = lock(&self.stage).as_ref().is_none_or(Track::still);
        stage && lock(&self.focus).as_ref().is_none_or(Track::still)
    }

    /// Whether every group followed has a reading after `t`, so that the pose at `t` won't change.
    pub fn decided(&self, t: Timestamp) -> bool {
        let stage = lock(&self.stage)
            .as_ref()
            .is_none_or(|track| track.newer_than(t));
        stage
            && lock(&self.focus)
                .as_ref()
                .is_none_or(|track| track.newer_than(t))
    }

    /// The pose at `t`.
    pub fn pose_at(&self, t: Timestamp) -> PoseAt {
        let stage = lock(&self.stage).as_ref().map(|track| track.at(t));
        let focus = lock(&self.focus).as_ref().map(|track| track.at(t));
        let unknown = PoseAt {
            pose: None,
            quality: PoseQuality::Unknown,
            moving: false,
        };
        let Some(Some(stage)) = stage else {
            return unknown;
        };
        let [x, y, z_coarse] = stage.position.map(Micrometers);
        let focus_at = focus.flatten();
        let pose = StagePose {
            x,
            y,
            z_coarse,
            focus: Micrometers(focus_at.map_or(0.0, |f| f.position[0])),
        };
        let quality = match focus {
            Some(None) => PoseQuality::Unknown,
            _ if stage.settled && focus_at.is_none_or(|f| f.settled) => PoseQuality::Settled,
            _ => PoseQuality::Interpolated {
                uncertainty: Micrometers(
                    stage
                        .uncertainty
                        .max(focus_at.map_or(0.0, |f| f.uncertainty)),
                ),
            },
        };
        PoseAt {
            pose: Some(pose),
            quality,
            moving: stage.moving || focus_at.is_some_and(|f| f.moving),
        }
    }
}
