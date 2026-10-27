//! The tiles of a mosaic, where their stage poses put them, which overlap, and where their pixels
//! come from (ADR-0061).

use siq_storage::{DatasetReader, FrameKey, MappedFrame};
use siq_types::{Affine2, FrameMeta, ImageView, OwnedImage};

use crate::StitchError;

/// A tile: a frame, and where its stage pose puts it on the mosaic.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tile {
    /// Its frame.
    pub key: FrameKey,
    /// Its width and height, in pixels.
    pub size: [u32; 2],
    /// Its top-left corner on the mosaic, in mosaic pixels, from its pose.
    pub nominal: [f64; 2],
    /// Its first pixel on the sensor, for the flat.
    pub sensor: [u32; 2],
    /// The sensor pixels a side of its pixels.
    pub binning: u32,
}

/// The tiles of a mosaic, placed by their poses. The mosaic's pixels are the frames', along the
/// camera's axes: the calibration's rotation, scale and shear are in the placing (ADR-0013), and
/// the tiles differ by translations alone.
#[derive(Debug, Clone, PartialEq)]
pub struct Mosaic {
    /// The tiles.
    pub tiles: Vec<Tile>,
    /// Mosaic pixels a µm.
    pub px_per_um: f64,
}

impl Mosaic {
    /// The tiles of `frames`, placed through `l` (sensor pixels a µm): a stage point `S` is at
    /// `L·S / binning` on the mosaic, so a frame taken at pose `P` has its corner at
    /// `L·P / binning − size / 2`. The frames must share a binning and have a pose.
    pub fn place(frames: &[(FrameKey, FrameMeta)], l: &Affine2) -> Result<Self, StitchError> {
        let binning = frames
            .first()
            .map_or(1, |(_, meta)| u32::from(meta.binning.horizontal.get()));
        let tiles = frames
            .iter()
            .map(|(key, meta)| {
                let pose = meta.pose.ok_or(StitchError::NoPose(*key))?;
                if u32::from(meta.binning.horizontal.get()) != binning {
                    return Err(StitchError::Binnings);
                }
                let (width, height) = meta.image_size();
                let b = f64::from(binning);
                let [x, y] = l.apply_vector([pose.x.0, pose.y.0]);
                Ok(Tile {
                    key: *key,
                    size: [width, height],
                    nominal: [
                        x / b - f64::from(width) / 2.0,
                        y / b - f64::from(height) / 2.0,
                    ],
                    sensor: [meta.roi.offset_x, meta.roi.offset_y],
                    binning,
                })
            })
            .collect::<Result<_, _>>()?;
        Ok(Self {
            tiles,
            px_per_um: l.determinant().abs().sqrt() / f64::from(binning),
        })
    }

    /// The pairs of tiles whose nominal rectangles overlap by at least `min` pixels each way, over
    /// at least `share` of the smaller tile's area: `(i, j)` with `i < j`, in order.
    pub fn overlaps(&self, min: f64, share: f64) -> Vec<(usize, usize)> {
        let tiles = &self.tiles;
        let mut order: Vec<usize> = (0..tiles.len()).collect();
        order.sort_by(|&a, &b| tiles[a].nominal[0].total_cmp(&tiles[b].nominal[0]));
        let mut pairs = Vec::new();
        for (k, &i) in order.iter().enumerate() {
            let a = &tiles[i];
            let right = a.nominal[0] + f64::from(a.size[0]) - min;
            for &j in order[k + 1..]
                .iter()
                .take_while(|&&j| tiles[j].nominal[0] <= right)
            {
                let b = &tiles[j];
                let area = |tile: &Tile| f64::from(tile.size[0]) * f64::from(tile.size[1]);
                let enough = |[w, h]: [f64; 2]| {
                    w >= min && h >= min && w * h >= share * area(a).min(area(b))
                };
                if overlap(a, b).is_some_and(|(_, size)| enough(size)) {
                    pairs.push((i.min(j), i.max(j)));
                }
            }
        }
        pairs.sort_unstable();
        pairs
    }
}

/// Where two tiles' nominal rectangles overlap on the mosaic: its corner and size.
pub(crate) fn overlap(a: &Tile, b: &Tile) -> Option<([f64; 2], [f64; 2])> {
    let span = |axis: usize| {
        let from = a.nominal[axis].max(b.nominal[axis]);
        let to = (a.nominal[axis] + f64::from(a.size[axis]))
            .min(b.nominal[axis] + f64::from(b.size[axis]));
        (to > from).then_some((from, to - from))
    };
    let ((x, w), (y, h)) = (span(0)?, span(1)?);
    Some(([x, y], [w, h]))
}

/// Where the tiles' pixels come from, by their index in the mosaic.
pub trait TileSource: Sync {
    /// Tile `index`'s pixels.
    fn pixels(&self, index: usize) -> Result<ImageView<'_, u16>, StitchError>;
}

/// A dataset's frames, mapped read-only.
pub struct MappedTiles {
    frames: Vec<MappedFrame>,
}

impl MappedTiles {
    /// The frames of `reader` that `keep` takes, mapped, in key order, and their metadata.
    pub fn open(
        reader: &DatasetReader,
        keep: impl Fn(&FrameKey) -> bool,
    ) -> Result<(Self, Vec<(FrameKey, FrameMeta)>), StitchError> {
        let kept: Vec<_> = reader
            .frames()
            .iter()
            .filter(|frame| keep(&frame.sidecar.key))
            .collect();
        let frames = kept
            .iter()
            .map(|frame| frame.map())
            .collect::<Result<Vec<_>, _>>()?;
        let metas = kept
            .iter()
            .map(|frame| (frame.sidecar.key, frame.sidecar.meta))
            .collect();
        Ok((Self { frames }, metas))
    }
}

impl TileSource for MappedTiles {
    fn pixels(&self, index: usize) -> Result<ImageView<'_, u16>, StitchError> {
        let frame = self.frames.get(index).ok_or(StitchError::NoTile(index))?;
        frame.image_u16().ok_or(StitchError::NotWords(index))
    }
}

/// Images in memory, as tiles: synthetic mosaics, and the tests'.
impl TileSource for [OwnedImage<u16>] {
    fn pixels(&self, index: usize) -> Result<ImageView<'_, u16>, StitchError> {
        Ok(self.get(index).ok_or(StitchError::NoTile(index))?.view())
    }
}
