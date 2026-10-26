//! Tile scans (ADR-0053): a region of the stage covered by a grid of frames along the camera's own
//! axes, with the overlap asked between neighbours, in an order; and what a scan will take.
//!
//! - **The grid's steps** come from the stage–camera calibration `L`: a step along a column moves
//!   the image by `(1 − overlap)` of its width, along a row by `(1 − overlap)` of its height. So
//!   neighbours overlap by exactly `overlap` of the frame, whatever the camera's rotation.
//! - **The tiles kept** are those whose frame meets the region; a frame's centre is where the
//!   stage is (architecture §13).
//! - **The order:** a snake, row by row turning at each end; a raster, every row the same way; or
//!   the nearest tile next, from the first.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use siq_types::Affine2;

/// A region of the stage, in µm.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "shape", rename_all = "snake_case")]
pub enum Region {
    /// The rectangle between two corners.
    Rect {
        /// A corner.
        x0: f64,
        /// A corner.
        y0: f64,
        /// The opposite corner.
        x1: f64,
        /// The opposite corner.
        y1: f64,
    },
    /// A polygon, by its vertices in order.
    Polygon {
        /// The vertices, `[x, y]` in µm.
        points: Vec<[f64; 2]>,
    },
}

impl Region {
    /// The region moved by `dx`, `dy` µm.
    pub fn shifted(&self, dx: f64, dy: f64) -> Self {
        match self {
            Self::Rect { x0, y0, x1, y1 } => Self::Rect {
                x0: x0 + dx,
                y0: y0 + dy,
                x1: x1 + dx,
                y1: y1 + dy,
            },
            Self::Polygon { points } => Self::Polygon {
                points: points.iter().map(|[x, y]| [x + dx, y + dy]).collect(),
            },
        }
    }

    /// Its vertices.
    fn vertices(&self) -> Vec<[f64; 2]> {
        match self {
            Self::Rect { x0, y0, x1, y1 } => vec![[*x0, *y0], [*x1, *y0], [*x1, *y1], [*x0, *y1]],
            Self::Polygon { points } => points.clone(),
        }
    }
}

/// The order tiles are visited in.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Order {
    /// Row by row, turning at each end.
    #[default]
    Snake,
    /// Row by row, every row the same way.
    Raster,
    /// The nearest tile not yet visited, from the first.
    Nearest,
}

/// A tile of a scan: its grid column and row, and where the stage goes for it, in µm.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Tile {
    /// Its column.
    pub column: i32,
    /// Its row.
    pub row: i32,
    /// Where the stage goes, in µm.
    pub x: f64,
    /// Where the stage goes, in µm.
    pub y: f64,
}

/// A scan's grid: its tiles in their order, and a step along a row and along a column, in µm.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Grid {
    /// The tiles, in the order they are visited.
    pub tiles: Vec<Tile>,
    /// The stage's move from a tile to the next in its row, and to the next in its column.
    pub steps: [[f64; 2]; 2],
    /// The overlap between neighbours, of the frame.
    pub overlap: f64,
}

/// Why a scan can't be planned.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ScanError {
    /// The region, the overlap or the calibration can't make a grid.
    #[error("the scan can't be planned: {0}")]
    Invalid(String),
}

/// The most tiles a scan may have.
pub const MAX_TILES: usize = 100_000;

/// The grid covering `region` with frames of `field` sensor pixels (`width`, `height`), through
/// `l`, stage µm to sensor pixels, with `overlap` of the frame between neighbours, in `order`.
pub fn plan(
    region: &Region,
    l: &Affine2,
    field: [f64; 2],
    overlap: f64,
    order: Order,
) -> Result<Grid, ScanError> {
    let invalid = |why: &str| Err(ScanError::Invalid(why.to_owned()));
    if !(0.0..=0.9).contains(&overlap) {
        return invalid("the overlap is from 0 to 0.9");
    }
    let vertices = region.vertices();
    if vertices.len() < 3 || vertices.iter().flatten().any(|v| !v.is_finite()) {
        return invalid("a region needs three finite vertices");
    }
    let inverse = l
        .invert()
        .ok_or_else(|| ScanError::Invalid("L isn't invertible".into()))?;
    let [[a, b, _], [c, d, _]] = l.rows();
    let share = 1.0 - overlap;
    // Grid coordinates, in steps from `origin`: L·(S − origin) over a step's pixels.
    let origin = centre(&vertices);
    let to_grid = |[x, y]: [f64; 2]| {
        let (dx, dy) = (x - origin[0], y - origin[1]);
        [
            (a * dx + b * dy) / (share * field[0]),
            (c * dx + d * dy) / (share * field[1]),
        ]
    };
    let polygon: Vec<[f64; 2]> = vertices.iter().copied().map(to_grid).collect();
    if area(&polygon).abs() < 1e-12 {
        return invalid("the region has no area");
    }
    // A frame reaches half its size each way from its centre: in steps, this far.
    let half = 0.5 / share;
    let span = |axis: usize| {
        let (low, high) = polygon.iter().fold((f64::MAX, f64::MIN), |(l, h), p| {
            (l.min(p[axis]), h.max(p[axis]))
        });
        let count = ((high - low) - 2.0 * half + 1.0 - 1e-9).ceil().max(1.0) as i64;
        (((low + high) / 2.0), count)
    };
    let ((ca, columns), (cb, rows)) = (span(0), span(1));
    if (columns as f64) * (rows as f64) > MAX_TILES as f64 {
        return invalid("the region needs more than 100 000 tiles");
    }
    let step = |axis: usize| {
        inverse.apply_vector(if axis == 0 {
            [share * field[0], 0.0]
        } else {
            [0.0, share * field[1]]
        })
    };
    let steps = [step(0), step(1)];
    let mut tiles = Vec::new();
    for row in 0..rows {
        let gb = cb + row as f64 - (rows - 1) as f64 / 2.0;
        let mut line: Vec<Tile> = (0..columns)
            .filter_map(|column| {
                let ga = ca + column as f64 - (columns - 1) as f64 / 2.0;
                let frame = [ga - half, gb - half, ga + half, gb + half];
                meets(&polygon, frame).then(|| Tile {
                    column: column as i32,
                    row: row as i32,
                    x: origin[0] + ga * steps[0][0] + gb * steps[1][0],
                    y: origin[1] + ga * steps[0][1] + gb * steps[1][1],
                })
            })
            .collect();
        if order == Order::Snake && row % 2 == 1 {
            line.reverse();
        }
        tiles.extend(line);
    }
    if order == Order::Nearest {
        tiles = nearest_first(tiles);
    }
    Ok(Grid {
        tiles,
        steps,
        overlap,
    })
}

/// The centre of `points`' bounding box.
fn centre(points: &[[f64; 2]]) -> [f64; 2] {
    let fold = |axis: usize| {
        let (low, high) = points.iter().fold((f64::MAX, f64::MIN), |(l, h), p| {
            (l.min(p[axis]), h.max(p[axis]))
        });
        (low + high) / 2.0
    };
    [fold(0), fold(1)]
}

/// The signed area of `polygon`.
fn area(polygon: &[[f64; 2]]) -> f64 {
    let edges = polygon.iter().zip(polygon.iter().cycle().skip(1));
    edges.map(|(p, q)| p[0] * q[1] - q[0] * p[1]).sum::<f64>() / 2.0
}

/// Whether `point` is inside `polygon`, by the crossings of a ray to +x.
fn inside(polygon: &[[f64; 2]], [x, y]: [f64; 2]) -> bool {
    let edges = polygon.iter().zip(polygon.iter().cycle().skip(1));
    let crossings = edges.filter(|(p, q)| {
        (p[1] > y) != (q[1] > y) && x < p[0] + (y - p[1]) / (q[1] - p[1]) * (q[0] - p[0])
    });
    crossings.count() % 2 == 1
}

/// Whether segments `p`–`q` and `r`–`s` cross or touch.
fn cross(p: [f64; 2], q: [f64; 2], r: [f64; 2], s: [f64; 2]) -> bool {
    let side = |a: [f64; 2], b: [f64; 2], c: [f64; 2]| {
        ((b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])).signum()
    };
    side(p, q, r) * side(p, q, s) <= 0.0 && side(r, s, p) * side(r, s, q) <= 0.0
}

/// Whether the box `[a0, b0, a1, b1]` meets `polygon`.
fn meets(polygon: &[[f64; 2]], [a0, b0, a1, b1]: [f64; 4]) -> bool {
    let corners = [[a0, b0], [a1, b0], [a1, b1], [a0, b1]];
    let within = |p: &[f64; 2]| (a0..=a1).contains(&p[0]) && (b0..=b1).contains(&p[1]);
    polygon.iter().any(within)
        || corners.iter().any(|corner| inside(polygon, *corner))
        || polygon
            .iter()
            .zip(polygon.iter().cycle().skip(1))
            .any(|(p, q)| (0..4).any(|k| cross(*p, *q, corners[k], corners[(k + 1) % 4])))
}

/// `tiles` from the first, each next the nearest not yet visited.
fn nearest_first(mut tiles: Vec<Tile>) -> Vec<Tile> {
    let mut ordered = Vec::with_capacity(tiles.len());
    if tiles.is_empty() {
        return ordered;
    }
    let mut at = tiles.remove(0);
    ordered.push(at);
    while !tiles.is_empty() {
        let distance = |tile: &Tile| (tile.x - at.x).hypot(tile.y - at.y);
        let next = (0..tiles.len())
            .min_by(|&i, &j| distance(&tiles[i]).total_cmp(&distance(&tiles[j])))
            .unwrap_or(0);
        at = tiles.swap_remove(next);
        ordered.push(at);
    }
    ordered
}

/// How long the parts of a scan take, for its estimate.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Timing {
    /// The stage's speed, in µm/s.
    pub speed_um_s: f64,
    /// Its acceleration, in µm/s².
    pub acceleration_um_s2: f64,
    /// What each move takes beyond its motion, in s: the driver seeing it done.
    pub move_overhead_s: f64,
    /// Putting a channel in place, beyond the exposure's half it waits, in s.
    pub channel_s: f64,
    /// A frame's readout and delivery, beyond its exposure, in s.
    pub readout_s: f64,
}

impl Default for Timing {
    /// The simulated stage (50 mm/s, 200 mm/s², its position read at 5 Hz, so a move is seen done
    /// half a period late on average) and peripherals, as measured on them.
    fn default() -> Self {
        Self {
            speed_um_s: 50_000.0,
            acceleration_um_s2: 200_000.0,
            move_overhead_s: 0.1,
            channel_s: 0.15,
            readout_s: 0.12,
        }
    }
}

/// What a scan will take.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Estimate {
    /// Its tiles.
    pub tiles: usize,
    /// Its frames.
    pub frames: usize,
    /// How long it takes, in s.
    pub seconds: f64,
    /// The bytes its frames take on disk.
    pub bytes: u64,
}

impl Estimate {
    /// Whether its frames fit on a disk with `free` bytes, keeping `keep` free. They pass through
    /// the frame pool to the disk, so the RAM a scan takes doesn't grow with it.
    pub fn fits(&self, free: u64, keep: u64) -> bool {
        free.saturating_sub(keep) >= self.bytes
    }
}

/// A move's time over `distance` µm, at `timing`'s speed and acceleration.
fn move_time(distance: f64, timing: &Timing) -> f64 {
    let (v, a) = (timing.speed_um_s, timing.acceleration_um_s2);
    match distance > v * v / a {
        true => distance / v + v / a,
        false => 2.0 * (distance / a).sqrt(),
    }
}

/// What `grid` takes from `start`, settling `settle_s` after each move, with a frame of
/// `frame_bytes` in each of `exposures_s` (a channel's exposure each) at every tile.
pub fn estimate(
    grid: &Grid,
    start: [f64; 2],
    settle_s: f64,
    exposures_s: &[f64],
    frame_bytes: u64,
    timing: &Timing,
) -> Estimate {
    let mut at = start;
    let mut seconds = 0.0;
    for tile in &grid.tiles {
        let distance = (tile.x - at[0]).hypot(tile.y - at[1]);
        seconds += move_time(distance, timing) + timing.move_overhead_s + settle_s;
        at = [tile.x, tile.y];
    }
    // One channel is put in place once; several, at each tile.
    let switches = match exposures_s.len() {
        0 | 1 => 1,
        n => n * grid.tiles.len(),
    };
    let per_frame: f64 = exposures_s
        .iter()
        .map(|e| e + timing.readout_s)
        .sum::<f64>()
        .max(timing.readout_s);
    let half_exposures: f64 = exposures_s.iter().map(|e| (e / 2.0).min(1.0)).sum();
    let average_half = half_exposures / exposures_s.len().max(1) as f64;
    seconds += switches as f64 * (timing.channel_s + average_half);
    seconds += per_frame * grid.tiles.len() as f64;
    let frames = grid.tiles.len() * exposures_s.len().max(1);
    Estimate {
        tiles: grid.tiles.len(),
        frames,
        seconds,
        bytes: frames as u64 * frame_bytes,
    }
}
