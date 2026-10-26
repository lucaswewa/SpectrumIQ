# siq-jobs

The job system (ADR-0043): heavy processing, such as pyramids, stitching and exports, run in the
background without disturbing acquisition. Jobs are admitted within the RAM and VRAM budgets, by
priority, on threads of below-normal priority, and recorded in their dataset's manifest.

## Place in the architecture

- **Layer:** L4 Engines.
- **Depends on:** `siq-storage` (the records), `siq-pyramid` and `siq-calib` (their jobs),
  `siq-telemetry`, `siq-types`; `rayon`, `tokio` (`watch`), `windows-sys`.
- **Used by:** `siq-server` (the `jobs` Thing); the jobs of P4.4 and later.
- **Phase:** P4.3; P4.4 added `kinds::PyramidJob`, P4.5 `kinds::DziExportJob`, P4.6
  `kinds::OmeTiffExportJob`, P4.7 `kinds::ZarrFromRawJob`, and P5.3 `kinds::ShadingJob`.

## API

| Type | What |
|---|---|
| `Job` | `kind`, `version`, `params`, `estimate() -> Resources`, `run(&JobCtx) -> Result<JobOutput, JobError>` |
| `JobCtx` | `dataset`, `pool` (below normal priority), `check` (`Err(Cancelled)` once cancelled), `progress`, `log`, `used` |
| `Scheduler` | `new(SchedulerConfig, registry)`; `submit(job, Priority, dataset)` → `JobId`; `cancel`, `info`, `wait`, `snapshot`, `watch` (a `JobsSnapshot` of `queued` and `running` `JobInfo`s), `in_use`, `estimates_exceeded`, `shutdown` |
| `SchedulerConfig` | The RAM and VRAM budgets, the pool's threads (half the processors), the starvation guard (60 s), the finished jobs kept (100) |
| `Resources` | `ram`, `vram`, `gpu`, `threads` |
| `this_thread_priority` | The calling thread's priority: −1 is below normal |
| `kinds::ZarrFromRawJob` | A dataset's raw frames as level 0 of a new OME-Zarr image, a channel per frame: the snap pipeline's first step (ADR-0041, amended) |
| `kinds::PyramidJob` | An OME-Zarr image's pyramid (ADR-0044), as a job: its estimate from the image, its progress by chunks, and the image as its output. The jobs the service runs live in `kinds`, since processing crates (L3) can't use the job system (L4) |
| `kinds::DziExportJob` | A plane of an OME-Zarr image as a static DZI pyramid on disk (ADR-0045): the descriptor and every level's tiles, rendered as the tile service does, in parallel |
| `kinds::OmeTiffExportJob` | An OME-Zarr image as one OME-TIFF file (ADR-0046), with its dataset's manifest in the OME-XML: its estimate from the image, its progress by tiles, and the file as its output |
| `kinds::ShadingJob { registry, key, dark, sensor }` | The flat of a dataset's frames of the whole sensor and of the key's channel, estimated a frame at a time (ADR-0050, BaSiC simplified), and kept in the registry for the key: its progress by frames, and the record's file as its output |

**Unsafe code** (ADR-0007): `priority` calls `SetThreadPriority` and `GetThreadPriority` on the
current thread's pseudo-handle, with its `SAFETY` comments.

## Examples

| Example | What it shows | Run |
|---|---|---|
| `jobs_demo` | Six synthetic jobs of different sizes and priorities: backfill, the GPU rule, priority, the starvation guard, a job bigger than the budget | `cargo run -p siq-jobs --example jobs_demo` |
| `export_dzi` | A synthetic dataset's pyramid, then its DZI export, through the scheduler, and their records | `cargo run --release -p siq-jobs --example export_dzi` |
| `export_ometiff` | A synthetic two-channel dataset's pyramid, then its OME-TIFF export, through the scheduler; `--out <folder>` keeps it, to open in Fiji or QuPath | `cargo run --release -p siq-jobs --example export_ometiff` |
| `estimate_flat` | A `siq-datagen` tile scan of 8 × 6 tiles of 2048 × 1536, with the simulator's vignetting, through the shading job: the flat found against the vignetting, and the seams before and after | `cargo run --release -p siq-jobs --example estimate_flat [-- --out <folder>]` |

Every example also runs in smoke mode (`SIQ_SMOKE=1`) in `cargo xtask ci`.

## Features

None.

## Testing

`cargo test -p siq-jobs`: admission and backfill, the GPU rule, priority and submission order, the
starvation guard, a job bigger than the budget, cancelling queued and running jobs, the dataset's
record, an exceeded estimate, below-normal threads, and shutting down. The jobs of `kinds`: a
dataset's pyramid, then its DZI and OME-TIFF exports, each recorded. `siq-server`'s `shading`
test runs the shading job on a tile scan.

## Limitations

- The scheduler trusts the jobs' estimates, and their reports of what they used.
- Queued jobs don't survive a restart.
- One GPU job at a time, until the GPU thread (P7.1).
