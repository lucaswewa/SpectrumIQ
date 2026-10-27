//! The paint kernel (ADR-0059), and its CPU reference: a frame laid over one page of the canvas.

use bytemuck::{Pod, Zeroable};
use siq_gpu::wgpu::util::DeviceExt;
use siq_gpu::{Gpu, GpuError, Shader, wgpu};
use siq_types::Affine2;

/// The paint's compute shader.
pub const SHADER: Shader = Shader {
    name: "paint",
    source: include_str!("shaders/paint.wgsl"),
    entry: "main",
};

/// A frame to paint: its samples, binned and flat-corrected, row by row.
#[derive(Debug, Clone, Copy)]
pub struct Frame<'a> {
    /// The samples.
    pub samples: &'a [u16],
    /// Its width, in pixels.
    pub width: u32,
    /// Its height, in pixels.
    pub height: u32,
}

impl Frame<'_> {
    /// Its corners on the canvas, by `m`, and their box: `[x0, y0, x1, y1]`.
    pub fn footprint(&self, m: &Affine2) -> [f64; 4] {
        let (w, h) = (f64::from(self.width), f64::from(self.height));
        let corners = [[0.0, 0.0], [w, 0.0], [w, h], [0.0, h]].map(|corner| m.apply(corner));
        let fold = |axis: usize, pick: fn(f64, f64) -> f64, from: f64| {
            corners
                .iter()
                .fold(from, |at, corner| pick(at, corner[axis]))
        };
        [
            fold(0, f64::min, f64::MAX),
            fold(1, f64::min, f64::MAX),
            fold(0, f64::max, f64::MIN),
            fold(1, f64::max, f64::MIN),
        ]
    }
}

/// One page's paint, as the kernel takes it: the page's pixels to the frame's, the frame's size,
/// the rectangle of the page the frame covers, the feather and the page's side.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
pub struct Params {
    row0: [f32; 4],
    row1: [f32; 4],
    frame_size: [u32; 2],
    rect_origin: [u32; 2],
    rect_size: [u32; 2],
    feather: f32,
    page: u32,
}

impl Params {
    /// The paint of `frame`, mapped by `m` (frame pixels to canvas pixels), on the page whose
    /// corner is canvas pixel `origin` and whose side is `side` pixels, feathered `feather` frame
    /// pixels in, at most an eighth of its shorter side: `None` if the frame misses the page, or
    /// `m` is singular.
    pub fn for_page(
        frame: &Frame<'_>,
        m: &Affine2,
        origin: [i64; 2],
        side: u32,
        feather: f32,
    ) -> Option<Self> {
        let [x0, y0, x1, y1] = frame.footprint(m);
        let clip = |low: f64, high: f64, at: i64| {
            let from = (low.floor() as i64 - at).clamp(0, i64::from(side));
            let to = (high.ceil() as i64 - at).clamp(0, i64::from(side));
            (from as u32, (to - from) as u32)
        };
        let ((rx, rw), (ry, rh)) = (clip(x0, x1, origin[0]), clip(y0, y1, origin[1]));
        if rw == 0 || rh == 0 {
            return None;
        }
        // The page's own pixels, in f64 until the numbers are a frame's: f32 keeps them exact.
        let shift = Affine2::translation([origin[0] as f64, origin[1] as f64]);
        let [[a, b, tx], [c, d, ty]] = m.invert()?.compose(&shift).rows();
        Some(Self {
            row0: [a as f32, b as f32, tx as f32, 0.0],
            row1: [c as f32, d as f32, ty as f32, 0.0],
            frame_size: [frame.width, frame.height],
            rect_origin: [rx, ry],
            rect_size: [rw, rh],
            feather: feather
                .min(frame.width.min(frame.height) as f32 / 8.0)
                .max(1e-3),
            page: side,
        })
    }
}

/// `samples` painted on `page` by `params`, as the kernel paints it.
pub fn cpu(page: &mut [u32], samples: &[u16], params: &Params) {
    let [width, height] = params.frame_size;
    let sample = |x: i32, y: i32| {
        let cx = x.clamp(0, width as i32 - 1) as u32;
        let cy = y.clamp(0, height as i32 - 1) as u32;
        f32::from(samples[(cy * width + cx) as usize])
    };
    let (r0, r1) = (params.row0, params.row1);
    for ly in 0..params.rect_size[1] {
        for lx in 0..params.rect_size[0] {
            let px = [params.rect_origin[0] + lx, params.rect_origin[1] + ly];
            let centre = [px[0] as f32 + 0.5, px[1] as f32 + 0.5];
            let f = [
                r0[0] * centre[0] + r0[1] * centre[1] + r0[2],
                r1[0] * centre[0] + r1[1] * centre[1] + r1[2],
            ];
            let (w, h) = (width as f32, height as f32);
            let edge = f[0].min(w - f[0]).min(f[1].min(h - f[1]));
            if edge <= 0.0 {
                continue;
            }
            let alpha = (edge / params.feather).min(1.0);
            let g = [f[0] - 0.5, f[1] - 0.5];
            let base = [g[0].floor(), g[1].floor()];
            let t = [g[0] - base[0], g[1] - base[1]];
            let (x0, y0) = (base[0] as i32, base[1] as i32);
            let top = sample(x0, y0) * (1.0 - t[0]) + sample(x0 + 1, y0) * t[0];
            let bottom = sample(x0, y0 + 1) * (1.0 - t[0]) + sample(x0 + 1, y0 + 1) * t[0];
            let value = top * (1.0 - t[1]) + bottom * t[1];
            let at = (px[1] * params.page + px[0]) as usize;
            let (old_value, old_cover) = (
                f32::from(page[at] as u16),
                (page[at] >> 16) as f32 / 65_535.0,
            );
            let cover = alpha + old_cover * (1.0 - alpha);
            let blended = (value * alpha + old_value * old_cover * (1.0 - alpha)) / cover;
            let value = blended.min(65_535.0).round_ties_even() as u32;
            page[at] = value | (((cover * 65_535.0).round_ties_even() as u32) << 16);
        }
    }
}

/// The uploaded frame `frame` painted on `page` by `params`, on `gpu`.
pub(crate) fn gpu(
    gpu: &mut Gpu,
    frame: &wgpu::Buffer,
    page: &wgpu::Buffer,
    params: &Params,
) -> Result<(), GpuError> {
    let pipeline = gpu.pipeline(&SHADER);
    let uniform = gpu
        .device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("paint params"),
            contents: bytemuck::bytes_of(params),
            usage: wgpu::BufferUsages::UNIFORM,
        });
    let group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("paint"),
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: frame.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: page.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: uniform.as_entire_binding(),
            },
        ],
    });
    let [w, h] = params.rect_size;
    gpu.timed_pass("paint", |pass| {
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.dispatch_workgroups(w.div_ceil(16), h.div_ceil(16), 1);
    })?;
    Ok(())
}
