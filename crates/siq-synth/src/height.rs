//! A height field: a textured relief whose height is known everywhere, for depth from focus.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use siq_types::{Micrometers, Vec2Um};

use crate::noise::{Octave, fractal, octaves};
use crate::{World, WorldInfo};

/// The parameters of [`HeightField`].
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct HeightFieldParams {
    /// The seed of every random choice.
    pub seed: u64,
    /// The relief's height runs from minus to plus this.
    pub amplitude: Micrometers,
    /// The distance between hills.
    pub period: Micrometers,
    /// The distance between features of the surface texture, which gives focus something to
    /// find.
    pub texture: Micrometers,
}

impl Default for HeightFieldParams {
    fn default() -> Self {
        Self {
            seed: 0,
            amplitude: Micrometers(40.0),
            period: Micrometers(400.0),
            texture: Micrometers(5.0),
        }
    }
}

/// A smooth relief of fractal noise, with a fine texture on it.
#[derive(Debug, Clone)]
pub struct HeightField {
    params: HeightFieldParams,
    relief: [Octave; 4],
    texture: [Octave; 4],
}

impl HeightField {
    /// A height field with `params`.
    pub fn new(params: HeightFieldParams) -> Self {
        Self {
            params,
            relief: octaves(params.seed, 50),
            texture: octaves(params.seed, 60),
        }
    }
}

impl World for HeightField {
    fn intensity(&self, at: Vec2Um, footprint: Micrometers) -> f32 {
        let p = &self.params;
        (0.15 + 0.7 * fractal(&self.texture, at, p.texture.0, footprint.0)) as f32
    }

    /// The height is a property of the sample, so it doesn't depend on the sampling.
    fn surface_z(&self, at: Vec2Um) -> Option<Micrometers> {
        let p = &self.params;
        let relief = fractal(&self.relief, at, p.period.0, 0.0);
        Some(p.amplitude * (2.0 * relief - 1.0))
    }

    fn info(&self) -> WorldInfo {
        WorldInfo::HeightField(self.params)
    }
}
