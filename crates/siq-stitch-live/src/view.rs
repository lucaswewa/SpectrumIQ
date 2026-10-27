//! The viewport (ADR-0059): what the client looks at, a stage point, a zoom and a size, rendered
//! from the pages through the display's LUT, a byte a pixel, on the GPU.

use bytemuck::{Pod, Zeroable};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use siq_gpu::wgpu::util::DeviceExt;
use siq_gpu::{Gpu, GpuError, Shader, wgpu};

use crate::CanvasGeometry;

/// The viewport's compute shader.
pub const SHADER: Shader = Shader {
    name: "view",
    source: include_str!("shaders/view.wgsl"),
    entry: "main",
};

/// The largest view, width by height.
pub const MAX_SIZE: [u32; 2] = [1920, 1080];

/// The zooms a view may have: from 1/1024 to 64 view pixels a canvas pixel.
pub const ZOOMS: [f64; 2] = [1.0 / 1024.0, 64.0];

/// What the client looks at.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Viewport {
    /// The stage point at its centre, in µm: `[x, y]`.
    pub centre: [f64; 2],
    /// View pixels a canvas pixel: 1 shows the canvas's own detail, ½ twice as much of it.
    pub zoom: f64,
    /// Its width, in pixels: at most 1920.
    pub width: u32,
    /// Its height, in pixels: at most 1080.
    pub height: u32,
}

impl Default for Viewport {
    /// 1280 × 720 about the stage's origin, at a quarter of the canvas's detail.
    fn default() -> Self {
        Self {
            centre: [0.0, 0.0],
            zoom: 0.25,
            width: 1280,
            height: 720,
        }
    }
}

impl Viewport {
    /// Why it can't be shown, if it can't: a size empty or past 1920 × 1080, a zoom outside
    /// [`ZOOMS`], or a centre that isn't finite.
    pub fn check(&self) -> Result<(), String> {
        let [width, height] = MAX_SIZE;
        if !(1..=width).contains(&self.width) || !(1..=height).contains(&self.height) {
            return Err(format!(
                "a view is from 1 × 1 to {width} × {height} pixels, not {} × {}",
                self.width, self.height
            ));
        }
        if !(ZOOMS[0]..=ZOOMS[1]).contains(&self.zoom) {
            return Err(format!("the zoom is from 1/1024 to 64, not {}", self.zoom));
        }
        if !self.centre.iter().all(|at| at.is_finite()) {
            return Err("the centre must be finite".to_owned());
        }
        Ok(())
    }

    /// It on the canvas of `geometry`.
    pub fn on_canvas(&self, geometry: &CanvasGeometry) -> View {
        let step = 1.0 / self.zoom;
        let [x, y] = geometry.to_canvas(self.centre);
        View {
            origin: [
                x - f64::from(self.width) / 2.0 * step,
                y - f64::from(self.height) / 2.0 * step,
            ],
            step,
            size: [self.width, self.height],
        }
    }
}

/// A view on the canvas: the canvas point at its top-left corner, the canvas pixels a side of its
/// pixels spans, and its size.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct View {
    /// Its corner, in canvas pixels.
    pub origin: [f64; 2],
    /// Canvas pixels a view pixel.
    pub step: f64,
    /// Its width and height, in pixels.
    pub size: [u32; 2],
}

impl View {
    /// Where the centre of view pixel `at` (column, row) is on the canvas.
    pub fn to_canvas(&self, [x, y]: [u32; 2]) -> [f64; 2] {
        [
            self.origin[0] + (f64::from(x) + 0.5) * self.step,
            self.origin[1] + (f64::from(y) + 0.5) * self.step,
        ]
    }

    /// Its words a row: four pixels a word.
    pub fn stride(&self) -> u32 {
        self.size[0].div_ceil(4)
    }
}

/// One page's part of a view, as the kernel takes it.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
pub struct Params {
    origin: [f32; 2],
    step: f32,
    side: u32,
    width: u32,
    stride: u32,
    first: [u32; 2],
    count: [u32; 2],
}

impl Params {
    /// `view` on the page whose corner is canvas pixel `corner`, of `side` pixels a side, each
    /// `factor` canvas pixels a side (a shrunk page's are larger): `None` if the view misses it.
    pub fn for_page(view: &View, corner: [i64; 2], side: u32, factor: u32) -> Option<Self> {
        let factor = f64::from(factor.max(1));
        let extent = f64::from(side) * factor;
        // The view's pixels whose centres may fall on the page, with a pixel to spare each side:
        // the kernel decides each.
        let span = |origin: f64, at: i64, size: u32| {
            let from = ((at as f64 - origin) / view.step - 1.0).floor().max(0.0);
            let to = ((at as f64 + extent - origin) / view.step + 1.0)
                .ceil()
                .min(f64::from(size));
            (from as u32, (to - from).max(0.0) as u32)
        };
        let (x, columns) = span(view.origin[0], corner[0], view.size[0]);
        let (y, rows) = span(view.origin[1], corner[1], view.size[1]);
        if columns == 0 || rows == 0 {
            return None;
        }
        let words = (x / 4, (x + columns).div_ceil(4));
        // The page's own pixels, in f64 until the numbers are a page's: f32 keeps them exact.
        let local = |origin: f64, at: i64| ((origin - at as f64) / factor) as f32;
        Some(Self {
            origin: [
                local(view.origin[0], corner[0]),
                local(view.origin[1], corner[1]),
            ],
            step: (view.step / factor) as f32,
            side,
            width: view.size[0],
            stride: view.stride(),
            first: [words.0, y],
            count: [words.1 - words.0, rows],
        })
    }
}

/// The page `page` laid over the view `out` by `params`, through the LUT `lut`, on `gpu`.
pub(crate) fn gpu(
    gpu: &mut Gpu,
    page: &wgpu::Buffer,
    lut: &wgpu::Buffer,
    out: &wgpu::Buffer,
    params: &Params,
) -> Result<(), GpuError> {
    let pipeline = gpu.pipeline(&SHADER);
    let uniform = gpu
        .device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("view params"),
            contents: bytemuck::bytes_of(params),
            usage: wgpu::BufferUsages::UNIFORM,
        });
    let entries: Vec<_> = (0..)
        .zip([page, lut, out, &uniform])
        .map(|(binding, buffer)| wgpu::BindGroupEntry {
            binding,
            resource: buffer.as_entire_binding(),
        })
        .collect();
    let group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("view"),
        layout: &pipeline.get_bind_group_layout(0),
        entries: &entries,
    });
    let [words, rows] = params.count;
    gpu.timed_pass("view", |pass| {
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.dispatch_workgroups(words.div_ceil(16), rows.div_ceil(16), 1);
    })?;
    Ok(())
}
