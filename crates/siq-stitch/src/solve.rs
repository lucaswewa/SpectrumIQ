//! Global alignment (ADR-0062): the tiles' corners that best agree with the trusted edges and,
//! weakly, with the stage, by weighted least squares over translations. Each edge weighs as its
//! correlation says, Huber's weights temper the edges that disagree, and the one that disagrees
//! most is rejected and the rest solved again, until none is out of bounds.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{Edge, Mosaic};

/// How the solution is found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Method {
    /// The normal equations factored by Cholesky: a few hundred tiles at most.
    Dense,
    /// Conjugate gradients on the sparse system, preconditioned by its diagonal.
    ConjugateGradient,
}

/// How the tiles are aligned.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct SolveOptions {
    /// The stage's uncertainty: its error's standard deviation per axis, in µm. Each tile is held
    /// to its stage position this loosely.
    pub stage_sigma_um: f64,
    /// An edge's uncertainty per axis at a correlation of 0.95, in mosaic pixels: the least of
    /// its robust spread.
    pub edge_sigma_px: f64,
    /// Huber's threshold, in the edges' standard deviations.
    pub huber_k: f64,
    /// The edge that disagrees most is rejected while its residual is over this many standard
    /// deviations, and over `reject_px`.
    pub reject_k: f64,
    /// The least residual rejected, in mosaic pixels.
    pub reject_px: f64,
    /// The reweightings at most, each round.
    pub iterations: usize,
    /// Below this many tiles the normal equations are factored; at or above it, conjugate
    /// gradients.
    pub dense_below: usize,
    /// The method, whatever the size: by `dense_below` if `None`.
    pub method: Option<Method>,
}

impl Default for SolveOptions {
    /// A stage of 5 µm and edges of a quarter pixel; Huber at 1.345, rejection at 4 standard
    /// deviations and a pixel; 10 reweightings; dense below 500 tiles, where a factoring takes
    /// about 12 ms and conjugate gradients a millisecond.
    fn default() -> Self {
        Self {
            stage_sigma_um: 5.0,
            edge_sigma_px: 0.25,
            huber_k: 1.345,
            reject_k: 4.0,
            reject_px: 1.0,
            iterations: 10,
            dense_below: 500,
            method: None,
        }
    }
}

/// An edge, against the solution.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Residual {
    /// The first tile.
    pub i: usize,
    /// The second.
    pub j: usize,
    /// How far the solution's offset is from the edge's, in mosaic pixels.
    pub residual: [f64; 2],
    /// Its weight in the last solve, against an untempered edge's 1.
    pub weight: f64,
    /// Whether it was used: trusted, and not rejected.
    pub used: bool,
}

/// A tile, against the solution.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TileReport {
    /// How far it moved from its stage position, in mosaic pixels.
    pub moved_px: f64,
    /// The largest residual of its edges used, in mosaic pixels: 0 without one.
    pub worst_px: f64,
    /// Its connected component, by the edges used: 0 is the largest.
    pub component: usize,
}

/// The tiles aligned.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Solution {
    /// Each tile's corner, in mosaic pixels.
    pub positions: Vec<[f64; 2]>,
    /// Each edge given, in order.
    pub edges: Vec<Residual>,
    /// Each tile.
    pub tiles: Vec<TileReport>,
    /// The connected components of the edges used: a tile without one is a component.
    pub components: usize,
    /// How it was found.
    pub method: Method,
    /// The edges used: their residuals' root mean square and largest, in mosaic pixels.
    pub rms_px: f64,
    /// The largest.
    pub max_px: f64,
    /// The solves made.
    pub solves: usize,
}

/// The positions of `mosaic`'s tiles that best agree with `edges`, the trusted ones, and with the
/// stage. Each component of the edges keeps its tiles' mean at their stage positions; a tile
/// without an edge stays at its own.
pub fn solve(mosaic: &Mosaic, edges: &[Edge], options: &SolveOptions) -> Solution {
    let n = mosaic.tiles.len();
    let method = options.method.unwrap_or(if n < options.dense_below {
        Method::Dense
    } else {
        Method::ConjugateGradient
    });
    let prior = (options.stage_sigma_um * mosaic.px_per_um)
        .max(1e-9)
        .powi(-2);
    // Each edge as the deviation of its offset from the nominal one.
    let measured: Vec<[f64; 2]> = edges
        .iter()
        .map(|edge| {
            let (a, b) = (mosaic.tiles[edge.i].nominal, mosaic.tiles[edge.j].nominal);
            [edge.shift[0] - (b[0] - a[0]), edge.shift[1] - (b[1] - a[1])]
        })
        .collect();
    let mut used: Vec<bool> = edges.iter().map(|edge| edge.confident).collect();
    let mut weights = vec![1.0; edges.len()];
    let base = options.edge_sigma_px.max(1e-9).powi(-2);
    let bases: Vec<f64> = edges.iter().map(|edge| base * trust(edge.ncc)).collect();
    let mut deviation = vec![[0.0; 2]; n];
    let mut solves = 0;
    let residual = |d: &[[f64; 2]], k: usize| {
        let (edge, m) = (&edges[k], measured[k]);
        let [di, dj] = [d[edge.i], d[edge.j]];
        [dj[0] - di[0] - m[0], dj[1] - di[1] - m[1]]
    };
    let norm = |[x, y]: [f64; 2]| x.hypot(y);
    // Reweighted rounds; after each, the edge that disagrees most is rejected, if it is out of
    // bounds: an outlier pulls its neighbours too, which it no longer does once rejected.
    for _round in 0..=edges.len() {
        let mut sigma = options.edge_sigma_px;
        for _ in 0..options.iterations.max(1) {
            let system = System::new(n, edges, &used, &weights, &bases, prior, &measured);
            let next = match method {
                Method::Dense => system.dense(),
                Method::ConjugateGradient => system.conjugate_gradient(),
            };
            solves += 1;
            let change = next
                .iter()
                .zip(&deviation)
                .map(|(a, b)| norm([a[0] - b[0], a[1] - b[1]]))
                .fold(0.0, f64::max);
            deviation = next;
            // The spread of the edges used, robustly: a 2D residual's median norm is 1.18 σ.
            let mut norms: Vec<f64> = (0..edges.len())
                .filter(|&k| used[k])
                .map(|k| norm(residual(&deviation, k)))
                .collect();
            sigma = median(&mut norms).map_or(options.edge_sigma_px, |m| {
                (m / 1.1774).max(options.edge_sigma_px)
            });
            let limit = options.huber_k * sigma;
            for (k, weight) in weights.iter_mut().enumerate() {
                let r = norm(residual(&deviation, k));
                *weight = if r <= limit { 1.0 } else { limit / r };
            }
            if change < 1e-9 {
                break;
            }
        }
        let bound = (options.reject_k * sigma).max(options.reject_px);
        let worst = (0..edges.len())
            .filter(|&k| used[k])
            .map(|k| (k, norm(residual(&deviation, k))))
            .max_by(|a, b| a.1.total_cmp(&b.1));
        match worst {
            Some((k, r)) if r > bound => used[k] = false,
            _ => break,
        }
    }

    let positions: Vec<[f64; 2]> = mosaic
        .tiles
        .iter()
        .zip(&deviation)
        .map(|(tile, d)| [tile.nominal[0] + d[0], tile.nominal[1] + d[1]])
        .collect();
    let reported: Vec<Residual> = (0..edges.len())
        .map(|k| Residual {
            i: edges[k].i,
            j: edges[k].j,
            residual: residual(&deviation, k),
            weight: if used[k] { weights[k] } else { 0.0 },
            used: used[k],
        })
        .collect();
    let component = components(n, edges, &used);
    let mut tiles: Vec<TileReport> = (0..n)
        .map(|i| TileReport {
            moved_px: norm(deviation[i]),
            worst_px: 0.0,
            component: component[i],
        })
        .collect();
    let used_norms: Vec<f64> = reported
        .iter()
        .filter(|r| r.used)
        .map(|r| {
            let size = norm(r.residual);
            for t in [r.i, r.j] {
                tiles[t].worst_px = tiles[t].worst_px.max(size);
            }
            size
        })
        .collect();
    let rms = if used_norms.is_empty() {
        0.0
    } else {
        (used_norms.iter().map(|r| r * r).sum::<f64>() / used_norms.len() as f64).sqrt()
    };
    Solution {
        positions,
        edges: reported,
        tiles,
        components: component.iter().max().map_or(0, |&c| c + 1),
        method,
        rms_px: rms,
        max_px: used_norms.iter().copied().fold(0.0, f64::max),
        solves,
    }
}

fn median(values: &mut [f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let middle = values.len() / 2;
    let (_, m, _) = values.select_nth_unstable_by(middle, f64::total_cmp);
    Some(*m)
}

/// The tiles' components by the edges used, largest first.
fn components(n: usize, edges: &[Edge], used: &[bool]) -> Vec<usize> {
    let mut parent: Vec<usize> = (0..n).collect();
    fn root(parent: &mut [usize], mut i: usize) -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }
    for (edge, _) in edges.iter().zip(used).filter(|&(_, used)| *used) {
        let (a, b) = (root(&mut parent, edge.i), root(&mut parent, edge.j));
        parent[a.max(b)] = a.min(b);
    }
    let roots: Vec<usize> = (0..n).map(|i| root(&mut parent, i)).collect();
    let mut sizes = std::collections::HashMap::new();
    for &r in &roots {
        *sizes.entry(r).or_insert(0usize) += 1;
    }
    let mut order: Vec<(usize, usize)> = sizes.into_iter().collect();
    order.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let rank: std::collections::HashMap<usize, usize> = order
        .iter()
        .enumerate()
        .map(|(k, &(r, _))| (r, k))
        .collect();
    roots.iter().map(|r| rank[r]).collect()
}

/// An edge's weight for its correlation, against one of 0.95 (P8.5): two views of the same content,
/// each with its noise, correlate as signal / (signal + noise), and an offset's variance goes as
/// noise / signal, so as (1 − ncc) / ncc. Above 0.95 a correlation no longer tells a better match,
/// and the edges there count alike, so that two true ones outvote a false one; below, the weight
/// falls to a fifth at 0.8, so that a tile's weak false match gives way to its true one.
fn trust(ncc: f64) -> f64 {
    let ncc = ncc.clamp(0.5, 0.95);
    ncc / (1.0 - ncc) / 19.0
}

/// The normal equations of the deviations from the stage: `(λI + L) d = B m`, where `L` is the
/// edges' weighted Laplacian and `B m` gathers each edge's measured deviation at its ends. Both
/// axes share the matrix.
struct System {
    n: usize,
    /// Each edge used: its ends and weight.
    links: Vec<(usize, usize, f64)>,
    diagonal: Vec<f64>,
    rhs: Vec<[f64; 2]>,
    prior: f64,
}

impl System {
    fn new(
        n: usize,
        edges: &[Edge],
        used: &[bool],
        weights: &[f64],
        bases: &[f64],
        prior: f64,
        measured: &[[f64; 2]],
    ) -> Self {
        let (mut diagonal, mut rhs) = (vec![prior; n], vec![[0.0; 2]; n]);
        let mut links = Vec::new();
        for (k, edge) in edges.iter().enumerate().filter(|&(k, _)| used[k]) {
            let w = bases[k] * weights[k];
            diagonal[edge.i] += w;
            diagonal[edge.j] += w;
            for axis in 0..2 {
                rhs[edge.j][axis] += w * measured[k][axis];
                rhs[edge.i][axis] -= w * measured[k][axis];
            }
            links.push((edge.i, edge.j, w));
        }
        Self {
            n,
            links,
            diagonal,
            rhs,
            prior,
        }
    }

    /// The solution by Cholesky's factoring of the dense matrix.
    fn dense(&self) -> Vec<[f64; 2]> {
        let n = self.n;
        let mut a = vec![0.0; n * n];
        for (i, &d) in self.diagonal.iter().enumerate() {
            a[i * n + i] = d;
        }
        for &(i, j, w) in &self.links {
            a[i * n + j] -= w;
            a[j * n + i] -= w;
        }
        // A = L·Lᵀ, in place in the lower triangle.
        for j in 0..n {
            let mut d = a[j * n + j];
            for k in 0..j {
                d -= a[j * n + k] * a[j * n + k];
            }
            let d = d.max(f64::MIN_POSITIVE).sqrt();
            a[j * n + j] = d;
            for i in j + 1..n {
                let mut s = a[i * n + j];
                for k in 0..j {
                    s -= a[i * n + k] * a[j * n + k];
                }
                a[i * n + j] = s / d;
            }
        }
        // L·y = b, then Lᵀ·x = y, for each axis.
        let mut solution = vec![[0.0; 2]; n];
        for axis in 0..2 {
            let mut x: Vec<f64> = self.rhs.iter().map(|r| r[axis]).collect();
            for i in 0..n {
                let s: f64 = (0..i).map(|k| a[i * n + k] * x[k]).sum();
                x[i] = (x[i] - s) / a[i * n + i];
            }
            for i in (0..n).rev() {
                let s: f64 = (i + 1..n).map(|k| a[k * n + i] * x[k]).sum();
                x[i] = (x[i] - s) / a[i * n + i];
            }
            for (out, value) in solution.iter_mut().zip(x) {
                out[axis] = value;
            }
        }
        solution
    }

    /// `A·x` for one axis.
    fn apply(&self, x: &[f64], out: &mut [f64]) {
        for (o, &v) in out.iter_mut().zip(x) {
            *o = self.prior * v;
        }
        for &(i, j, w) in &self.links {
            let d = w * (x[i] - x[j]);
            out[i] += d;
            out[j] -= d;
        }
    }

    /// The solution by conjugate gradients, preconditioned by the diagonal, to a relative residual
    /// of 10⁻¹².
    fn conjugate_gradient(&self) -> Vec<[f64; 2]> {
        let n = self.n;
        let mut solution = vec![[0.0; 2]; n];
        for axis in 0..2 {
            let b: Vec<f64> = self.rhs.iter().map(|r| r[axis]).collect();
            let bound = 1e-24 * b.iter().map(|v| v * v).sum::<f64>().max(f64::MIN_POSITIVE);
            let mut x = vec![0.0; n];
            let mut r = b.clone();
            let mut z: Vec<f64> = r.iter().zip(&self.diagonal).map(|(r, d)| r / d).collect();
            let mut p = z.clone();
            let mut rz: f64 = r.iter().zip(&z).map(|(a, b)| a * b).sum();
            let mut ap = vec![0.0; n];
            for _ in 0..10 * n + 1000 {
                if r.iter().map(|v| v * v).sum::<f64>() <= bound {
                    break;
                }
                self.apply(&p, &mut ap);
                let alpha = rz / p.iter().zip(&ap).map(|(a, b)| a * b).sum::<f64>();
                for k in 0..n {
                    x[k] += alpha * p[k];
                    r[k] -= alpha * ap[k];
                }
                for k in 0..n {
                    z[k] = r[k] / self.diagonal[k];
                }
                let next: f64 = r.iter().zip(&z).map(|(a, b)| a * b).sum();
                let beta = next / rz;
                rz = next;
                for k in 0..n {
                    p[k] = z[k] + beta * p[k];
                }
            }
            for (s, v) in solution.iter_mut().zip(x) {
                s[axis] = v;
            }
        }
        solution
    }
}
