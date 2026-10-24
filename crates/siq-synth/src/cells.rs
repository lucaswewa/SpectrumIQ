//! Cells: elliptical cells with nuclei and texture, on a faintly uneven background. For
//! transmitted light and fluorescence.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use siq_types::{Micrometers, Vec2Um};

use crate::noise::{Octave, cells_within, coverage, fractal, key, mix_values, octaves, units};
use crate::{World, WorldInfo};

/// The parameters of [`Cells`].
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct CellsParams {
    /// The seed of every random choice.
    pub seed: u64,
    /// The spacing of the grid that places the cells: at most one per square.
    pub spacing: Micrometers,
    /// The share of grid squares that hold a cell, from 0 to 1.
    pub density: f64,
    /// The shortest and the longest semi-axis.
    pub radius: [Micrometers; 2],
}

impl Default for CellsParams {
    fn default() -> Self {
        Self {
            seed: 0,
            spacing: Micrometers(40.0),
            density: 0.6,
            radius: [Micrometers(8.0), Micrometers(16.0)],
        }
    }
}

/// An ellipse, with a signed distance that is negative inside.
struct Ellipse {
    centre: [f64; 2],
    axes: [f64; 2],
    rotation: [f64; 2],
}

impl Ellipse {
    /// About the distance from the outline: exact along the short axis, shorter elsewhere.
    fn distance(&self, [x, y]: [f64; 2]) -> f64 {
        let ([dx, dy], [cos, sin]) = ([x - self.centre[0], y - self.centre[1]], self.rotation);
        let (u, v) = (dx * cos + dy * sin, dy * cos - dx * sin);
        let rho = ((u / self.axes[0]).powi(2) + (v / self.axes[1]).powi(2)).sqrt();
        (rho - 1.0) * self.axes[0].min(self.axes[1])
    }
}

/// Cells, placed at most one per grid square, each with a nucleus.
#[derive(Debug, Clone)]
pub struct Cells {
    params: CellsParams,
    places: u64,
    shapes: u64,
    background: [Octave; 3],
    texture: [Octave; 3],
}

struct Cell {
    outline: Ellipse,
    nucleus: Ellipse,
    tone: f64,
}

impl Cells {
    /// Cells with `params`.
    pub fn new(params: CellsParams) -> Self {
        let seed = params.seed;
        Self {
            params,
            places: key(seed, 1),
            shapes: key(seed, 2),
            background: octaves(seed, 10),
            texture: octaves(seed, 20),
        }
    }

    /// The cell of grid square `(i, j)`, if it has one whose centre is within `reach` of
    /// `point`.
    fn cell(&self, i: i64, j: i64, point: [f64; 2], reach: f64) -> Option<Cell> {
        let p = &self.params;
        let [present, cx, cy] = units(self.places, i, j, 0);
        let centre = [(i as f64 + cx) * p.spacing.0, (j as f64 + cy) * p.spacing.0];
        let far = (centre[0] - point[0]).powi(2) + (centre[1] - point[1]).powi(2) > reach * reach;
        if present >= p.density || far {
            return None;
        }
        let [a, b, angle, size, ox, oy, tone] = units(self.shapes, i, j, 0);
        let [short, long] = p.radius.map(|r| r.0);
        let axes = [short + (long - short) * a, short + (long - short) * b];
        let (sin, cos) = (angle * std::f64::consts::PI).sin_cos();
        let shift = [(ox - 0.5) * 0.4 * axes[0], (oy - 0.5) * 0.4 * axes[1]];
        let nucleus = Ellipse {
            centre: [
                centre[0] + shift[0] * cos - shift[1] * sin,
                centre[1] + shift[0] * sin + shift[1] * cos,
            ],
            axes: axes.map(|axis| axis * (0.3 + 0.15 * size)),
            rotation: [cos, sin],
        };
        let outline = Ellipse {
            centre,
            axes,
            rotation: [cos, sin],
        };
        Some(Cell {
            outline,
            nucleus,
            tone,
        })
    }
}

impl World for Cells {
    fn intensity(&self, at: Vec2Um, footprint: Micrometers) -> f32 {
        let (p, f, point) = (&self.params, footprint.0, at.to_um());
        let background = 0.85 + 0.1 * (fractal(&self.background, at, 200.0, f) - 0.5);
        // A cell reaches no further than its longest semi-axis, and its edge half a footprint.
        let reach = p.radius[0].0.max(p.radius[1].0) + f;
        let mut nearest: Option<(f64, Cell)> = None;
        for i in cells_within(point[0], reach, p.spacing.0) {
            for j in cells_within(point[1], reach, p.spacing.0) {
                let Some(cell) = self.cell(i, j, point, reach) else {
                    continue;
                };
                let distance = cell.outline.distance(point);
                if nearest.as_ref().is_none_or(|(best, _)| distance < *best) {
                    nearest = Some((distance, cell));
                }
            }
        }
        let Some((distance, cell)) = nearest else {
            return background as f32;
        };
        let texture = fractal(&self.texture, at, 4.0, f) - 0.5;
        let cytoplasm = 0.5 + 0.2 * cell.tone + 0.15 * texture;
        let nucleus = 0.2 + 0.1 * cell.tone + 0.2 * texture;
        let inside = mix_values(
            cytoplasm,
            nucleus,
            coverage(cell.nucleus.distance(point), f),
        );
        mix_values(background, inside, coverage(distance, f)) as f32
    }

    fn surface_z(&self, _at: Vec2Um) -> Option<Micrometers> {
        Some(Micrometers::ZERO)
    }

    fn info(&self) -> WorldInfo {
        WorldInfo::Cells(self.params)
    }
}
