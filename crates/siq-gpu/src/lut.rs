//! The display LUT: each u16 sample to the u8 a 65 536-entry table gives it, on the GPU or the
//! CPU. The live mosaic's viewport (P7.3) maps its samples so.

use std::sync::Arc;
use std::time::Duration;

use wgpu::util::DeviceExt;

use crate::{Compute, Gpu, GpuError, Shader};

/// The LUT's compute shader.
pub const SHADER: Shader = Shader {
    name: "lut",
    source: include_str!("shaders/lut.wgsl"),
    entry: "main",
};

/// A table of one u8 for each u16 value.
pub type Table = [u8; 65_536];

/// `samples` through `table`, on the CPU: the reference the GPU is held to.
pub fn cpu(samples: &[u16], table: &Table) -> Vec<u8> {
    samples
        .iter()
        .map(|&sample| table[usize::from(sample)])
        .collect()
}

/// `samples` through `table` on `gpu`, and how long the GPU took, if it can say.
pub fn gpu(
    gpu: &mut Gpu,
    samples: &[u16],
    table: &Table,
) -> Result<(Vec<u8>, Option<Duration>), GpuError> {
    let words = samples.len().div_ceil(4).max(1) as u64;
    gpu.fits_binding(words * 4, "the mapped samples")?;
    let input = gpu.upload_u16(samples)?;
    let entries: Vec<u32> = table.iter().map(|&value| u32::from(value)).collect();
    let device = &gpu.device;
    let lut = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("lut table"),
        contents: bytemuck::cast_slice(&entries),
        usage: wgpu::BufferUsages::STORAGE,
    });
    let mapped = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("lut mapped"),
        size: words * 4,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let count = u32::try_from(samples.len())
        .map_err(|_| GpuError::TooLarge(format!("{} samples are too many", samples.len())))?;
    let params = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("lut params"),
        contents: bytemuck::cast_slice(&[count, 0, 0, 0]),
        usage: wgpu::BufferUsages::UNIFORM,
    });
    let pipeline = gpu.pipeline(&SHADER);
    let layout = pipeline.get_bind_group_layout(0);
    let bindings = [&input, &lut, &mapped, &params];
    let entries: Vec<_> = (0..)
        .zip(bindings)
        .map(|(binding, buffer)| wgpu::BindGroupEntry {
            binding,
            resource: buffer.as_entire_binding(),
        })
        .collect();
    let group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("lut"),
        layout: &layout,
        entries: &entries,
    });
    let [across, down] = gpu.groups(words, 256)?;
    let took = gpu.timed_pass("lut", |pass| {
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.dispatch_workgroups(across, down, 1);
    })?;
    let mut bytes = gpu.read(&mapped, words * 4)?;
    bytes.truncate(samples.len());
    Ok((bytes, took))
}

/// `samples` through `table` where `compute` says: on the GPU's thread, or here.
pub async fn apply(
    compute: &Compute,
    samples: Arc<[u16]>,
    table: Arc<Table>,
) -> Result<Vec<u8>, GpuError> {
    match compute {
        Compute::Gpu(handle) => {
            let mapped = handle.run(move |on| gpu(on, &samples, &table)).await?;
            Ok(mapped.0)
        }
        Compute::Cpu { .. } => Ok(cpu(&samples, &table)),
    }
}
