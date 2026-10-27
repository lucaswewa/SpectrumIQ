# ADR-0058: wgpu usage: backends, buffers and textures, limits, and threading

- **Status:** Accepted
- **Date:** 2026-10-27
- **Phase:** P7.1
- **Related:** ADR-0005, ADR-0043, REQ-STI-001, REQ-3D-005, REQ-NFR-002; the plan's risk R4

## Context

- **The GPU work ahead:** the live mosaic paints each frame on a paged canvas and renders the
  client's viewport (M7); depth from focus and deconvolution run on volumes (M11, M12). They should
  share one way to reach the GPU.
- **The workstation** has an NVIDIA RTX 4060 and an AMD 780M, on Windows 11 (ADR-0005). Both are
  seen through DX12 and Vulkan, and Windows adds WARP, a software DX12 adapter.
- **A 100 MP frame of u16 is 200 MB.** wgpu's default limits bind 128 MB of storage and textures
  of 8 192 texels a side (risk R4).
- **wgpu's `Device` and `Queue` are `Send` and `Sync`,** but its errors are reported through
  scopes that are per thread, and its default for an error outside one is to panic.
- **The job scheduler** (ADR-0043) admits one GPU job at a time, by its VRAM budget. No job uses the
  GPU yet.

## Decision

- **wgpu 30, with DX12, Vulkan and WGSL:** not GL, Metal or WebGPU, which this platform doesn't
  need. naga validates the WGSL in the tests.
- **The adapter:** of those both backends offer, a discrete GPU first, NVIDIA's before another
  vendor's, then integrated, virtual and software ones; the backends' order otherwise. On this
  workstation, the RTX 4060 on Vulkan. `software` asks for the fallback adapter, WARP, for
  machines without a GPU.
- **Limits:** the device is made with the adapter's own, not wgpu's defaults, and they are
  reported: the storage binding, the buffer, the 2D texture, the workgroups a dimension and the
  invocations a workgroup. Here, 2 GB of storage binding and 32 768 texels. A kernel checks what it
  binds against them (`TooLarge`) rather than failing in the driver. Timestamp queries are
  requested when the adapter has them.
- **One thread owns the device and queue.** A `GpuHandle` sends it jobs, closures over a `Gpu`, one
  at a time, and each caller awaits its job's result. So:
  - **each job runs in error scopes,** validation and out-of-memory, pushed and popped on that
    thread: a rejected command fails its job alone. An error outside a job is logged, not a
    panic;
  - **a job's panic is caught:** the job fails, and the thread goes on;
  - **a lost device is recorded** by its callback, and every job after fails with the reason. It
    isn't remade: its owner opens another.
  - The scheduler's one GPU job at a time stays its admission, by VRAM; a GPU job's work goes
    to the handle.
- **Buffers, not textures, for compute:** u16 samples go two to a 32-bit word in a storage buffer,
  the first in the low half, as their little-endian bytes lie. Storage buffers bind 2 GB here,
  index linearly, and need no texture format. `R16Uint` textures are there for kernels that want
  2D addressing; they can't be filtered, so a kernel samples them itself.
- **Staging:** uploads go through the queue (`write_buffer`, `write_texture`), whose own staging
  wgpu manages. Results come back through readback buffers kept for reuse, four of each power of
  two.
- **Pipelines** are made once each, by shader name, with the layout the shader implies.
- **Dispatches** go across, then down, once one row of workgroups would pass the 65 535 a dimension
  allows: a shader finds its index as `id.x + id.y × groups.x × per_group`.
- **The CPU fallback:** `Compute` is the GPU or the CPU, with the reason. Every kernel has a CPU
  reference, used when there is no GPU and held to by the tests.

## Alternatives considered

- **wgpu's default features:** GL through ANGLE or EGL, Metal and WebGPU add crates and a backend
  this platform never uses.
- **CUDA, through cudarc:** the NVIDIA GPU alone, a toolkit to install, and no WGSL to share with
  other vendors. wgpu reaches both GPUs here, and needs only the driver.
- **The device used from any thread:** wgpu allows it, but a thread's error scopes would catch
  another's errors, and a validation error would panic wherever it happened.
- **Textures for frames:** `R16Uint` can't be filtered or written as storage everywhere, and
  textures are capped at 16 384 texels on DX12. Buffers bind a 100 MP frame whole.
- **A staging ring of our own for uploads:** wgpu's queue already stages each write; one of ours
  would map and copy again.
- **Remaking a lost device in place:** every buffer and pipeline the jobs hold would be the old
  device's. Failing plainly, and opening another, keeps that visible.

## Consequences

- **A 100 MP frame binds whole** on both GPUs here. A GPU with less leaves `TooLarge`, and a kernel
  that bands it.
- **The upload is the cost:** a 100 MP frame's LUT pass takes 1.2 ms on the RTX 4060, but the job,
  with its 200 MB upload and its readback, about 136 ms, against 33 ms on one CPU thread. The live
  canvas keeps its pages on the GPU, so each frame is uploaded once.
- **GPU jobs queue behind each other:** a long kernel holds up a short one. The mosaic's painting
  and its viewport share the thread with any job.
- **Tests run on the machine's GPU,** and skip, saying why, without one.

## Verification

- `crates/siq-gpu/tests/gpu.rs`: the WGSL validated; the adapter chosen, with its limits; uploads
  read back; the LUT against its CPU reference, timed; a dispatch of two rows of workgroups; a
  rejected job, a panic, and a lost device.
- `gpu_info`: every adapter, the one opened, and a 100 MP frame mapped and compared.
