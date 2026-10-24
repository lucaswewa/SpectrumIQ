//! Named counters, gauges and histograms, and their JSON snapshot.
//!
//! A metric is created, or fetched, once by name from the [`Registry`], and then updated through
//! its `Arc` with one atomic operation: no lock, no allocation. Names are lowercase,
//! dot-separated paths, such as `camera.frames_lost` or `preview.latency_ms`.
//!
//! Services share one `Arc<Registry>`, and `/api/siq/system/metrics` serves its
//! [`snapshot`](Registry::snapshot).

#[cfg(test)]
mod tests;

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::sync::{Arc, Mutex, PoisonError};

use schemars::JsonSchema;
use serde::Serialize;

/// A count that only goes up, such as frames acquired.
#[derive(Debug, Default)]
pub struct Counter(AtomicU64);

impl Counter {
    /// Adds one.
    pub fn increment(&self) {
        self.add(1);
    }

    /// Adds `n`.
    pub fn add(&self, n: u64) {
        self.0.fetch_add(n, Relaxed);
    }

    /// The count.
    pub fn get(&self) -> u64 {
        self.0.load(Relaxed)
    }
}

/// A value that goes up and down, such as free pool slots.
#[derive(Debug, Default)]
pub struct Gauge(AtomicU64);

impl Gauge {
    /// Sets the value.
    pub fn set(&self, value: f64) {
        self.0.store(value.to_bits(), Relaxed);
    }

    /// The value.
    pub fn get(&self) -> f64 {
        f64::from_bits(self.0.load(Relaxed))
    }
}

/// A distribution of values, such as latencies, counted into fixed buckets.
///
/// Bucket `i` counts the values up to `bounds[i]` (inclusive) that are above the bound before it.
/// A last, unbounded bucket counts the values above every bound.
#[derive(Debug)]
pub struct Histogram {
    bounds: Box<[f64]>,
    counts: Box<[AtomicU64]>,
    sum: AtomicU64,
}

impl Histogram {
    fn new(bounds: &[f64]) -> Result<Self, String> {
        let ascending = bounds.windows(2).all(|pair| pair[0] < pair[1]);
        if bounds.is_empty() || !ascending || !bounds.iter().all(|b| b.is_finite()) {
            return Err(format!(
                "bounds {bounds:?} are not finite and strictly ascending"
            ));
        }
        Ok(Self {
            bounds: bounds.into(),
            counts: (0..=bounds.len()).map(|_| AtomicU64::new(0)).collect(),
            sum: AtomicU64::new(0),
        })
    }

    /// Counts `value`. A NaN is ignored.
    pub fn record(&self, value: f64) {
        if value.is_nan() {
            return;
        }
        let bucket = self.bounds.partition_point(|bound| *bound < value);
        self.counts[bucket].fetch_add(1, Relaxed);
        // A compare-and-swap loop: `fetch_update`, deprecated since 1.99, would need a newer
        // minimum Rust than the workspace's.
        let mut current = self.sum.load(Relaxed);
        loop {
            let sum = (f64::from_bits(current) + value).to_bits();
            match self
                .sum
                .compare_exchange_weak(current, sum, Relaxed, Relaxed)
            {
                Ok(_) => break,
                Err(actual) => current = actual,
            }
        }
    }

    /// The number of values counted.
    pub fn count(&self) -> u64 {
        self.counts.iter().map(|count| count.load(Relaxed)).sum()
    }

    /// The sum of the values counted.
    pub fn sum(&self) -> f64 {
        f64::from_bits(self.sum.load(Relaxed))
    }

    fn buckets(&self) -> Vec<Bucket> {
        let bounds = self.bounds.iter().map(|bound| Some(*bound)).chain([None]);
        bounds
            .zip(&*self.counts)
            .map(|(le, count)| Bucket {
                le,
                count: count.load(Relaxed),
            })
            .collect()
    }
}

/// Why a metric couldn't be made.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MetricError {
    /// Names are lowercase letters, digits and `_`, in dot-separated parts.
    #[error(
        "{0:?} is not a metric name: use lowercase letters, digits and `_`, in parts joined by `.`"
    )]
    InvalidName(String),
    /// The name is taken by a metric of another kind, or a histogram with other bounds.
    #[error("the metric {name:?} already exists as a {existing}")]
    Conflict {
        /// The name.
        name: String,
        /// What it already is.
        existing: String,
    },
    /// A histogram's bounds must be finite and strictly ascending.
    #[error("the histogram {name:?}: {reason}")]
    InvalidBounds {
        /// The name.
        name: String,
        /// What is wrong with the bounds.
        reason: String,
    },
}

#[derive(Debug, Clone)]
enum Metric {
    Counter(Arc<Counter>),
    Gauge(Arc<Gauge>),
    Histogram(Arc<Histogram>),
}

impl Metric {
    fn describe(&self) -> String {
        match self {
            Metric::Counter(_) => "counter".to_owned(),
            Metric::Gauge(_) => "gauge".to_owned(),
            Metric::Histogram(h) => format!("histogram with bounds {:?}", h.bounds),
        }
    }
}

/// One bucket of a histogram's snapshot.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct Bucket {
    /// The bucket's upper bound, inclusive; `null` for the last, unbounded bucket.
    pub le: Option<f64>,
    /// The values in the bucket.
    pub count: u64,
}

/// A metric's value at the moment of a snapshot.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MetricValue {
    /// A counter.
    Counter {
        /// The count.
        value: u64,
    },
    /// A gauge.
    Gauge {
        /// The value.
        value: f64,
    },
    /// A histogram.
    Histogram {
        /// The values counted.
        count: u64,
        /// Their sum.
        sum: f64,
        /// The counts per bucket, lowest first.
        buckets: Vec<Bucket>,
    },
}

/// The metrics of a service, by name.
#[derive(Debug, Default)]
pub struct Registry {
    metrics: Mutex<BTreeMap<String, Metric>>,
}

impl Registry {
    /// An empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// The counter called `name`, made if it doesn't exist yet.
    pub fn counter(&self, name: &str) -> Result<Arc<Counter>, MetricError> {
        let new = || Metric::Counter(Arc::default());
        self.get_or_make(name, new, |metric| match metric {
            Metric::Counter(counter) => Some(Arc::clone(counter)),
            _ => None,
        })
    }

    /// The gauge called `name`, made if it doesn't exist yet.
    pub fn gauge(&self, name: &str) -> Result<Arc<Gauge>, MetricError> {
        let new = || Metric::Gauge(Arc::default());
        self.get_or_make(name, new, |metric| match metric {
            Metric::Gauge(gauge) => Some(Arc::clone(gauge)),
            _ => None,
        })
    }

    /// The histogram called `name`, made with `bounds` if it doesn't exist yet. If it exists, its
    /// bounds must be the same.
    pub fn histogram(&self, name: &str, bounds: &[f64]) -> Result<Arc<Histogram>, MetricError> {
        let histogram = Histogram::new(bounds).map_err(|reason| MetricError::InvalidBounds {
            name: name.to_owned(),
            reason,
        })?;
        let new = || Metric::Histogram(Arc::new(histogram));
        self.get_or_make(name, new, |metric| match metric {
            Metric::Histogram(h) if *h.bounds == *bounds => Some(Arc::clone(h)),
            _ => None,
        })
    }

    /// Every metric's value now, by name.
    pub fn snapshot(&self) -> BTreeMap<String, MetricValue> {
        let metrics = self.metrics.lock().unwrap_or_else(PoisonError::into_inner);
        let value = |metric: &Metric| match metric {
            Metric::Counter(counter) => MetricValue::Counter {
                value: counter.get(),
            },
            Metric::Gauge(gauge) => MetricValue::Gauge { value: gauge.get() },
            Metric::Histogram(h) => MetricValue::Histogram {
                count: h.count(),
                sum: h.sum(),
                buckets: h.buckets(),
            },
        };
        metrics
            .iter()
            .map(|(name, metric)| (name.clone(), value(metric)))
            .collect()
    }

    fn get_or_make<T>(
        &self,
        name: &str,
        new: impl FnOnce() -> Metric,
        pick: impl Fn(&Metric) -> Option<Arc<T>>,
    ) -> Result<Arc<T>, MetricError> {
        let valid_part = |part: &str| {
            let chars = |c: char| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_';
            !part.is_empty() && part.chars().all(chars)
        };
        if !name.split('.').all(valid_part) {
            return Err(MetricError::InvalidName(name.to_owned()));
        }
        let mut metrics = self.metrics.lock().unwrap_or_else(PoisonError::into_inner);
        let metric = metrics.entry(name.to_owned()).or_insert_with(new);
        pick(metric).ok_or_else(|| MetricError::Conflict {
            name: name.to_owned(),
            existing: metric.describe(),
        })
    }
}
