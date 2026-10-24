//! What a world is, so that it can be built again, and the ground truth written beside a
//! dataset.

use std::fs;
use std::path::Path;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use siq_config::model::WorldConfig;
use siq_types::{Micrometers, PointPx, RectUm};

use crate::{
    Bead, Beads, BeadsParams, Cells, CellsParams, Grains, GrainsParams, HeightField,
    HeightFieldParams, ImageParams, ImageWorld, SynthError, World, grid,
};

/// A world's kind and parameters: enough to build it again.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WorldInfo {
    /// [`Cells`].
    Cells(CellsParams),
    /// [`Grains`].
    Grains(GrainsParams),
    /// [`HeightField`].
    HeightField(HeightFieldParams),
    /// [`Beads`].
    Beads(BeadsParams),
    /// [`ImageWorld`].
    Image(ImageParams),
}

impl WorldInfo {
    /// The world a configuration's `[sim.world]` names, with its seed and default parameters.
    /// An image's path is used as it is; `siq-config` has resolved it.
    pub fn from_config(world: &WorldConfig, seed: u64) -> Self {
        match world {
            WorldConfig::Cells => Self::Cells(CellsParams {
                seed,
                ..CellsParams::default()
            }),
            WorldConfig::Grains => Self::Grains(GrainsParams {
                seed,
                ..GrainsParams::default()
            }),
            WorldConfig::HeightField => Self::HeightField(HeightFieldParams {
                seed,
                ..HeightFieldParams::default()
            }),
            WorldConfig::Beads => Self::Beads(BeadsParams {
                seed,
                ..BeadsParams::default()
            }),
            WorldConfig::Image { path, pixel_size } => Self::Image(ImageParams {
                path: path.clone(),
                pixel_size: *pixel_size,
            }),
        }
    }

    /// The world. An image world reads its file.
    pub fn build(&self) -> Result<Box<dyn World>, SynthError> {
        Ok(match self {
            Self::Cells(params) => Box::new(Cells::new(*params)),
            Self::Grains(params) => Box::new(Grains::new(*params)),
            Self::HeightField(params) => Box::new(HeightField::new(*params)),
            Self::Beads(params) => Box::new(Beads::new(*params)),
            Self::Image(params) => Box::new(ImageWorld::open(params.clone())?),
        })
    }
}

/// A height map in a file of raw little-endian `f32` heights in µm, row after row.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct HeightMap {
    /// The file's name, in the ground truth's folder.
    pub file: String,
    /// The number of columns.
    pub width: u32,
    /// The number of rows.
    pub height: u32,
    /// The pixel size: pixel `(column, row)` is the height at the region's origin plus
    /// `(column + 0.5, row + 0.5)` times it.
    pub scale: Micrometers,
}

impl HeightMap {
    /// The heights, read from `dir`.
    pub fn read(&self, dir: &Path) -> Result<Vec<f32>, SynthError> {
        let path = dir.join(&self.file);
        let bytes = fs::read(&path).map_err(|source| SynthError::Io { path, source })?;
        let (heights, _) = bytes.as_chunks();
        Ok(heights.iter().map(|&b| f32::from_le_bytes(b)).collect())
    }
}

/// The ground truth of a region of a world.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct GroundTruth {
    /// The world, which [`WorldInfo::build`] makes again.
    pub world: WorldInfo,
    /// The region.
    pub region: RectUm,
    /// The surface's height over the region, if the world has a surface.
    pub height_map: Option<HeightMap>,
    /// The beads in the region, if the world is made of beads.
    pub beads: Vec<Bead>,
}

/// Writes the ground truth of `region` of `world` into `dir`: `<name>.json`, and, if the world
/// has a surface, its heights sampled every `scale` in `<name>.height.f32`.
pub fn write_ground_truth<W: World + ?Sized>(
    world: &W,
    region: RectUm,
    scale: Micrometers,
    dir: &Path,
    name: &str,
) -> Result<GroundTruth, SynthError> {
    let write = |file: String, bytes: Vec<u8>| {
        let path = dir.join(file);
        fs::write(&path, bytes).map_err(|source| SynthError::Io { path, source })
    };
    let [width, height] = region
        .size
        .to_um()
        .map(|size| (size / scale.0).ceil() as u32);
    let to_stage = grid(region.origin, scale);
    let heights: Option<Vec<f32>> = (0..height)
        .flat_map(|row| (0..width).map(move |column| PointPx::pixel_centre(column, row)))
        .map(|pixel| world.surface_z(to_stage.apply(pixel)).map(|z| z.0 as f32))
        .collect();
    let height_map = match heights {
        Some(heights) if !heights.is_empty() => {
            let file = format!("{name}.height.f32");
            write(
                file.clone(),
                heights.iter().flat_map(|z| z.to_le_bytes()).collect(),
            )?;
            Some(HeightMap {
                file,
                width,
                height,
                scale,
            })
        }
        _ => None,
    };
    let truth = GroundTruth {
        world: world.info(),
        region,
        height_map,
        beads: world.beads(region),
    };
    let json = serde_json::to_vec_pretty(&truth).expect("ground truth is plain data");
    write(format!("{name}.json"), json)?;
    Ok(truth)
}
