//! The GPU foundation (ADR-0058) on this machine's GPU, each test skipped with its reason where
//! there is none: the adapter chosen and its limits, uploads read back, the LUT against its CPU
//! reference, a dispatch over 65 535 workgroups, timestamps, and a job's failure, panic and lost
//! device; and the WGSL validated by naga, which needs no GPU. REQ-STI-001, REQ-3D-005, REQ-NFR-002.

use std::sync::Arc;

use siq_gpu::{Compute, GpuError, GpuHandle, GpuOptions, adapters, lut};

/// The GPU, or `None` with the reason said.
fn gpu() -> Option<GpuHandle> {
    match GpuHandle::open(&GpuOptions::default()) {
        Ok(handle) => Some(handle),
        Err(error) => {
            eprintln!("skipped: {error}");
            None
        }
    }
}

/// Samples that vary in every bit, the same each run.
fn samples(count: usize) -> Vec<u16> {
    let mut state = 0x2545_f491_u32;
    (0..count)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            (state >> 7) as u16
        })
        .collect()
}

/// A gamma of 0.5 from 14 bits to 8.
fn table() -> Arc<lut::Table> {
    let mut table = [0u8; 65_536];
    for (value, entry) in table.iter_mut().enumerate() {
        *entry = (255.0 * (value.min(16_383) as f64 / 16_383.0).sqrt()).round() as u8;
    }
    Arc::new(table)
}

#[test]
fn the_wgsl_validates() {
    for shader in siq_gpu::SHADERS {
        let module = naga::front::wgsl::parse_str(shader.source).unwrap_or_else(|error| {
            panic!("{}: {}", shader.name, error.emit_to_string(shader.source))
        });
        let mut validator = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        );
        validator
            .validate(&module)
            .unwrap_or_else(|error| panic!("{}: {error:?}", shader.name));
        assert!(
            module
                .entry_points
                .iter()
                .any(|entry| entry.name == shader.entry)
        );
    }
}

#[test]
fn the_best_adapter_is_chosen_with_its_own_limits() {
    let Some(handle) = gpu() else { return };
    let all = adapters(&GpuOptions::default());
    for adapter in &all {
        eprintln!("{adapter:?}");
    }
    // The one opened is the first: a discrete GPU ahead of an integrated one, NVIDIA's first.
    let info = handle.info();
    assert_eq!(info.name, all[0].name);
    let kinds: Vec<_> = all
        .iter()
        .map(|adapter| adapter.device_type.as_str())
        .collect();
    if let Some(discrete) = kinds.iter().position(|&kind| kind == "DiscreteGpu") {
        assert!(
            kinds[..discrete].iter().all(|&kind| kind == "DiscreteGpu"),
            "{kinds:?}"
        );
    }
    // The device has the adapter's limits, not wgpu's defaults.
    let limits = handle.run_blocking(|gpu| Ok(gpu.device.limits())).unwrap();
    assert_eq!(siq_gpu::LimitsReport::from(&limits), info.limits);
    let defaults = wgpu_defaults();
    assert!(
        info.limits.max_storage_buffer_binding_size >= defaults.max_storage_buffer_binding_size
    );
    assert!(info.limits.max_texture_dimension_2d >= defaults.max_texture_dimension_2d);
}

fn wgpu_defaults() -> siq_gpu::LimitsReport {
    siq_gpu::LimitsReport::from(&siq_gpu::wgpu::Limits::default())
}

#[test]
fn samples_uploaded_are_read_back_as_they_were() {
    let Some(handle) = gpu() else { return };
    // An odd count: the last word's high half is 0.
    let sent = samples(1001);
    let expected = sent.clone();
    let back = handle
        .run_blocking(move |gpu| {
            let buffer = gpu.upload_u16(&sent)?;
            gpu.read(&buffer, 1002 * 2)
        })
        .unwrap();
    let back: Vec<u16> = back
        .chunks(2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .collect();
    assert_eq!(&back[..1001], &expected[..]);
    assert_eq!(back[1001], 0);

    // As an R16Uint texture, 37 × 13: copied out a row of 256 bytes at a time.
    let (width, height) = (37u32, 13u32);
    let sent = samples((width * height) as usize);
    let expected = sent.clone();
    let rows = handle
        .run_blocking(move |gpu| {
            let texture = gpu.upload_u16_texture(&sent, width, height)?;
            let buffer = gpu.device.create_buffer(&siq_gpu::wgpu::BufferDescriptor {
                label: None,
                size: 256 * u64::from(height),
                usage: siq_gpu::wgpu::BufferUsages::COPY_DST
                    | siq_gpu::wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            });
            let mut encoder = gpu.device.create_command_encoder(&Default::default());
            encoder.copy_texture_to_buffer(
                texture.as_image_copy(),
                siq_gpu::wgpu::TexelCopyBufferInfo {
                    buffer: &buffer,
                    layout: siq_gpu::wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(256),
                        rows_per_image: Some(height),
                    },
                },
                texture.size(),
            );
            gpu.queue.submit([encoder.finish()]);
            gpu.read(&buffer, 256 * u64::from(height))
        })
        .unwrap();
    for row in 0..height as usize {
        let line: Vec<u16> = rows[row * 256..row * 256 + width as usize * 2]
            .chunks(2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .collect();
        assert_eq!(
            line,
            expected[row * width as usize..(row + 1) * width as usize]
        );
    }
}

#[test]
fn the_lut_maps_as_its_cpu_reference_does() {
    let Some(handle) = gpu() else { return };
    let table = table();
    for count in [1, 3, 4, 5, 1_000_003] {
        let sent: Arc<[u16]> = samples(count).into();
        let expected = lut::cpu(&sent, &table);
        let (gpu_sent, gpu_table) = (Arc::clone(&sent), Arc::clone(&table));
        let (mapped, took) = handle
            .run_blocking(move |gpu| lut::gpu(gpu, &gpu_sent, &gpu_table))
            .unwrap();
        assert_eq!(mapped, expected, "{count} samples");
        // Timed where the adapter can.
        assert_eq!(took.is_some(), handle.info().timestamps);
    }
    // Through `Compute`, on the GPU and on the CPU alike.
    let sent: Arc<[u16]> = samples(10_000).into();
    let expected = lut::cpu(&sent, &table);
    for compute in [
        Compute::Gpu(handle),
        Compute::Cpu {
            why: "a test".into(),
        },
    ] {
        let mapped = futures::executor::block_on(lut::apply(
            &compute,
            Arc::clone(&sent),
            Arc::clone(&table),
        ));
        assert_eq!(mapped.unwrap(), expected);
    }
}

#[test]
fn a_dispatch_of_more_workgroups_than_a_row_holds_goes_down_too() {
    let Some(handle) = gpu() else { return };
    // 65 535 × 256 words of 4 samples, and 4 096 more: two rows of workgroups. 135 MB of samples.
    let per_row = 65_535 * 256 * 4;
    let count = per_row + 4_096 * 4;
    let bytes = count as u64 * 2;
    if handle.info().limits.max_storage_buffer_binding_size < bytes {
        eprintln!("skipped: the adapter binds less than {bytes} bytes");
        return;
    }
    let table = table();
    let sent: Arc<[u16]> = samples(count).into();
    let expected = lut::cpu(&sent, &table);
    let (mapped, took) = handle
        .run_blocking(move |gpu| lut::gpu(gpu, &sent, &table))
        .unwrap();
    assert_eq!(mapped.len(), expected.len());
    assert!(mapped == expected, "the second row of workgroups maps too");
    eprintln!("{count} samples mapped in {took:?}");
}

#[test]
fn a_job_that_fails_or_panics_fails_alone_and_a_lost_device_fails_every_job_after() {
    let Some(handle) = gpu() else { return };
    // A copy the GPU rejects: a size not a multiple of 4.
    let rejected = handle.run_blocking(|gpu| {
        let buffer = gpu.upload_u16(&[1, 2, 3, 4])?;
        let other = gpu.upload_u16(&[0; 4])?;
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        encoder.copy_buffer_to_buffer(&buffer, 0, &other, 0, 3);
        gpu.queue.submit([encoder.finish()]);
        Ok(())
    });
    assert!(
        matches!(rejected, Err(GpuError::Rejected(_))),
        "{rejected:?}"
    );
    let panicked = handle.run_blocking(|_| -> Result<(), GpuError> { panic!("on purpose") });
    assert_eq!(panicked, Err(GpuError::Panicked("on purpose".into())));
    // The thread goes on.
    let fine = handle.run_blocking(|gpu| gpu.read(&gpu.upload_u16(&[7, 8])?, 4));
    assert_eq!(fine.unwrap(), [7, 0, 8, 0]);
    // Its device destroyed: lost, and every job after fails so.
    handle
        .run_blocking(|gpu| {
            gpu.device.destroy();
            gpu.wait().or(Ok(()))
        })
        .unwrap();
    assert!(handle.lost().is_some());
    let after = handle.run_blocking(|_| Ok(()));
    assert!(matches!(after, Err(GpuError::Lost(_))), "{after:?}");
}

#[test]
fn the_cpu_is_used_when_asked_for() {
    let options = GpuOptions {
        cpu: true,
        ..GpuOptions::default()
    };
    let Compute::Cpu { why } = Compute::open(&options) else {
        panic!("the CPU was asked for")
    };
    assert_eq!(why, "the CPU was asked for");
}
