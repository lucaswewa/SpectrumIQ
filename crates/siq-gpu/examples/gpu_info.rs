//! The GPU this machine offers (ADR-0058): every adapter, best first, with its limits; the one
//! opened; and a 100 MP frame of u16 through the display LUT on it and on the CPU, timed and
//! compared.
//!
//! ```text
//! cargo run --release -p siq-gpu --example gpu_info [-- --software]
//! ```
//!
//! `--software` uses the backend's software adapter (WARP on DX12). Without an adapter it says so,
//! and maps on the CPU alone. Smoke mode (`SIQ_SMOKE=1`) maps a 1 MP frame.

use std::sync::Arc;
use std::time::Instant;

use siq_gpu::{Compute, GpuOptions, adapters, lut};

fn main() {
    let smoke = std::env::var_os("SIQ_SMOKE").is_some();
    let options = GpuOptions {
        software: std::env::args().any(|arg| arg == "--software"),
        ..GpuOptions::default()
    };
    for (i, adapter) in adapters(&options).iter().enumerate() {
        let limits = adapter.limits;
        println!(
            "{i}: {} ({}, {}), {}; timestamps {}",
            adapter.name, adapter.backend, adapter.device_type, adapter.driver, adapter.timestamps
        );
        println!(
            "   storage binding {} MB, buffer {} MB, 2D texture {}, {} workgroups a dimension, {} invocations a workgroup",
            limits.max_storage_buffer_binding_size >> 20,
            limits.max_buffer_size >> 20,
            limits.max_texture_dimension_2d,
            limits.max_compute_workgroups_per_dimension,
            limits.max_compute_invocations_per_workgroup
        );
    }

    let (width, height) = if smoke { (1024, 1024) } else { (11_648, 8_742) };
    let count = width * height;
    let samples: Arc<[u16]> = (0..count).map(|i| (i * 7 % 16_384) as u16).collect();
    let mut table = [0u8; 65_536];
    for (value, entry) in table.iter_mut().enumerate() {
        *entry = (255.0 * (value.min(16_383) as f64 / 16_383.0).sqrt()).round() as u8;
    }
    let table = Arc::new(table);

    let began = Instant::now();
    let expected = lut::cpu(&samples, &table);
    let cpu = began.elapsed();
    println!("{width} × {height} through the LUT on the CPU, one thread: {cpu:.2?}");
    match Compute::open(&options) {
        Compute::Gpu(handle) => {
            println!("opened {} on {}", handle.info().name, handle.info().backend);
            for run in ["first", "second"] {
                let (sent, table) = (Arc::clone(&samples), Arc::clone(&table));
                let began = Instant::now();
                let (mapped, took) = handle
                    .run_blocking(move |gpu| lut::gpu(gpu, &sent, &table))
                    .expect("the LUT maps");
                let whole = began.elapsed();
                let pass = took.map_or("not timed".to_owned(), |took| format!("{took:.2?}"));
                println!(
                    "on the GPU, {run} time: {whole:.2?} with the upload and readback, the pass {pass}; {}",
                    if mapped == expected {
                        "as the CPU's"
                    } else {
                        "NOT as the CPU's"
                    }
                );
            }
        }
        Compute::Cpu { why } => println!("no GPU: {why}"),
    }
}
