//! A stitch's quality report (ADR-0064): where each tile of the reference plane went and how far it
//! moved, each pair's registration against the solution, and the solution's measures. Written
//! beside the mosaic as `qa/<image>.json`, it is what the viewer's overlays draw, and its summary
//! goes into the dataset's history.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::composite::Bounds;
use crate::solve::{Method, Solution};
use crate::{Edge, Mosaic};

/// A tile, as stitched.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TileQa {
    /// Its stage position in the scan.
    pub p: u32,
    /// Its corner in the image, in pixels.
    pub corner: [f64; 2],
    /// Its width and height.
    pub size: [u32; 2],
    /// How far the solution moved it from where the stage said, in pixels.
    pub moved_px: f64,
    /// The largest residual of its edges used, in pixels.
    pub worst_px: f64,
    /// Its connected component: 0 is the largest.
    pub component: usize,
}

/// A pair, as registered and solved.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct EdgeQa {
    /// The first tile, by its index in `tiles`.
    pub i: usize,
    /// The second.
    pub j: usize,
    /// How far the solution is from the registration, in pixels.
    pub residual_px: f64,
    /// The registration's energy and correlation.
    pub pce: f64,
    /// Its correlation.
    pub ncc: f64,
    /// Whether the registration was trusted.
    pub confident: bool,
    /// Whether the solve used it: trusted, and not rejected.
    pub used: bool,
}

/// What a stitch did, and how well.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Report {
    /// The image: its size, and the mosaic pixel at its first pixel.
    pub bounds: Bounds,
    /// Each tile of the reference plane.
    pub tiles: Vec<TileQa>,
    /// Each pair registered.
    pub edges: Vec<EdgeQa>,
    /// The measures, for the dataset's history.
    pub summary: Summary,
}

/// A stitch's measures.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Summary {
    /// The tiles of the reference plane.
    pub tiles: usize,
    /// The pairs registered.
    pub pairs: usize,
    /// Those trusted.
    pub trusted: usize,
    /// Those trusted that the solve rejected.
    pub rejected: usize,
    /// The connected components of the edges used.
    pub components: usize,
    /// How it was solved.
    pub method: Method,
    /// The edges used against the solution: their residuals' root mean square, in pixels.
    pub rms_px: f64,
    /// Their largest.
    pub max_px: f64,
    /// The most a tile moved from where the stage said, in pixels.
    pub moved_px: f64,
}

impl Report {
    /// The report of `mosaic`'s tiles at `solution`, their `edges`, in an image of `bounds`.
    pub fn new(mosaic: &Mosaic, edges: &[Edge], solution: &Solution, bounds: &Bounds) -> Self {
        let origin = bounds.origin.map(|at| at as f64);
        let tiles: Vec<TileQa> = mosaic
            .tiles
            .iter()
            .zip(&solution.positions)
            .zip(&solution.tiles)
            .map(|((tile, at), report)| TileQa {
                p: tile.key.p,
                corner: [at[0] - origin[0], at[1] - origin[1]],
                size: tile.size,
                moved_px: report.moved_px,
                worst_px: report.worst_px,
                component: report.component,
            })
            .collect();
        let edges: Vec<EdgeQa> = edges
            .iter()
            .zip(&solution.edges)
            .map(|(edge, residual)| EdgeQa {
                i: edge.i,
                j: edge.j,
                residual_px: residual.residual[0].hypot(residual.residual[1]),
                pce: edge.pce,
                ncc: edge.ncc,
                confident: edge.confident,
                used: residual.used,
            })
            .collect();
        let trusted = edges.iter().filter(|edge| edge.confident).count();
        let summary = Summary {
            tiles: tiles.len(),
            pairs: edges.len(),
            trusted,
            rejected: trusted - edges.iter().filter(|edge| edge.used).count(),
            components: solution.components,
            method: solution.method,
            rms_px: solution.rms_px,
            max_px: solution.max_px,
            moved_px: tiles.iter().map(|tile| tile.moved_px).fold(0.0, f64::max),
        };
        Self {
            bounds: *bounds,
            tiles,
            edges,
            summary,
        }
    }
}
