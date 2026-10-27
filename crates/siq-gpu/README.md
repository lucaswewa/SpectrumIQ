# siq-gpu

SpectrumIQ's GPU foundation (ADR-0058), on wgpu: what live stitching (M7) and the 3D kernels
(M11, M12) share.

- **The adapter:** DX12 and Vulkan, Windows' own. A discrete GPU first, NVIDIA's before another
  vendor's, then integrated, virtual and software ones. The device is made with the adapter's own
  limits: wgpu's default storage binding, 128 MB, is less than a 100 MP frame of u16.
- **One thread owns the device and its queue.** A `GpuHandle`, cheap to clone, sends it jobs:
  closures over a `Gpu`, run one at a time, each in error scopes. A command the GPU rejects, or
  its running out of memory, fails that job alone; a job that panics fails, and the thread goes
  on. A lost device is recorded, and every job after fails with it.
- **On the thread:**
  - u16 samples uploaded two to a 32-bit word, the first in the low half, or as an `R16Uint`
    texture;
  - results read back through staging buffers kept for reuse, four of each size;
  - compute pipelines made once each, by shader;
  - passes timed with timestamp queries, where the adapter has them;
  - dispatches across, then down, past the 65 535 workgroups a dimension allows.
- **The CPU fallback:** `Compute` is the GPU, or the CPU when there is none or it isn't wanted,
  with the reason. Each kernel has a CPU reference, which the tests hold the GPU to.
- **Kernels:** `lut`, the display LUT, each u16 sample to its u8.

## Place in the architecture

- **Layer:** L1 Core.
- **Depends on:** `wgpu` (DX12, Vulkan and WGSL only), `bytemuck`, `futures` (its executor and
  oneshot), `schemars`, `serde`, `thiserror`, `tracing`; `naga` in the tests.
- **Used by:** `siq-stitch-live`, the live canvas (P7.2), and its viewport (P7.3); later the GPU
  depth from focus (P11.2) and deconvolution (P12.3).
- **Phase:** P7.1.

## API

| Item | What |
|---|---|
| `GpuHandle::open(&options)` | The best adapter opened on a thread of its own, or `GpuError::NoAdapter` |
| `handle.run(work).await`, `run_blocking(work)` | `work`, a `FnOnce(&mut Gpu) -> Result<R, GpuError>`, run on the GPU's thread after the jobs before it |
| `handle.info()`, `handle.lost()` | The adapter's `AdapterReport`; why the device was lost, if it was |
| `Gpu` | `device`, `queue`; `pipeline(&shader)`; `upload_u16(&samples)`, `upload_u16_texture(&samples, width, height)`; `read(&buffer, bytes)`, `read_at(&buffer, offset, bytes)`; `timed_pass(label, record)` → the GPU's time if it can say; `groups(invocations, per_group)` → `[across, down]`; `fits_binding(bytes, what)`; `wait()` |
| `Shader { name, source, entry }` | A compute shader, its WGSL `include_str!`d |
| `adapters(&options)` | Every adapter, best first: `AdapterReport { name, backend, device_type, vendor, driver, timestamps, limits }` |
| `LimitsReport` | `max_storage_buffer_binding_size`, `max_buffer_size`, `max_texture_dimension_2d`, `max_compute_workgroups_per_dimension`, `max_compute_invocations_per_workgroup` |
| `GpuOptions { cpu, software }` | The CPU instead of a GPU; the software adapter (WARP) instead of a GPU |
| `Compute::open(&options)` | `Gpu(handle)`, or `Cpu { why }` |
| `GpuError` | `NoAdapter`, `Device`, `Lost`, `Rejected`, `TooLarge`, `Panicked`, `Failed`, `Closed` |
| `lut::gpu(&mut gpu, &samples, &table)`, `lut::cpu(&samples, &table)`, `lut::apply(&compute, samples, table).await` | Samples through a 65 536-entry table, on the GPU (with its pass's time), the CPU, or where `compute` says; `lut::SHADER` |

**On this workstation** (`gpu_info`): the RTX 4060 on Vulkan binds 2 GB of storage and takes 32 768
texels a side. A 100 MP frame's LUT pass takes 1.2 ms; with its 200 MB upload and readback the job
takes about 136 ms, against 33 ms on one CPU thread. A kernel pays when its data stays on the GPU,
as the live canvas's does.

## Examples

| Example | What it shows | Run |
|---|---|---|
| `gpu_info` | Every adapter, best first, with its limits; the one opened; a 100 MP frame through the LUT on it and on the CPU, timed and compared (1 MP in smoke mode) | `cargo run --release -p siq-gpu --example gpu_info [-- --software]` |

## Features

None. wgpu is built with DX12, Vulkan and WGSL only: not GL, Metal or WebGPU.

## Testing

`cargo test -p siq-gpu` runs on this machine's GPU, each test skipped with its reason where there is
none:

- the WGSL parsed and validated by naga, with no GPU;
- the adapter opened the first of `adapters`, discrete ahead of integrated, with its own limits;
- samples uploaded, an odd count, and as a 37 × 13 texture, read back as they were;
- the LUT as its CPU reference at 1 to a million samples, timed where the adapter can, and through
  `Compute` on the GPU and the CPU;
- a dispatch of 67 million samples, two rows of workgroups;
- a rejected copy and a panic, each failing its job alone, then a destroyed device failing every
  job after;
- the CPU when asked for.

## Limitations

- **One thread, one job at a time:** a long kernel holds up the ones behind it.
- **Uploads go through the queue's own staging,** `write_buffer`: no ring of ours.
- **A lost device isn't remade:** the handle fails every job after, and its owner opens another.
- **No GPU in CI elsewhere:** the tests skip; `--software` runs them on WARP by hand.
