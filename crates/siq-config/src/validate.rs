//! The rules a configuration must follow beyond its types: ranges, uniqueness, and references
//! between sections. Every broken rule is collected, with the path of the value it is about; the
//! message is written to follow that path.

use std::fmt;

use crate::model::{
    CameraConfig, FocusConfig, InstrumentConfig, PeripheralsConfig, StageConfig, WorldConfig,
};

/// One step of a path into the configuration: a key, or an index into an array.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Seg {
    Key(&'static str),
    Index(usize),
}

impl From<&'static str> for Seg {
    fn from(key: &'static str) -> Self {
        Seg::Key(key)
    }
}

impl From<usize> for Seg {
    fn from(index: usize) -> Self {
        Seg::Index(index)
    }
}

/// A path such as `channels[1].id`.
pub(crate) struct PathDisplay<'a>(pub &'a [Seg]);

impl fmt::Display for PathDisplay<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, seg) in self.0.iter().enumerate() {
            match seg {
                Seg::Key(key) if i == 0 => write!(f, "{key}")?,
                Seg::Key(key) => write!(f, ".{key}")?,
                Seg::Index(index) => write!(f, "[{index}]")?,
            }
        }
        Ok(())
    }
}

macro_rules! at {
    ($($seg:expr),*) => { vec![$(Seg::from($seg)),*] };
}

/// The broken rules, with the paths they are about.
#[derive(Debug, Default)]
pub(crate) struct Problems(pub Vec<(Vec<Seg>, String)>);

impl Problems {
    fn check(&mut self, ok: bool, path: Vec<Seg>, message: impl Into<String>) {
        if !ok {
            self.0.push((path, message.into()));
        }
    }

    /// Reports each item equal to an earlier one, at `path(index)`.
    fn unique<T: PartialEq>(&mut self, items: &[T], path: impl Fn(usize) -> Vec<Seg>) {
        for i in 0..items.len() {
            if let Some(first) = items[..i].iter().position(|item| *item == items[i]) {
                let message = format!("{} has it too", PathDisplay(&path(first)));
                self.0.push((path(i), message));
            }
        }
    }
}

fn positive(value: f64) -> bool {
    value.is_finite() && value > 0.0
}

fn not_negative(value: f64) -> bool {
    value.is_finite() && value >= 0.0
}

/// Every broken rule of `config`.
pub(crate) fn validate(config: &InstrumentConfig) -> Problems {
    let mut p = Problems::default();
    let memory = &config.memory;
    p.check(config.server.port != 0, at!["server", "port"], "can't be 0");
    let slots = memory.pool_slots;
    let message = format!("must be 2 to 64 frames, not {slots}");
    p.check(
        (2..=64).contains(&slots),
        at!["memory", "pool_slots"],
        message,
    );
    p.check(
        positive(memory.job_ram_gb),
        at!["memory", "job_ram_gb"],
        "must be positive",
    );
    p.check(
        not_negative(memory.job_vram_gb),
        at!["memory", "job_vram_gb"],
        "can't be negative",
    );
    p.check(
        memory.tile_cache_mb > 0,
        at!["memory", "tile_cache_mb"],
        "must be positive",
    );
    devices(&mut p, config);
    objectives(&mut p, config);
    channels(&mut p, config);
    peripherals(&mut p, &config.peripherals);
    if let WorldConfig::Image { path, pixel_size } = &config.sim.world {
        let named = !path.as_os_str().is_empty();
        p.check(
            named,
            at!["sim", "world", "path"],
            "must name the image file",
        );
        p.check(
            positive(pixel_size.0),
            at!["sim", "world", "pixel_size"],
            "must be positive",
        );
    }
    let errors = &config.sim.errors;
    let sizes = [
        ("backlash", errors.backlash.0),
        ("scale_error", errors.scale_error),
        ("position_noise", errors.position_noise.0),
        ("reply_latency", errors.reply_latency.0),
    ];
    for (key, value) in sizes {
        p.check(
            not_negative(value),
            at!["sim", "errors", key],
            "can't be negative",
        );
    }
    p
}

fn devices(p: &mut Problems, config: &InstrumentConfig) {
    if let CameraConfig::Sim(camera) = &config.camera {
        let size = camera.width > 0 && camera.height > 0;
        p.check(size, at!["camera"], "the sensor needs a width and a height");
        p.check(
            positive(camera.frame_rate.0),
            at!["camera", "frame_rate"],
            "must be positive",
        );
        p.check(
            positive(camera.pixel_size.0),
            at!["camera", "pixel_size"],
            "must be positive",
        );
    }
    let (poll_hz, motion) = match &config.stage {
        StageConfig::Sim(stage) => (stage.poll_hz, &stage.motion),
        StageConfig::Grbl(stage) => {
            let named = !stage.port.is_empty();
            p.check(
                named,
                at!["stage", "port"],
                "must name the serial port, such as \"COM5\"",
            );
            p.check(stage.baud > 0, at!["stage", "baud"], "can't be 0");
            (stage.poll_hz, &stage.motion)
        }
    };
    let polling = poll_hz.0 > 0.0 && poll_hz.0 <= 100.0;
    let message = format!("must be above 0 and at most 100 Hz, not {poll_hz}");
    p.check(polling, at!["stage", "poll_hz"], message);
    let sides = motion.approach.iter().all(|side| side.abs() == 1);
    let message = format!("must be 1 or -1 for each axis, not {:?}", motion.approach);
    p.check(sides, at!["stage", "motion", "approach"], message);
    p.check(
        not_negative(motion.overshoot.0),
        at!["stage", "motion", "overshoot"],
        "must be 0 or more",
    );
    if let FocusConfig::Ascii(focus) = &config.focus {
        let named = !focus.port.is_empty();
        p.check(
            named,
            at!["focus", "port"],
            "must name the serial port, such as \"COM6\"",
        );
        p.check(focus.baud > 0, at!["focus", "baud"], "can't be 0");
        let dialect = !focus.dialect.as_os_str().is_empty();
        p.check(
            dialect,
            at!["focus", "dialect"],
            "must name the protocol's dialect file",
        );
    }
}

fn objectives(p: &mut Problems, config: &InstrumentConfig) {
    let objectives = &config.objectives;
    p.check(
        !objectives.is_empty(),
        at!["objectives"],
        "needs at least one [[objectives]]",
    );
    let ids: Vec<_> = objectives.iter().map(|o| o.id).collect();
    p.unique(&ids, |i| at!["objectives", i, "id"]);
    let names: Vec<_> = objectives.iter().map(|o| &o.name).collect();
    p.unique(&names, |i| at!["objectives", i, "name"]);
    let positions: Vec<_> = objectives
        .iter()
        .filter_map(|o| o.turret_position)
        .collect();
    let turret = config.peripherals.objective_turret.as_ref();
    for (i, objective) in objectives.iter().enumerate() {
        let magnification = positive(objective.magnification);
        p.check(
            magnification,
            at!["objectives", i, "magnification"],
            "must be positive",
        );
        let na = objective.numerical_aperture;
        let message = format!("must be above 0 and at most 1.7, not {na}");
        p.check(
            na > 0.0 && na <= 1.7,
            at!["objectives", i, "numerical_aperture"],
            message,
        );
        let Some(position) = objective.turret_position else {
            continue;
        };
        let path = at!["objectives", i, "turret_position"];
        match turret {
            None => p.check(false, path, "there is no [peripherals.objective_turret]"),
            Some(turret) => {
                let last = turret.positions.saturating_sub(1);
                let message = format!("must be a turret position, 0 to {last}");
                p.check(position < turret.positions, path, message);
            }
        }
    }
    let on_turret: Vec<usize> = (0..objectives.len())
        .filter(|&i| objectives[i].turret_position.is_some())
        .collect();
    p.unique(&positions, |n| {
        at!["objectives", on_turret[n], "turret_position"]
    });
}

fn channels(p: &mut Problems, config: &InstrumentConfig) {
    let channels = &config.channels;
    p.check(
        !channels.is_empty(),
        at!["channels"],
        "needs at least one [[channels]]",
    );
    let ids: Vec<_> = channels.iter().map(|c| c.id).collect();
    p.unique(&ids, |i| at!["channels", i, "id"]);
    let names: Vec<_> = channels.iter().map(|c| &c.name).collect();
    p.unique(&names, |i| at!["channels", i, "name"]);
    let peripherals = &config.peripherals;
    let sources = peripherals.illumination.as_ref().map(|il| &il.sources);
    let filters = peripherals
        .filter_wheel
        .as_ref()
        .map(|wheel| &wheel.filters);
    for (i, channel) in channels.iter().enumerate() {
        p.check(
            positive(channel.exposure.0),
            at!["channels", i, "exposure"],
            "must be positive",
        );
        if let Some(light) = &channel.illumination {
            let (source, intensity) = (&light.source, light.intensity);
            let message = match sources {
                None => "there is no [peripherals.illumination]".to_owned(),
                Some(sources) => format!("{source:?} is not one of the sources {sources:?}"),
            };
            let known = sources.is_some_and(|sources| sources.contains(source));
            p.check(known, at!["channels", i, "illumination", "source"], message);
            let message = format!("must be from 0 to 1, not {intensity}");
            let in_range = (0.0..=1.0).contains(&intensity);
            p.check(
                in_range,
                at!["channels", i, "illumination", "intensity"],
                message,
            );
        }
        if let Some(filter) = &channel.filter {
            let message = match filters {
                None => "there is no [peripherals.filter_wheel]".to_owned(),
                Some(filters) => format!("{filter:?} is not one of the filters {filters:?}"),
            };
            let known = filters.is_some_and(|filters| filters.contains(filter));
            p.check(known, at!["channels", i, "filter"], message);
        }
    }
}

fn peripherals(p: &mut Problems, peripherals: &PeripheralsConfig) {
    if let Some(illumination) = &peripherals.illumination {
        let sources = &illumination.sources;
        p.check(
            !sources.is_empty(),
            at!["peripherals", "illumination", "sources"],
            "is empty",
        );
        p.unique(sources, |i| {
            at!["peripherals", "illumination", "sources", i]
        });
    }
    if let Some(wheel) = &peripherals.filter_wheel {
        let filters = &wheel.filters;
        p.check(
            !filters.is_empty(),
            at!["peripherals", "filter_wheel", "filters"],
            "is empty",
        );
        p.unique(filters, |i| {
            at!["peripherals", "filter_wheel", "filters", i]
        });
    }
    if let Some(turret) = &peripherals.objective_turret {
        let path = at!["peripherals", "objective_turret", "positions"];
        p.check(turret.positions > 0, path, "must be at least 1");
    }
    if let Some(modality) = &peripherals.modality {
        let modes = !modality.modes.is_empty();
        p.check(
            modes,
            at!["peripherals", "modality", "modes"],
            "needs at least one mode",
        );
    }
}
