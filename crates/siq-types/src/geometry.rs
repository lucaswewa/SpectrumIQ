//! Positions and rectangles: in the stage frame (µm) and in images (pixels).
//!
//! The frames are defined in ADR-0018:
//! - **stage frame:** µm, right-handed, `+z` toward the objective;
//! - **image frame:** pixels, origin at the top-left corner of the top-left pixel, `x` right and
//!   `y` down, so pixel `(i, j)` has its centre at `(i + 0.5, j + 0.5)`.

#[cfg(test)]
mod tests;

use std::ops::{Add, Mul, Neg, Sub};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::units::Micrometers;

/// A position or displacement in the stage's XY plane.
///
/// A sample point's coordinates are the stage reading that puts it at the centre of the sensor.
/// Positions on the sample and stage positions are therefore the same kind of value.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Vec2Um {
    /// Along the stage's x axis.
    pub x: Micrometers,
    /// Along the stage's y axis.
    pub y: Micrometers,
}

impl Vec2Um {
    /// The origin.
    pub const ZERO: Self = Self::from_um(0.0, 0.0);

    /// A vector from its components.
    pub const fn new(x: Micrometers, y: Micrometers) -> Self {
        Self { x, y }
    }

    /// A vector from its components in µm.
    pub const fn from_um(x: f64, y: f64) -> Self {
        Self::new(Micrometers(x), Micrometers(y))
    }

    /// The components in µm, for numeric code.
    pub const fn to_um(self) -> [f64; 2] {
        [self.x.0, self.y.0]
    }

    /// The vector's length.
    pub fn length(self) -> Micrometers {
        Micrometers(self.x.0.hypot(self.y.0))
    }
}

impl Add for Vec2Um {
    type Output = Self;
    fn add(self, other: Self) -> Self {
        Self::new(self.x + other.x, self.y + other.y)
    }
}

impl Sub for Vec2Um {
    type Output = Self;
    fn sub(self, other: Self) -> Self {
        Self::new(self.x - other.x, self.y - other.y)
    }
}

impl Neg for Vec2Um {
    type Output = Self;
    fn neg(self) -> Self {
        Self::new(-self.x, -self.y)
    }
}

impl Mul<f64> for Vec2Um {
    type Output = Self;
    fn mul(self, factor: f64) -> Self {
        Self::new(self.x * factor, self.y * factor)
    }
}

/// A point in an image, in pixels, with sub-pixel precision.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PointPx {
    /// Right from the image's left edge.
    pub x: f64,
    /// Down from the image's top edge.
    pub y: f64,
}

impl PointPx {
    /// A point from its coordinates.
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }

    /// The centre of pixel `(column, row)`.
    pub fn pixel_centre(column: u32, row: u32) -> Self {
        Self::new(f64::from(column) + 0.5, f64::from(row) + 0.5)
    }

    /// The coordinates, for numeric code.
    pub const fn to_array(self) -> [f64; 2] {
        [self.x, self.y]
    }
}

impl Add for PointPx {
    type Output = Self;
    fn add(self, other: Self) -> Self {
        Self::new(self.x + other.x, self.y + other.y)
    }
}

impl Sub for PointPx {
    type Output = Self;
    fn sub(self, other: Self) -> Self {
        Self::new(self.x - other.x, self.y - other.y)
    }
}

/// A rectangle in the stage's XY plane: `origin` is its corner with the smallest coordinates.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RectUm {
    /// The corner with the smallest x and y.
    pub origin: Vec2Um,
    /// The width (x) and height (y), not negative.
    pub size: Vec2Um,
}

impl RectUm {
    /// The rectangle's centre.
    pub fn centre(&self) -> Vec2Um {
        self.origin + self.size * 0.5
    }

    /// Whether `point` is inside: the minimum edges are included, the maximum ones aren't.
    pub fn contains(&self, point: Vec2Um) -> bool {
        let end = self.origin + self.size;
        (self.origin.x..end.x).contains(&point.x) && (self.origin.y..end.y).contains(&point.y)
    }
}

/// A rectangle of whole pixels in an image: columns `x..x + width`, rows `y..y + height`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub struct RectPx {
    /// The first column.
    pub x: u32,
    /// The first row.
    pub y: u32,
    /// The number of columns.
    pub width: u32,
    /// The number of rows.
    pub height: u32,
}

impl RectPx {
    /// A rectangle from its first column and row, and its size.
    pub const fn new(x: u32, y: u32, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// Whether it has no pixels.
    pub const fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }

    /// The column after the last one, without overflow.
    pub const fn right(&self) -> u64 {
        self.x as u64 + self.width as u64
    }

    /// The row after the last one, without overflow.
    pub const fn bottom(&self) -> u64 {
        self.y as u64 + self.height as u64
    }

    /// The pixels both rectangles cover, or `None` if they don't overlap.
    pub fn intersection(&self, other: &Self) -> Option<Self> {
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        let right = self.right().min(other.right());
        let bottom = self.bottom().min(other.bottom());
        let width = u32::try_from(right.checked_sub(u64::from(x))?).ok()?;
        let height = u32::try_from(bottom.checked_sub(u64::from(y))?).ok()?;
        let overlap = Self::new(x, y, width, height);
        (!overlap.is_empty()).then_some(overlap)
    }
}

/// Where the stage and the focus drive are.
///
/// Every axis is in the stage frame of ADR-0018. `+z_coarse` and `+focus` both bring the sample
/// and the objective closer together.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct StagePose {
    /// The stage's x axis.
    pub x: Micrometers,
    /// The stage's y axis.
    pub y: Micrometers,
    /// The stage's own z axis, for coarse height and clearance.
    pub z_coarse: Micrometers,
    /// The focus drive: the fine z axis, for z-stacks and autofocus.
    pub focus: Micrometers,
}

impl StagePose {
    /// The position in the XY plane.
    pub const fn xy(&self) -> Vec2Um {
        Vec2Um::new(self.x, self.y)
    }
}
