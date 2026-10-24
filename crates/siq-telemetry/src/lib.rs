//! SpectrumIQ's logging and metrics.
//!
//! - [`init`] sets up `tracing` once per process: the console, daily rolling files, and a filter.
//! - [`metrics`] is a registry of named counters, gauges and histograms. They update with one
//!   atomic operation, so the frame path can use them, and the registry snapshots them as JSON.

pub mod metrics;

use std::path::PathBuf;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, fmt};

pub use metrics::{Counter, Gauge, Histogram, MetricError, MetricValue, Registry};

/// Where logs go: the `[log]` section of the instrument configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct LogConfig {
    /// What to log, in `RUST_LOG` syntax, such as `"info"` or `"info,siq_grbl=debug"`. The
    /// `RUST_LOG` environment variable overrides it.
    pub filter: String,
    /// Whether to log to the console (standard error).
    pub console: bool,
    /// The folder for the daily log files, `spectrumiq.YYYY-MM-DD.log`. Without one, nothing is
    /// written to files.
    pub directory: Option<PathBuf>,
    /// How many daily files to keep.
    pub keep_days: u16,
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            filter: "info".to_owned(),
            console: true,
            directory: None,
            keep_days: 30,
        }
    }
}

/// Why logging couldn't be set up.
#[derive(Debug, thiserror::Error)]
pub enum TelemetryError {
    /// The filter isn't valid `RUST_LOG` syntax.
    #[error("the log filter {filter:?} is not valid: {source}")]
    Filter {
        /// The filter.
        filter: String,
        /// The parser's complaint.
        source: tracing_subscriber::filter::ParseError,
    },
    /// The log folder can't be made.
    #[error("the log folder {path:?} can't be made: {source}")]
    Folder {
        /// The folder.
        path: PathBuf,
        /// The system's complaint.
        source: std::io::Error,
    },
    /// The log files can't be opened.
    #[error("the log files can't be opened: {0}")]
    Files(#[from] tracing_appender::rolling::InitError),
    /// Logging was already set up in this process.
    #[error("logging is already set up: {0}")]
    AlreadySet(#[from] tracing_subscriber::util::TryInitError),
}

/// Keeps the file logger running. Dropping it flushes the files, so a service holds it until it
/// exits.
#[must_use = "dropping the guard stops writing log files"]
#[derive(Debug)]
pub struct LogGuard(#[allow(dead_code)] Option<WorkerGuard>);

/// Sets up logging for the process, as `config` says. It can be called only once per process.
pub fn init(config: &LogConfig) -> Result<LogGuard, TelemetryError> {
    let filter = match EnvFilter::try_from_default_env() {
        Ok(from_env) => from_env,
        Err(_) => EnvFilter::try_new(&config.filter).map_err(|source| TelemetryError::Filter {
            filter: config.filter.clone(),
            source,
        })?,
    };
    let console = config
        .console
        .then(|| fmt::layer().with_writer(std::io::stderr));
    let (files, guard) = match &config.directory {
        Some(directory) => {
            // Made first: the appender prunes old files before it would make the folder.
            std::fs::create_dir_all(directory).map_err(|source| TelemetryError::Folder {
                path: directory.clone(),
                source,
            })?;
            let appender = RollingFileAppender::builder()
                .rotation(Rotation::DAILY)
                .filename_prefix("spectrumiq")
                .filename_suffix("log")
                .max_log_files(usize::from(config.keep_days.max(1)))
                .build(directory)?;
            let (writer, guard) = tracing_appender::non_blocking(appender);
            let layer = fmt::layer().with_ansi(false).with_writer(writer);
            (Some(layer), Some(guard))
        }
        None => (None, None),
    };
    tracing_subscriber::registry()
        .with(filter)
        .with(console)
        .with(files)
        .try_init()?;
    Ok(LogGuard(guard))
}
