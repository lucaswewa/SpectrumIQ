# siq-jobs

The job system (ADR-0043): heavy processing, such as pyramids, stitching and exports, run in the
background without disturbing acquisition. Jobs are admitted within the RAM and VRAM budgets, by
priority, on threads of below-normal priority, and recorded in their dataset's manifest.

## Place in the architecture

- **Layer:** L4 Engines.
- **Depends on:** `siq-storage` (the records), `siq-telemetry`, `siq-types`; `rayon`, `tokio`
  (`watch`), `windows-sys`.
- **Used by:** `siq-server` (the `jobs` Thing); the jobs of P4.4 and later.
- **Phase:** P4.3.

## API

| Type | What |
|---|---|
| `Job` | `kind`, `version`, `params`, `estimate() -> Resources`, `run(&JobCtx) -> Result<JobOutput, JobError>` |
| `JobCtx` | `dataset`, `pool` (below normal priority), `check` (`Err(Cancelled)` once cancelled), `progress`, `log`, `used` |
| `Scheduler` | `new(SchedulerConfig, registry)`; `submit(job, Priority, dataset)` → `JobId`; `cancel`, `info`, `wait`, `snapshot`, `watch` (a `JobsSnapshot` of `queued` and `running` `JobInfo`s), `in_use`, `estimates_exceeded`, `shutdown` |
| `SchedulerConfig` | The RAM and VRAM budgets, the pool's threads (half the processors), the starvation guard (60 s), the finished jobs kept (100) |
| `Resources` | `ram`, `vram`, `gpu`, `threads` |
| `this_thread_priority` | The calling thread's priority: −1 is below normal |

**Unsafe code** (ADR-0007): `priority` calls `SetThreadPriority` and `GetThreadPriority` on the
current thread's pseudo-handle, with its `SAFETY` comments.

## Examples

| Example | What it shows | Run |
|---|---|---|
| `jobs_demo` | Six synthetic jobs of different sizes and priorities: backfill, the GPU rule, priority, the starvation guard, a job bigger than the budget | `cargo run -p siq-jobs --example jobs_demo` |

Every example also runs in smoke mode (`SIQ_SMOKE=1`) in `cargo xtask ci`.

## Features

None.

## Testing

`cargo test -p siq-jobs`: admission and backfill, the GPU rule, priority and submission order, the
starvation guard, a job bigger than the budget, cancelling queued and running jobs, the dataset's
record, an exceeded estimate, below-normal threads, and shutting down.

## Limitations

- The scheduler trusts the jobs' estimates, and their reports of what they used.
- Queued jobs don't survive a restart.
- One GPU job at a time, until the GPU thread (P7.1).
