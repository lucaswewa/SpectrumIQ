//! SpectrumIQ's GPU foundation (ADR-0058), on wgpu: what live stitching (M7) and the 3D kernels
//! (M11, M12) share.
//!
//! - **The adapter:** DX12 and Vulkan; a discrete GPU first, NVIDIA's before others, then
//!   integrated, virtual and software ones. The device is made with the adapter's own limits,
//!   not wgpu's defaults: a 100 MP frame of u16 is 200 MB, beyond the default storage binding.
//!   [`adapters`] reports each, with the limits that matter.
//! - **One thread owns the device and its queue.** A [`GpuHandle`] sends it jobs, closures over a
//!   [`Gpu`], and awaits each one's result. Each job runs in an error scope, so a rejected
//!   command fails that job alone; a job that panics fails, and the thread goes on. A lost device
//!   is recorded, and every job after fails with it.
//! - **On the thread:** u16 samples uploaded two to a 32-bit word, or as an `R16Uint` texture;
//!   results read back through staging buffers kept for reuse; compute pipelines made once each;
//!   and passes timed, where the adapter has timestamp queries.
//! - **The CPU fallback:** [`Compute`] is the GPU, or the CPU when there is none or it isn't
//!   wanted. Each kernel has its CPU reference, which the tests hold the GPU to.
//! - **Kernels:** [`lut`], the display LUT from u16 to u8.

mod thread;

pub mod lut;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub use thread::{Gpu, GpuHandle, Shader};
pub use wgpu;

/// Every shader here: the tests validate each.
pub const SHADERS: &[Shader] = &[lut::SHADER];

/// What went wrong on the GPU.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GpuError {
    /// No adapter could be had.
    #[error("no GPU: {0}")]
    NoAdapter(String),
    /// The adapter wouldn't make a device.
    #[error("the GPU device couldn't be made: {0}")]
    Device(String),
    /// The device was lost: removed, reset, or destroyed.
    #[error("the GPU device was lost: {0}")]
    Lost(String),
    /// The GPU rejected a command, or ran out of memory.
    #[error("the GPU rejected the work: {0}")]
    Rejected(String),
    /// More than the adapter allows.
    #[error("{0}")]
    TooLarge(String),
    /// The job panicked.
    #[error("the GPU job panicked: {0}")]
    Panicked(String),
    /// Waiting on the GPU, or mapping its memory, failed.
    #[error("the GPU failed: {0}")]
    Failed(String),
    /// The GPU's thread has stopped.
    #[error("the GPU's thread has stopped")]
    Closed,
}

/// How the GPU is chosen.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct GpuOptions {
    /// No GPU: the kernels run on the CPU.
    pub cpu: bool,
    /// The backend's software adapter (WARP on DX12) rather than a GPU: for machines without one.
    pub software: bool,
}

/// An adapter, as it reports itself.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AdapterReport {
    /// The adapter's name.
    pub name: String,
    /// `Dx12` or `Vulkan`.
    pub backend: String,
    /// `DiscreteGpu`, `IntegratedGpu`, `VirtualGpu`, `Cpu` or `Other`.
    pub device_type: String,
    /// The PCI vendor: 0x10de is NVIDIA.
    pub vendor: u32,
    /// The driver and its version.
    pub driver: String,
    /// Whether passes can be timed.
    pub timestamps: bool,
    /// The limits that matter here.
    pub limits: LimitsReport,
}

/// The limits that matter for 100 MP frames and their kernels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct LimitsReport {
    /// The largest storage buffer one binding can see, in bytes.
    pub max_storage_buffer_binding_size: u64,
    /// The largest buffer, in bytes.
    pub max_buffer_size: u64,
    /// The widest and tallest 2D texture, in texels.
    pub max_texture_dimension_2d: u32,
    /// The most workgroups a dispatch has along each dimension.
    pub max_compute_workgroups_per_dimension: u32,
    /// The most invocations a workgroup has.
    pub max_compute_invocations_per_workgroup: u32,
}

impl From<&wgpu::Limits> for LimitsReport {
    fn from(limits: &wgpu::Limits) -> Self {
        Self {
            max_storage_buffer_binding_size: limits.max_storage_buffer_binding_size,
            max_buffer_size: limits.max_buffer_size,
            max_texture_dimension_2d: limits.max_texture_dimension_2d,
            max_compute_workgroups_per_dimension: limits.max_compute_workgroups_per_dimension,
            max_compute_invocations_per_workgroup: limits.max_compute_invocations_per_workgroup,
        }
    }
}

const NVIDIA: u32 = 0x10de;

/// The backends used: Windows' own.
const BACKENDS: wgpu::Backends = wgpu::Backends::DX12.union(wgpu::Backends::VULKAN);

fn report(adapter: &wgpu::Adapter) -> AdapterReport {
    let info = adapter.get_info();
    AdapterReport {
        name: info.name,
        backend: format!("{:?}", info.backend),
        device_type: format!("{:?}", info.device_type),
        vendor: info.vendor,
        driver: format!("{} {}", info.driver, info.driver_info)
            .trim()
            .to_owned(),
        timestamps: adapter.features().contains(wgpu::Features::TIMESTAMP_QUERY),
        limits: LimitsReport::from(&adapter.limits()),
    }
}

/// How an adapter ranks: lower first. A discrete GPU before an integrated one, NVIDIA's before
/// another vendor's, then the backend's order.
fn rank(info: &wgpu::AdapterInfo) -> (u8, bool) {
    let kind = match info.device_type {
        wgpu::DeviceType::DiscreteGpu => 0,
        wgpu::DeviceType::IntegratedGpu => 1,
        wgpu::DeviceType::VirtualGpu => 2,
        wgpu::DeviceType::Cpu => 3,
        wgpu::DeviceType::Other => 4,
    };
    (kind, info.vendor != NVIDIA)
}

fn instance() -> wgpu::Instance {
    wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: BACKENDS,
        ..wgpu::InstanceDescriptor::new_without_display_handle_from_env()
    })
}

/// The adapters for `options`, best first: the software one alone if it asks for that.
fn ranked(instance: &wgpu::Instance, options: &GpuOptions) -> Vec<wgpu::Adapter> {
    if options.software {
        let request = wgpu::RequestAdapterOptions {
            force_fallback_adapter: true,
            ..wgpu::RequestAdapterOptions::default()
        };
        return futures::executor::block_on(instance.request_adapter(&request))
            .into_iter()
            .collect();
    }
    let mut adapters = futures::executor::block_on(instance.enumerate_adapters(BACKENDS));
    adapters.sort_by_key(|adapter| rank(&adapter.get_info()));
    adapters
}

/// Every adapter there is for `options`, in the order one is chosen.
pub fn adapters(options: &GpuOptions) -> Vec<AdapterReport> {
    ranked(&instance(), options).iter().map(report).collect()
}

/// Where kernels run: the GPU, or the CPU, and why.
#[derive(Clone)]
pub enum Compute {
    /// The GPU.
    Gpu(GpuHandle),
    /// The CPU.
    Cpu {
        /// Why not the GPU.
        why: String,
    },
}

impl Compute {
    /// The GPU of `options`, or the CPU if they say so or there is none: it says why.
    pub fn open(options: &GpuOptions) -> Self {
        if options.cpu {
            return Self::Cpu {
                why: "the CPU was asked for".to_owned(),
            };
        }
        match GpuHandle::open(options) {
            Ok(handle) => Self::Gpu(handle),
            Err(error) => {
                tracing::warn!(%error, "the kernels run on the CPU");
                Self::Cpu {
                    why: error.to_string(),
                }
            }
        }
    }
}
