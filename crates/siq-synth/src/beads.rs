//! Beads: sub-resolution fluorescent beads scattered through a slab, for deconvolution and light
//! field. Each is a 3D Gaussian, and the list of them is the ground truth.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use siq_types::{Micrometers, RectUm, Vec2Um};

use crate::noise::{cell, cells_within, key, units};
use crate::{Bead, World, WorldInfo};

/// The parameters of [`Beads`].
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct BeadsParams {
    /// The seed of every random choice.
    pub seed: u64,
    /// The spacing of the grid that places the beads: at most one per cube.
    pub spacing: Micrometers,
    /// The share of grid cubes that hold a bead, from 0 to 1.
    pub density: f64,
    /// The slab's lowest z.
    pub z_min: Micrometers,
    /// The slab's highest z.
    pub z_max: Micrometers,
    /// Each bead's standard deviation.
    pub sigma: Micrometers,
}

impl Default for BeadsParams {
    fn default() -> Self {
        Self {
            seed: 0,
            spacing: Micrometers(15.0),
            density: 0.5,
            z_min: Micrometers(-20.0),
            z_max: Micrometers(20.0),
            sigma: Micrometers(0.1),
        }
    }
}

/// Beads, at most one per grid cube, within a slab.
#[derive(Debug, Clone)]
pub struct Beads {
    params: BeadsParams,
    places: u64,
}

impl Beads {
    /// Beads with `params`.
    pub fn new(params: BeadsParams) -> Self {
        let places = key(params.seed, 70);
        Self { params, places }
    }

    /// The bead of grid cube `(i, j, k)`, if it has one inside the slab.
    fn bead(&self, i: i64, j: i64, k: i64) -> Option<Bead> {
        let p = &self.params;
        let [present, u, v, w, brightness] = units(self.places, i, j, k);
        let s = p.spacing.0;
        let z = Micrometers((k as f64 + w) * s);
        (present < p.density && p.z_min <= z && z <= p.z_max).then_some(Bead {
            x: Micrometers((i as f64 + u) * s),
            y: Micrometers((j as f64 + v) * s),
            z,
            sigma: p.sigma,
            brightness: (0.5 + 0.5 * brightness) as f32,
        })
    }

    /// The beads in the columns of cubes `i` × `j`, through every layer of the slab.
    fn cubes(
        &self,
        i: impl Iterator<Item = i64> + Clone,
        j: impl Iterator<Item = i64> + Clone,
    ) -> impl Iterator<Item = Bead> {
        let s = self.params.spacing.0;
        let layers = cell(self.params.z_min.0, s).0..=cell(self.params.z_max.0, s).0;
        i.flat_map(move |i| j.clone().map(move |j| (i, j)))
            .flat_map(move |(i, j)| layers.clone().filter_map(move |k| self.bead(i, j, k)))
    }

    /// The beads whose centres are within `reach` of `at` in x and y. Beyond five standard
    /// deviations, a bead adds under 4·10⁻⁶ of its brightness.
    fn near(&self, at: Vec2Um, reach: f64) -> impl Iterator<Item = Bead> {
        let (s, [x, y]) = (self.params.spacing.0, at.to_um());
        let (i, j) = (cells_within(x, reach, s), cells_within(y, reach, s));
        let close =
            move |bead: &Bead| (bead.x.0 - x).powi(2) + (bead.y.0 - y).powi(2) <= reach * reach;
        self.cubes(i, j).filter(close)
    }
}

impl World for Beads {
    /// The beads seen from above, all in focus: each a 2D Gaussian, widened by the pixel's
    /// footprint, with its total kept.
    fn intensity(&self, at: Vec2Um, footprint: Micrometers) -> f32 {
        let [x, y] = at.to_um();
        let sigma2 = self.params.sigma.0.powi(2);
        let widened = sigma2 + footprint.0.powi(2) / 12.0;
        let sum: f64 = self
            .near(at, 5.0 * widened.sqrt())
            .map(|bead| {
                let r2 = (bead.x.0 - x).powi(2) + (bead.y.0 - y).powi(2);
                f64::from(bead.brightness) * sigma2 / widened * (-r2 / (2.0 * widened)).exp()
            })
            .sum();
        sum.min(1.0) as f32
    }

    fn density(&self, at: Vec2Um, z: Micrometers) -> Option<f32> {
        let [x, y] = at.to_um();
        let two_sigma2 = 2.0 * self.params.sigma.0.powi(2);
        let sum: f64 = self
            .near(at, 5.0 * self.params.sigma.0)
            .map(|bead| {
                let r2 = (bead.x.0 - x).powi(2) + (bead.y.0 - y).powi(2) + (bead.z.0 - z.0).powi(2);
                f64::from(bead.brightness) * (-r2 / two_sigma2).exp()
            })
            .sum();
        Some(sum as f32)
    }

    fn beads(&self, region: RectUm) -> Vec<Bead> {
        let s = self.params.spacing.0;
        let [x0, y0] = region.origin.to_um();
        let [x1, y1] = (region.origin + region.size).to_um();
        let (i, j) = (cell(x0, s).0..=cell(x1, s).0, cell(y0, s).0..=cell(y1, s).0);
        let inside = |bead: &Bead| (x0..x1).contains(&bead.x.0) && (y0..y1).contains(&bead.y.0);
        self.cubes(i, j).filter(inside).collect()
    }

    fn info(&self) -> WorldInfo {
        WorldInfo::Beads(self.params)
    }
}
