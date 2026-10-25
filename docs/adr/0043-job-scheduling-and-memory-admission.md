# ADR-0043: Job scheduling and memory admission

- **Status:** Accepted
- **Date:** 2026-10-25
- **Phase:** P4.3
- **Related:** ADR-0005, ADR-0007, ADR-0041, REQ-3D-005, REQ-NFR-002; architecture §4.3, §5

## Context

- **Heavy processing runs while the instrument acquires:** pyramids, stitching, exports, and later
  3D reconstruction.
- **Acquisition must not suffer.** The frame path has its own threads and budget (architecture
  §4); jobs get what is left.
- **Memory is fixed:** jobs share 14 GB of RAM and 4 GB of VRAM by default (`[memory] job_ram_gb`,
  `job_vram_gb`, architecture §5). A job that runs past the budget pushes the process, or the
  frame pool's locked pages, into paging.
- **The GPU** (P7.1) will have one submitting thread; until then, one GPU job at a time is the
  rule.
- **Users and the UI** need to see what waits and what runs, and to cancel.
- **Provenance** (REQ-DATA-001, ADR-0041) asks for every step of processing to be recorded with
  its dataset.

## Decision

### Jobs

- **A job** (`siq_jobs::Job`) gives:
  - its kind, version and parameters, for its record;
  - an `estimate()` of what it needs at its peak (`Resources`: RAM, VRAM, whether it uses the GPU,
    the threads it can keep busy);
  - `run(&JobCtx)`, which returns its outputs or a `JobError` (`Cancelled` or `Failed`).
- **The context** (`JobCtx`) gives the job:
  - its dataset's folder;
  - the jobs' thread pool;
  - `progress(fraction)`, published every hundredth;
  - `log(message)`, which goes to the log and becomes the job's last message;
  - `check()`, which is `Err(Cancelled)` once it is cancelled;
  - `used(resources)`, which reports its real peak.

### Admission

- **The queue is ordered by priority** (`Interactive` > `Normal` > `Background`), then by
  submission.
- **Admission is decided whenever a job is submitted, finishes or is cancelled:** each queued job,
  in order, starts if it fits.
  - It fits if the running jobs' estimates plus its own stay within both budgets.
  - A GPU job fits only if no GPU job runs.
- **A job that doesn't fit is passed over,** and the ones behind it may start (backfill).
- **The starvation guard:** once a passed-over job has waited 60 s, nothing behind it starts
  until it has started.
  - A higher-priority job submitted later is still ahead of it.
- **A job bigger than a budget** starts alone, once nothing else runs, rather than never.
- **Estimates are trusted, and checked:**
  - a job that reports using more than it estimated goes on;
  - it is logged, marked `exceeded`, and counted in `jobs.estimates_exceeded`.

  The scheduler can't measure a job's memory alone, so it relies on what jobs report.

### Running

- **Each admitted job runs on a thread of its own,** named after it.
- **That thread runs at below-normal priority** (`THREAD_PRIORITY_BELOW_NORMAL`), and so does the
  jobs' rayon pool, which gets half the processors by default. Their start handler sets it.
  Acquisition, the previews and the service go first.
- **A panicking job fails;** it doesn't take the scheduler with it.
- **Cancelling:**
  - a queued job is cancelled at once;
  - a running one is asked to stop, and ends `Cancelled` when its next `check()` returns early.
- **Shutting down** cancels the queue, asks the running jobs to stop, and waits for them. The
  service does that after stopping live view and motion.

### Records and observation

- **A job on a dataset is recorded in its manifest's history** (ADR-0041's `ProcessingRecord`):
  - when it starts, with state `running`;
  - when it ends, with its state, progress, times, outputs and error.

  The record's id is unique across restarts. One manifest update runs at a time. A job ends only
  once its record is written.
- **The `jobs` Thing** shows the scheduler:
  - `queue` and `running`, observable, each a list of `JobInfo`: id, kind, priority, state,
    progress, estimate, exceeded, dataset, times, last message, error;
  - `cancel(id)`, which doesn't take the global lock.
- **The scheduler keeps the last 100 finished jobs,** to look up by id.

## Alternatives considered

- **Strict first come first served:** simple and fair, but a big job at the head idles the
  budget while small ones wait behind it.
- **Backfill without a guard:** a big job could wait forever behind a stream of small ones.
- **Aging (raising a waiting job's priority):** one more rule to explain. The guard gives the same
  promise: a job that has waited long enough starts next.
- **Measuring each job's memory:** a process's memory can't be split between its threads, and
  allocators don't know which job asked.
- **Running jobs on tokio's blocking pool:** that pool runs at normal priority and has no budget.
- **A separate process for jobs:** isolation, but frames, datasets and the GPU would have to be
  shared across processes.

## Consequences

- **Estimates matter:**
  - a job that underestimates can push the process past its budget, and is counted when it says
    so;
  - a job that overestimates waits longer than it needs to.
- **Jobs on the same dataset** update its manifest one at a time.
- **Queued jobs don't survive a restart.** Their datasets' records show only what ran.
- **P7.1 replaces the GPU rule** with the GPU thread's own queue.

## Verification

- `crates/siq-jobs/tests/scheduler.rs`:
  - admission within the RAM and VRAM budgets, with backfill, and one GPU job at a time;
  - priority first, then submission order;
  - the starvation guard stopping a later job overtaking;
  - a job bigger than the budget running alone;
  - cancelling a queued and a running job;
  - the record, done and failed, in the dataset's manifest;
  - an estimate exceeded: the job goes on, and is counted;
  - the job's thread and the pool's at below normal priority;
  - shutting down.
- `crates/siq-server/tests/jobs.rs`: the `jobs` Thing's `running` and `queue`, observed, and
  `cancel` over HTTP.
- **`jobs_demo`:** six synthetic jobs, showing each rule.
