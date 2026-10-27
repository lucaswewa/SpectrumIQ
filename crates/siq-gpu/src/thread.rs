//! The GPU's thread (ADR-0058): it owns the device and queue, and runs the jobs sent to it one at
//! a time, each in its error scopes.

use std::collections::HashMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use futures::channel::oneshot;
use futures::executor::block_on;
use wgpu::util::DeviceExt;

use crate::{AdapterReport, GpuError, GpuOptions, instance, ranked, report};

type Job = Box<dyn FnOnce(&mut Gpu) + Send>;

/// The GPU, from any thread: it sends jobs to the thread that owns the device. Its clones share
/// the thread, which ends with the last.
#[derive(Clone)]
pub struct GpuHandle {
    jobs: mpsc::Sender<Job>,
    info: Arc<AdapterReport>,
    lost: Arc<Mutex<Option<String>>>,
}

/// A compute shader: its WGSL, included with `include_str!`, and the entry point run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Shader {
    /// Its name: one pipeline is made for each.
    pub name: &'static str,
    /// Its WGSL.
    pub source: &'static str,
    /// The entry point.
    pub entry: &'static str,
}

/// What a job has on the GPU's thread: the device and queue, and what it keeps for jobs.
pub struct Gpu {
    /// The device.
    pub device: wgpu::Device,
    /// Its queue.
    pub queue: wgpu::Queue,
    pipelines: HashMap<&'static str, wgpu::ComputePipeline>,
    /// Readback buffers kept for reuse, by size: the power of two at or above what was asked.
    readback: HashMap<u64, Vec<wgpu::Buffer>>,
    timestamps: Option<(wgpu::QuerySet, wgpu::Buffer)>,
}

/// The most readback buffers kept of each size.
const KEPT: usize = 4;

impl GpuHandle {
    /// The best adapter of `options` opened, on a thread of its own: with its own limits, and
    /// timestamp queries if it has them.
    pub fn open(options: &GpuOptions) -> Result<Self, GpuError> {
        let instance = instance();
        let adapter = ranked(&instance, options).into_iter().next();
        let adapter = adapter.ok_or_else(|| GpuError::NoAdapter("no adapter was found".into()))?;
        let info = report(&adapter);
        let features = adapter.features() & wgpu::Features::TIMESTAMP_QUERY;
        let descriptor = wgpu::DeviceDescriptor {
            label: Some("siq-gpu"),
            required_features: features,
            required_limits: adapter.limits(),
            ..wgpu::DeviceDescriptor::default()
        };
        let (device, queue) = block_on(adapter.request_device(&descriptor))
            .map_err(|error| GpuError::Device(error.to_string()))?;
        // Errors are caught by each job's scopes; any other is logged, not a panic.
        device.on_uncaptured_error(Arc::new(|error: wgpu::Error| {
            tracing::error!(%error, "a GPU error outside a job");
        }));
        let lost = Arc::new(Mutex::new(None));
        let noted = Arc::clone(&lost);
        device.set_device_lost_callback(move |reason, message| {
            tracing::error!(?reason, %message, "the GPU device was lost");
            *noted.lock().unwrap_or_else(PoisonError::into_inner) =
                Some(format!("{reason:?}: {message}"));
        });
        let timestamps = features.contains(wgpu::Features::TIMESTAMP_QUERY).then(|| {
            let set = device.create_query_set(&wgpu::QuerySetDescriptor {
                label: Some("siq-gpu timestamps"),
                ty: wgpu::QueryType::Timestamp,
                count: 2,
            });
            let resolve = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("siq-gpu timestamps resolved"),
                size: 16,
                usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            });
            (set, resolve)
        });
        let (jobs, received) = mpsc::channel::<Job>();
        let mut gpu = Gpu {
            device,
            queue,
            pipelines: HashMap::new(),
            readback: HashMap::new(),
            timestamps,
        };
        std::thread::Builder::new()
            .name("siq-gpu".into())
            .spawn(move || received.into_iter().for_each(|job| job(&mut gpu)))
            .map_err(|error| GpuError::Device(error.to_string()))?;
        tracing::info!(adapter = %info.name, backend = %info.backend, "the GPU is open");
        Ok(Self {
            jobs,
            info: Arc::new(info),
            lost,
        })
    }

    /// The adapter.
    pub fn info(&self) -> &AdapterReport {
        &self.info
    }

    /// Why the device was lost, if it was.
    pub fn lost(&self) -> Option<String> {
        self.lost
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Runs `work` on the GPU's thread, after the jobs before it: its result, or why it failed.
    pub async fn run<R: Send + 'static>(
        &self,
        work: impl FnOnce(&mut Gpu) -> Result<R, GpuError> + Send + 'static,
    ) -> Result<R, GpuError> {
        let (send, receive) = oneshot::channel();
        let lost = Arc::clone(&self.lost);
        let job: Job = Box::new(move |gpu| {
            let why = lost.lock().unwrap_or_else(PoisonError::into_inner).clone();
            let result = match why {
                Some(why) => Err(GpuError::Lost(why)),
                None => gpu.scoped(work),
            };
            let _ = send.send(result);
        });
        self.jobs.send(job).map_err(|_| GpuError::Closed)?;
        receive.await.map_err(|_| GpuError::Closed)?
    }

    /// [`run`](Self::run), waited for on this thread: for callers outside async code.
    pub fn run_blocking<R: Send + 'static>(
        &self,
        work: impl FnOnce(&mut Gpu) -> Result<R, GpuError> + Send + 'static,
    ) -> Result<R, GpuError> {
        block_on(self.run(work))
    }
}

impl Gpu {
    /// `work` run in scopes that catch what the GPU rejects and its running out of memory, and
    /// its panic caught: the thread goes on.
    fn scoped<R>(
        &mut self,
        work: impl FnOnce(&mut Gpu) -> Result<R, GpuError>,
    ) -> Result<R, GpuError> {
        let memory = self.device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let validation = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let result = catch_unwind(AssertUnwindSafe(|| work(self))).unwrap_or_else(|panic| {
            let message = panic
                .downcast_ref::<&str>()
                .map(|s| (*s).to_owned())
                .or_else(|| panic.downcast_ref::<String>().cloned())
                .unwrap_or_default();
            Err(GpuError::Panicked(message))
        });
        let rejected = [block_on(validation.pop()), block_on(memory.pop())];
        match rejected.into_iter().flatten().next() {
            Some(error) => Err(GpuError::Rejected(error.to_string())),
            None => result,
        }
    }

    /// `shader`'s compute pipeline, made the first time it is asked for.
    pub fn pipeline(&mut self, shader: &Shader) -> wgpu::ComputePipeline {
        let device = &self.device;
        let made = self.pipelines.entry(shader.name).or_insert_with(|| {
            let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(shader.name),
                source: wgpu::ShaderSource::Wgsl(shader.source.into()),
            });
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(shader.name),
                layout: None,
                module: &module,
                entry_point: Some(shader.entry),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                cache: None,
            })
        });
        made.clone()
    }

    /// `samples` in a storage buffer, two to a 32-bit word, the first in the low half: as their
    /// little-endian bytes lie. An odd last sample has 0 beside it.
    pub fn upload_u16(&self, samples: &[u16]) -> Result<wgpu::Buffer, GpuError> {
        let mut bytes: Vec<u8> = bytemuck::cast_slice(samples).to_vec();
        bytes.resize(bytes.len().div_ceil(4).max(1) * 4, 0);
        self.fits_binding(bytes.len() as u64, "the samples")?;
        Ok(self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("siq-gpu u16 samples"),
                contents: &bytes,
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_DST
                    | wgpu::BufferUsages::COPY_SRC,
            }))
    }

    /// `samples`, `width` × `height` row by row, as an `R16Uint` texture.
    pub fn upload_u16_texture(
        &self,
        samples: &[u16],
        width: u32,
        height: u32,
    ) -> Result<wgpu::Texture, GpuError> {
        let largest = self.device.limits().max_texture_dimension_2d;
        if width.max(height) > largest {
            let why = format!("a {width} × {height} texture is more than the adapter's {largest}");
            return Err(GpuError::TooLarge(why));
        }
        let size = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("siq-gpu u16 texture"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R16Uint,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let layout = wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(width * 2),
            rows_per_image: Some(height),
        };
        let bytes = bytemuck::cast_slice(samples);
        self.queue
            .write_texture(texture.as_image_copy(), bytes, layout, size);
        Ok(texture)
    }

    /// An error if `bytes` is more than one storage binding can see.
    pub fn fits_binding(&self, bytes: u64, what: &str) -> Result<(), GpuError> {
        let most = self.device.limits().max_storage_buffer_binding_size;
        match bytes <= most {
            true => Ok(()),
            false => Err(GpuError::TooLarge(format!(
                "{what}, {bytes} bytes, are more than the adapter's storage binding of {most}"
            ))),
        }
    }

    /// The first `bytes` of `buffer`, copied through a readback buffer and waited for.
    pub fn read(&mut self, buffer: &wgpu::Buffer, bytes: u64) -> Result<Vec<u8>, GpuError> {
        let copied = bytes.div_ceil(4) * 4;
        let class = copied.next_power_of_two().max(256);
        let kept = self.readback.entry(class).or_default().pop();
        let staging = kept.unwrap_or_else(|| {
            self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("siq-gpu readback"),
                size: class,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        encoder.copy_buffer_to_buffer(buffer, 0, &staging, 0, copied);
        self.queue.submit([encoder.finish()]);
        let (send, receive) = mpsc::channel();
        staging.map_async(wgpu::MapMode::Read, 0..copied, move |mapped| {
            let _ = send.send(mapped);
        });
        self.wait()?;
        let mapped = receive
            .recv()
            .map_err(|_| GpuError::Failed("no map".into()))?;
        mapped.map_err(|error| GpuError::Failed(error.to_string()))?;
        let view = staging.get_mapped_range(0..copied);
        let data =
            view.map_err(|error| GpuError::Failed(error.to_string()))?[..bytes as usize].to_vec();
        staging.unmap();
        let pool = self.readback.entry(class).or_default();
        if pool.len() < KEPT {
            pool.push(staging);
        }
        Ok(data)
    }

    /// Waits for the work submitted.
    pub fn wait(&self) -> Result<(), GpuError> {
        let waited = self.device.poll(wgpu::PollType::wait_indefinitely());
        waited
            .map(drop)
            .map_err(|error| GpuError::Failed(error.to_string()))
    }

    /// A compute pass recorded by `record`, submitted and waited for: how long the GPU took on it,
    /// if the adapter can time passes.
    pub fn timed_pass(
        &mut self,
        label: &str,
        record: impl FnOnce(&mut wgpu::ComputePass<'_>),
    ) -> Result<Option<Duration>, GpuError> {
        let mut encoder = self.device.create_command_encoder(&Default::default());
        {
            let timestamp_writes =
                self.timestamps
                    .as_ref()
                    .map(|(set, _)| wgpu::ComputePassTimestampWrites {
                        query_set: set,
                        beginning_of_pass_write_index: Some(0),
                        end_of_pass_write_index: Some(1),
                    });
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some(label),
                timestamp_writes,
            });
            record(&mut pass);
        }
        if let Some((set, resolve)) = &self.timestamps {
            encoder.resolve_query_set(set, 0..2, resolve, 0);
        }
        self.queue.submit([encoder.finish()]);
        self.wait()?;
        let Some((_, resolve)) = self.timestamps.clone() else {
            return Ok(None);
        };
        let bytes = self.read(&resolve, 16)?;
        let tick = |at: usize| u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap_or_default());
        let period = f64::from(self.queue.get_timestamp_period());
        let nanoseconds = tick(8).saturating_sub(tick(0)) as f64 * period;
        Ok(Some(Duration::from_nanos(nanoseconds as u64)))
    }

    /// A dispatch of `invocations`, `per_group` to a workgroup: across, then down when one row
    /// of workgroups would be more than the adapter allows. A shader finds its invocation's index
    /// as `id.x + id.y * num_workgroups.x * per_group`.
    pub fn groups(&self, invocations: u64, per_group: u32) -> Result<[u32; 2], GpuError> {
        let groups = invocations.div_ceil(u64::from(per_group)).max(1);
        let across = u64::from(self.device.limits().max_compute_workgroups_per_dimension);
        let down = groups.div_ceil(across);
        match down <= across {
            true => Ok([groups.min(across) as u32, down as u32]),
            false => Err(GpuError::TooLarge(format!(
                "{invocations} invocations are too many"
            ))),
        }
    }
}
