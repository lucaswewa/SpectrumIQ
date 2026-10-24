//! SpectrumIQ's instrument configuration: one TOML file describes the instrument (ADR-0020).
//!
//! [`load`] reads and checks a file, and reports **every** problem with its line and column:
//!
//! ```text
//! config/sim.toml:41:6: channels[1].id: channel id 1 is channels[0]'s too
//! config/sim.toml:12:1: unknown field `prot`, expected one of `bind`, `port`, …
//! ```
//!
//! Missing sections and keys take their defaults. Relative paths are resolved against the
//! configuration file's folder. [`json_schema`] describes the format for editors and the service's
//! `--print-config-schema`.

pub mod model;
mod validate;

use std::fmt;
use std::ops::Range;
use std::path::{Path, PathBuf};

use toml::Spanned;
use toml::de::{DeTable, DeValue};

pub use model::*;
use validate::{PathDisplay, Seg};

/// One thing wrong with a configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    /// The line and column, from 1, if the problem has a place in the file.
    pub location: Option<(usize, usize)>,
    /// The path of the value, such as `channels[1].id`; empty for the file as a whole.
    pub path: String,
    /// What is wrong.
    pub message: String,
}

/// Why a configuration was rejected: every problem found, one per line when displayed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigError {
    /// Where the configuration came from, such as its file name.
    pub origin: String,
    /// The problems, in the order found.
    pub problems: Vec<Problem>,
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, problem) in self.problems.iter().enumerate() {
            if i > 0 {
                writeln!(f)?;
            }
            write!(f, "{}", self.origin)?;
            if let Some((line, column)) = problem.location {
                write!(f, ":{line}:{column}")?;
            }
            if !problem.path.is_empty() {
                write!(f, ": {}", problem.path)?;
            }
            write!(f, ": {}", problem.message)?;
        }
        Ok(())
    }
}

impl std::error::Error for ConfigError {}

/// Reads, checks and resolves the configuration file at `path`.
pub fn load(path: &Path) -> Result<InstrumentConfig, ConfigError> {
    let origin = path.display().to_string();
    let text = std::fs::read_to_string(path).map_err(|error| ConfigError {
        origin: origin.clone(),
        problems: vec![Problem {
            location: None,
            path: String::new(),
            message: format!("can't be read: {error}"),
        }],
    })?;
    from_toml(&text, &origin, path.parent().unwrap_or(Path::new("")))
}

/// Parses and checks a configuration, then resolves its relative paths against `base`.
/// `origin` names it in messages.
pub fn from_toml(text: &str, origin: &str, base: &Path) -> Result<InstrumentConfig, ConfigError> {
    let fail = |problems| ConfigError {
        origin: origin.to_owned(),
        problems,
    };
    let mut config: InstrumentConfig = toml::from_str(text).map_err(|error| {
        fail(vec![Problem {
            location: error.span().map(|span| location(text, span.start)),
            path: String::new(),
            message: error.message().trim().to_owned(),
        }])
    })?;
    let problems = validate::validate(&config).0;
    if !problems.is_empty() {
        let document = DeTable::parse(text).ok();
        let located = |(path, message): (Vec<Seg>, String)| Problem {
            location: document
                .as_ref()
                .map(|document| location(text, span_of(document, &path).start)),
            path: PathDisplay(&path).to_string(),
            message,
        };
        let mut problems: Vec<_> = problems.into_iter().map(located).collect();
        problems.sort_by_key(|problem| problem.location);
        return Err(fail(problems));
    }
    resolve_paths(&mut config, base);
    Ok(config)
}

/// The JSON Schema of the configuration file.
pub fn json_schema() -> serde_json::Value {
    serde_json::to_value(schemars::schema_for!(InstrumentConfig)).expect("a schema is plain JSON")
}

/// Makes every relative path relative to `base` instead.
fn resolve_paths(config: &mut InstrumentConfig, base: &Path) {
    let resolve = |path: &mut PathBuf| {
        if path.is_relative() && !path.as_os_str().is_empty() {
            *path = base.join(&*path);
        }
    };
    let optional = match &mut config.camera {
        CameraConfig::Euresys(camera) => camera.features.as_mut(),
        CameraConfig::Sim(_) => None,
    };
    let dialect = match &mut config.focus {
        FocusConfig::Sim(focus) => focus.dialect.as_mut(),
        FocusConfig::Ascii(focus) => Some(&mut focus.dialect),
    };
    let world = match &mut config.sim.world {
        WorldConfig::Image { path, .. } => Some(path),
        _ => None,
    };
    let paths = [&mut config.server.data_root, &mut config.server.scratch];
    let optional = [config.log.directory.as_mut(), optional, dialect, world];
    paths
        .into_iter()
        .chain(optional.into_iter().flatten())
        .for_each(resolve);
}

/// The span of the value at `path`, or of the deepest part of the path that exists.
fn span_of(document: &Spanned<DeTable<'_>>, path: &[Seg]) -> Range<usize> {
    let mut span = document.span();
    let mut value: Option<&DeValue<'_>> = None;
    for seg in path {
        let table = match value {
            None => Some(document.get_ref()),
            Some(DeValue::Table(table)) => Some(table),
            _ => None,
        };
        let next = match (seg, table, value) {
            (Seg::Key(key), Some(table), _) => table
                .iter()
                .find(|(name, _)| name.get_ref().as_ref() == *key)
                .map(|(_, value)| value),
            (Seg::Index(index), _, Some(DeValue::Array(array))) => array.get(*index),
            _ => None,
        };
        let Some(next) = next else { break };
        span = next.span();
        value = Some(next.get_ref());
    }
    span
}

/// The line and column, from 1, of the byte `offset` of `text`.
fn location(text: &str, offset: usize) -> (usize, usize) {
    let before = &text[..offset.min(text.len())];
    let line = before.matches('\n').count() + 1;
    let column = before.rsplit('\n').next().unwrap_or("").chars().count() + 1;
    (line, column)
}
