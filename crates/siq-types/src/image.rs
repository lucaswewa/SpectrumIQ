//! Pixel formats, sensor regions, and borrowed image views.
//!
//! [`ImageView`] and [`ImageViewMut`] borrow pixels from anywhere (a frame-pool slot, a memory map,
//! a GPU readback) and never allocate. Rows are `stride` elements apart, so a view of a crop
//! shares its parent's memory. [`OwnedImage`] owns its pixels, for tests and tools.

#[cfg(test)]
mod tests;

use std::num::NonZeroU8;
use std::ops::Range;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::geometry::RectPx;

/// How a camera delivers pixels. Every format wider than 8 bits arrives in `u16` words, with the
/// value in the low bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub enum PixelFormat {
    /// 8-bit monochrome, in `u8`.
    Mono8,
    /// 10-bit monochrome, in `u16`.
    Mono10,
    /// 12-bit monochrome, in `u16`.
    Mono12,
    /// 14-bit monochrome, in `u16`.
    Mono14,
    /// 16-bit monochrome, in `u16`.
    Mono16,
}

impl PixelFormat {
    /// The number of significant bits.
    pub const fn bits(self) -> u32 {
        match self {
            PixelFormat::Mono8 => 8,
            PixelFormat::Mono10 => 10,
            PixelFormat::Mono12 => 12,
            PixelFormat::Mono14 => 14,
            PixelFormat::Mono16 => 16,
        }
    }

    /// The bytes each pixel takes: 1 for 8 bits, 2 for the formats in `u16` words.
    pub const fn bytes_per_pixel(self) -> usize {
        self.bits().div_ceil(8) as usize
    }

    /// The largest value a pixel can have.
    pub const fn max_value(self) -> u16 {
        u16::MAX >> (16 - self.bits())
    }
}

/// A region of the sensor, in unbinned sensor pixels, as GenICam's `OffsetX`, `OffsetY`,
/// `Width` and `Height`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub struct Roi {
    /// The first sensor column.
    pub offset_x: u32,
    /// The first sensor row.
    pub offset_y: u32,
    /// The number of sensor columns.
    pub width: u32,
    /// The number of sensor rows.
    pub height: u32,
}

impl Roi {
    /// The whole of a `width` × `height` sensor.
    pub const fn full(width: u32, height: u32) -> Self {
        Self {
            offset_x: 0,
            offset_y: 0,
            width,
            height,
        }
    }
}

impl From<Roi> for RectPx {
    fn from(roi: Roi) -> Self {
        RectPx::new(roi.offset_x, roi.offset_y, roi.width, roi.height)
    }
}

/// How many sensor pixels each image pixel combines, across and down.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub struct Binning {
    /// Sensor columns per image column.
    pub horizontal: NonZeroU8,
    /// Sensor rows per image row.
    pub vertical: NonZeroU8,
}

impl Binning {
    /// No binning: one sensor pixel per image pixel.
    pub const NONE: Self = Self {
        horizontal: NonZeroU8::MIN,
        vertical: NonZeroU8::MIN,
    };

    /// Binning by these factors, or `None` if either is 0.
    pub fn new(horizontal: u8, vertical: u8) -> Option<Self> {
        let horizontal = NonZeroU8::new(horizontal)?;
        let vertical = NonZeroU8::new(vertical)?;
        Some(Self {
            horizontal,
            vertical,
        })
    }

    /// The factors, across and down.
    pub const fn factors(self) -> (u8, u8) {
        (self.horizontal.get(), self.vertical.get())
    }

    /// The image size that a region of `width` × `height` sensor pixels gives. Partial bins at the
    /// edges are dropped.
    pub const fn image_size(self, width: u32, height: u32) -> (u32, u32) {
        let (horizontal, vertical) = self.factors();
        (width / horizontal as u32, height / vertical as u32)
    }
}

impl Default for Binning {
    fn default() -> Self {
        Self::NONE
    }
}

/// Why an image view couldn't be made.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ImageError {
    /// Rows would overlap.
    #[error("a stride of {stride} is less than the width of {width}")]
    StrideTooSmall {
        /// The stride asked for.
        stride: usize,
        /// The width asked for.
        width: usize,
    },
    /// The buffer is too short for the rows.
    #[error("the image needs {needed} elements, but the buffer has {len}")]
    BufferTooSmall {
        /// Elements needed.
        needed: usize,
        /// Elements available.
        len: usize,
    },
    /// A sub-view's rectangle goes outside the image.
    #[error("{rect:?} is outside a {width}×{height} image")]
    OutOfBounds {
        /// The rectangle asked for.
        rect: RectPx,
        /// The image's width.
        width: usize,
        /// The image's height.
        height: usize,
    },
}

/// How an image's rows sit in its data: `height` rows of `width` elements, `stride` apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Layout {
    width: usize,
    height: usize,
    stride: usize,
}

impl Layout {
    /// The layout, if `len` elements hold its rows. The last row needs no padding after it.
    fn new(len: usize, width: usize, height: usize, stride: usize) -> Result<Self, ImageError> {
        if stride < width {
            return Err(ImageError::StrideTooSmall { stride, width });
        }
        let needed = match (width, height) {
            (0, _) | (_, 0) => 0,
            _ => stride
                .checked_mul(height - 1)
                .and_then(|rows| rows.checked_add(width))
                .unwrap_or(usize::MAX),
        };
        if len < needed {
            return Err(ImageError::BufferTooSmall { needed, len });
        }
        Ok(Self {
            width,
            height,
            stride,
        })
    }

    /// Where row `y` is in the data. A row without columns is the empty range at 0, so that a
    /// zero-width view needs no data.
    fn row(&self, y: usize) -> Range<usize> {
        assert!(y < self.height, "row {y} of a {}-row image", self.height);
        if self.width == 0 {
            return 0..0;
        }
        let start = y * self.stride;
        start..start + self.width
    }

    /// The layout of `rect` within this one, and where it starts in the data.
    fn sub(&self, rect: RectPx) -> Result<(usize, Self), ImageError> {
        let (width, height) = (self.width, self.height);
        if rect.right() > width as u64 || rect.bottom() > height as u64 {
            return Err(ImageError::OutOfBounds {
                rect,
                width,
                height,
            });
        }
        // An empty rectangle may start at the very end of the data. A rectangle inside the parent
        // needs no more data after its start than the parent has, so its layout holds.
        let start = rect.y as usize * self.stride + rect.x as usize;
        let layout = Self {
            width: rect.width as usize,
            height: rect.height as usize,
            stride: self.stride,
        };
        Ok((start, layout))
    }
}

/// A borrowed image: `height` rows of `width` pixels, each row `stride` elements after the last.
#[derive(Debug)]
pub struct ImageView<'a, T> {
    data: &'a [T],
    layout: Layout,
}

// By hand: a derive would require `T: Copy`, though a view only holds a reference.
impl<T> Clone for ImageView<'_, T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for ImageView<'_, T> {}

impl<'a, T> ImageView<'a, T> {
    /// A view of `data`, or an error if the rows don't fit in it.
    pub fn new(
        data: &'a [T],
        width: usize,
        height: usize,
        stride: usize,
    ) -> Result<Self, ImageError> {
        let layout = Layout::new(data.len(), width, height, stride)?;
        Ok(Self { data, layout })
    }

    /// The number of pixels in a row.
    pub const fn width(&self) -> usize {
        self.layout.width
    }

    /// The number of rows.
    pub const fn height(&self) -> usize {
        self.layout.height
    }

    /// The distance between the starts of two rows, in elements.
    pub const fn stride(&self) -> usize {
        self.layout.stride
    }

    /// Row `y`.
    ///
    /// # Panics
    ///
    /// If `y` is not less than the height.
    pub fn row(&self, y: usize) -> &'a [T] {
        &self.data[self.layout.row(y)]
    }

    /// Every row, top to bottom.
    pub fn rows(&self) -> impl Iterator<Item = &'a [T]> + 'a {
        let view = *self;
        (0..view.height()).map(move |y| view.row(y))
    }

    /// The pixel at column `x` of row `y`, if it is inside the image.
    pub fn get(&self, x: usize, y: usize) -> Option<&'a T> {
        let inside = x < self.width() && y < self.height();
        inside.then(|| &self.data[y * self.stride() + x])
    }

    /// A view of `rect` within this view, sharing its memory.
    pub fn sub_view(&self, rect: RectPx) -> Result<Self, ImageError> {
        let (start, layout) = self.layout.sub(rect)?;
        let data = self.data.get(start..).unwrap_or_default();
        Ok(Self { data, layout })
    }
}

/// A mutably borrowed image, laid out as [`ImageView`].
#[derive(Debug)]
pub struct ImageViewMut<'a, T> {
    data: &'a mut [T],
    layout: Layout,
}

impl<'a, T> ImageViewMut<'a, T> {
    /// A view of `data`, or an error if the rows don't fit in it.
    pub fn new(
        data: &'a mut [T],
        width: usize,
        height: usize,
        stride: usize,
    ) -> Result<Self, ImageError> {
        let layout = Layout::new(data.len(), width, height, stride)?;
        Ok(Self { data, layout })
    }

    /// The number of pixels in a row.
    pub const fn width(&self) -> usize {
        self.layout.width
    }

    /// The number of rows.
    pub const fn height(&self) -> usize {
        self.layout.height
    }

    /// The distance between the starts of two rows, in elements.
    pub const fn stride(&self) -> usize {
        self.layout.stride
    }

    /// The same view for a shorter borrow, to hand to a function that takes a view by value.
    pub fn reborrow(&mut self) -> ImageViewMut<'_, T> {
        ImageViewMut {
            data: &mut *self.data,
            layout: self.layout,
        }
    }

    /// A read-only view of the same pixels.
    pub fn as_view(&self) -> ImageView<'_, T> {
        let data = &*self.data;
        ImageView {
            data,
            layout: self.layout,
        }
    }

    /// Row `y`, to change.
    ///
    /// # Panics
    ///
    /// If `y` is not less than the height.
    pub fn row_mut(&mut self, y: usize) -> &mut [T] {
        &mut self.data[self.layout.row(y)]
    }

    /// Every row, top to bottom, to change. The rows are disjoint, so they can be handed to
    /// different threads. An image with no columns yields no rows.
    pub fn rows_mut(&mut self) -> impl Iterator<Item = &mut [T]> {
        let (width, stride) = (self.layout.width, self.layout.stride);
        let rows = if width == 0 { 0 } else { self.layout.height };
        self.data
            .chunks_mut(stride.max(1))
            .take(rows)
            .map(move |row| &mut row[..width])
    }

    /// The rows above `y`, and the rows from `y` on, as two views that can go to different
    /// threads. `y` past the last row leaves the second view empty.
    pub fn split_at_row(self, y: usize) -> (Self, Self) {
        let Layout {
            width,
            height,
            stride,
        } = self.layout;
        let y = y.min(height);
        let (top, bottom) = self.data.split_at_mut((y * stride).min(self.data.len()));
        let layout = |height| Layout {
            width,
            height,
            stride,
        };
        let (top, bottom) = (
            Self {
                data: top,
                layout: layout(y),
            },
            Self {
                data: bottom,
                layout: layout(height - y),
            },
        );
        (top, bottom)
    }

    /// A view of `rect` within this view, sharing its memory.
    pub fn sub_view_mut(&mut self, rect: RectPx) -> Result<ImageViewMut<'_, T>, ImageError> {
        let (start, layout) = self.layout.sub(rect)?;
        let data = self.data.get_mut(start..).unwrap_or_default();
        Ok(ImageViewMut { data, layout })
    }

    /// Sets every pixel to `value`.
    pub fn fill(&mut self, value: T)
    where
        T: Copy,
    {
        self.rows_mut().for_each(|row| row.fill(value));
    }
}

/// The boundary each row of an [`OwnedImage`] starts on, in bytes.
pub const ROW_ALIGNMENT: usize = 64;

/// An image that owns its pixels, with every row starting on a [`ROW_ALIGNMENT`] boundary when
/// the pixel size divides it.
///
/// It is for tests and tools: camera frames live in the frame pool and are seen through views.
#[derive(Debug)]
pub struct OwnedImage<T> {
    buffer: Vec<T>,
    offset: usize,
    layout: Layout,
}

impl<T: Copy + Default> OwnedImage<T> {
    /// An image of `width` × `height` default pixels.
    pub fn new(width: usize, height: usize) -> Self {
        Self::from_fn(width, height, |_, _| T::default())
    }

    /// An image whose pixel at column `x` of row `y` is `pixel(x, y)`.
    pub fn from_fn(width: usize, height: usize, mut pixel: impl FnMut(usize, usize) -> T) -> Self {
        let size = size_of::<T>().max(1);
        let per_boundary = if ROW_ALIGNMENT.is_multiple_of(size) {
            ROW_ALIGNMENT / size
        } else {
            1
        };
        let stride = width.div_ceil(per_boundary) * per_boundary;
        let mut buffer = vec![T::default(); stride * height + per_boundary];
        let base = buffer.as_ptr().addr();
        let offset = (0..per_boundary)
            .find(|elements| (base + elements * size).is_multiple_of(ROW_ALIGNMENT))
            .unwrap_or(0);
        for y in 0..height {
            let row = &mut buffer[offset + y * stride..][..width];
            for (x, value) in row.iter_mut().enumerate() {
                *value = pixel(x, y);
            }
        }
        let layout = Layout {
            width,
            height,
            stride,
        };
        Self {
            buffer,
            offset,
            layout,
        }
    }
}

/// A clone lays its pixels out afresh, because a copied buffer would lose the row alignment.
impl<T: Copy + Default> Clone for OwnedImage<T> {
    fn clone(&self) -> Self {
        let view = self.view();
        Self::from_fn(view.width(), view.height(), |x, y| view.row(y)[x])
    }
}

impl<T> OwnedImage<T> {
    /// The number of pixels in a row.
    pub const fn width(&self) -> usize {
        self.layout.width
    }

    /// The number of rows.
    pub const fn height(&self) -> usize {
        self.layout.height
    }

    /// The distance between the starts of two rows, in elements.
    pub const fn stride(&self) -> usize {
        self.layout.stride
    }

    /// A view of the pixels.
    pub fn view(&self) -> ImageView<'_, T> {
        let data = &self.buffer[self.offset..];
        ImageView {
            data,
            layout: self.layout,
        }
    }

    /// A view of the pixels, to change.
    pub fn view_mut(&mut self) -> ImageViewMut<'_, T> {
        let data = &mut self.buffer[self.offset..];
        ImageViewMut {
            data,
            layout: self.layout,
        }
    }
}
