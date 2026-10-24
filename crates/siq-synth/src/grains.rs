//! Grains: a polished metal or ceramic under reflected light. Voronoi grains, each with its own
//! reflectance, dark boundaries between them, and a fine texture.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use siq_types::{Micrometers, Vec2Um};

use crate::noise::{Octave, cell, coverage, fractal, key, mix_values, octaves, units};
use crate::{World, WorldInfo};

/// The parameters of [`Grains`].
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct GrainsParams {
    /// The seed of every random choice.
    pub seed: u64,
    /// The mean distance between grain centres.
    pub size: Micrometers,
    /// The width of the dark boundary between grains.
    pub boundary: Micrometers,
}

impl Default for GrainsParams {
    fn default() -> Self {
        Self {
            seed: 0,
            size: Micrometers(60.0),
            boundary: Micrometers(1.5),
        }
    }
}

/// Voronoi grains around one seed point per grid square.
#[derive(Debug, Clone)]
pub struct Grains {
    params: GrainsParams,
    points: u64,
    texture: [Octave; 3],
}

impl Grains {
    /// Grains with `params`.
    pub fn new(params: GrainsParams) -> Self {
        Self {
            params,
            points: key(params.seed, 30),
            texture: octaves(params.seed, 40),
        }
    }

    /// The seed point of grid square `(i, j)`.
    fn point(&self, i: i64, j: i64) -> [f64; 2] {
        let [u, v] = units(self.points, i, j, 0);
        [
            (i as f64 + u) * self.params.size.0,
            (j as f64 + v) * self.params.size.0,
        ]
    }

    /// The reflectance of the grain of square `(i, j)`, from 0 to 1.
    fn reflectance(&self, i: i64, j: i64) -> f64 {
        let [reflectance] = units(self.points, i, j, 1);
        reflectance
    }
}

/// The seed nearest the point that `squared` measures from, and its distance.
fn nearest(
    seeds: &[([f64; 2], (i64, i64))],
    squared: impl Fn([f64; 2]) -> f64,
) -> (([f64; 2], (i64, i64)), f64) {
    let nearest = seeds
        .iter()
        .min_by(|a, b| squared(a.0).total_cmp(&squared(b.0)))
        .copied()
        .expect("seeds to search");
    (nearest, squared(nearest.0).sqrt())
}

impl World for Grains {
    fn intensity(&self, at: Vec2Um, footprint: Micrometers) -> f32 {
        let (p, f, [x, y]) = (&self.params, footprint.0, at.to_um());
        let size = p.size.0;
        let ((i, fx), (j, fy)) = (cell(x, size), cell(y, size));
        let squared = |[px, py]: [f64; 2]| (px - x).powi(2) + (py - y).powi(2);
        let seeds = |n: i64| {
            let side = 2 * n + 1;
            (0..side * side).map(move |m| (i + m / side - n, j + m % side - n))
        };
        let points: [([f64; 2], (i64, i64)); 9] = {
            let mut squares = seeds(1);
            std::array::from_fn(|_| {
                let square = squares.next().expect("nine squares");
                (self.point(square.0, square.1), square)
            })
        };
        // A boundary shows only within `reach`: the ramp of a footprint around a line `boundary`
        // wide.
        let reach = (p.boundary.0 + f) / 2.0;
        let (mut nearest, mut d1) = nearest(&points, squared);
        // A seed outside the 3 × 3 squares is at least `edge` away. When the nearest seed and its
        // boundaries could reach that far, the 5 × 5 squares are searched instead.
        let edge = size * (1.0 + fx.min(1.0 - fx).min(fy).min(1.0 - fy));
        let wide: Vec<_>;
        let candidates: &[([f64; 2], (i64, i64))] = if d1 + 2.0 * reach < edge {
            &points
        } else {
            wide = seeds(2).map(|(i, j)| (self.point(i, j), (i, j))).collect();
            (nearest, d1) = self::nearest(&wide, squared);
            &wide
        };
        // The boundary with another grain is their bisector, (d² − d₁²) / 2|p − p₁| away, and at
        // least (d − d₁) / 2, so seeds further than d₁ + 2 reach can't show one.
        let mut to_boundary = reach;
        for &(other, square) in candidates {
            let d_squared = squared(other);
            if square != nearest.1 && d_squared < (d1 + 2.0 * to_boundary).powi(2) {
                let gap = (other[0] - nearest.0[0]).hypot(other[1] - nearest.0[1]);
                to_boundary = to_boundary.min((d_squared - d1 * d1) / (2.0 * gap));
            }
        }
        let texture = fractal(&self.texture, at, 6.0, f) - 0.5;
        let grain = 0.35 + 0.5 * self.reflectance(nearest.1.0, nearest.1.1) + 0.08 * texture;
        mix_values(grain, 0.08, coverage(to_boundary - p.boundary.0 / 2.0, f)) as f32
    }

    fn surface_z(&self, _at: Vec2Um) -> Option<Micrometers> {
        Some(Micrometers::ZERO)
    }

    fn info(&self) -> WorldInfo {
        WorldInfo::Grains(self.params)
    }
}
