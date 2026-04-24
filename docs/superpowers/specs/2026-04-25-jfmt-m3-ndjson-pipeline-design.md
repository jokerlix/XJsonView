# jfmt M3 — NDJSON Parallel Pipeline Design

> Refinement of the Phase 1 spec (`2026-04-23-jfmt-phase1-design.md`) §4.1
> `ndjson/*` and §5.1 "NDJSON parallel". This document locks in the public
> API, threading model, and integration with existing subcommands so the
> implementation plan has unambiguous contracts to follow.

**Milestone:** M3 — ships as `v0.0.3`
**Estimate:** 1 week per roadmap
**Depends on:** M1 (transcode, writer traits), M2 (Stats, StatsCollector, Error::Syntax)

---

## 1. Goal

Add an ordered, multi-threaded NDJSON processing pipeline to `jfmt-core`
and route `jfmt pretty`/`minify`/`validate` through it when `--ndjson`
is set. Single code path for both serial (`--threads 1`) and parallel
modes; the serial `validate_ndjson` from M2 is deleted.

## 2. Non-goals

- `jfmt filter` — deferred to M4.
- Schema validation in the stats pipeline — deferred to M5 (adds a
  `schema_violations` field to `Stats` then, not now).
- Progress bar (`indicatif`) — M6.
- Platform-specific async I/O (io_uring, IOCP) — use `std::io::BufReader`
  directly; profile and reconsider in M6.

## 3. Public API (jfmt-core)

```rust
// crates/jfmt-core/src/ndjson/mod.rs

pub struct NdjsonPipelineOptions {
    /// Worker thread count. 0 = auto-detect via num_cpus physical cores.
    pub threads: usize,
    /// Channel depth between splitter→workers and workers→reorder.
    /// Default: threads * 4. Must be ≥ 1.
    pub channel_capacity: usize,
    /// If true, the first per-line error stops the pipeline and is
    /// returned. If false, errors accumulate in PipelineReport.errors
    /// and successful lines continue to stream to output in order.
    pub fail_fast: bool,
    /// If true, merge per-worker StatsCollectors into PipelineReport.stats.
    pub collect_stats: bool,
}

impl Default for NdjsonPipelineOptions {
    fn default() -> Self {
        Self {
            threads: 0,
            channel_capacity: 0, // replaced at run-time by threads*4
            fail_fast: false,
            collect_stats: false,
        }
    }
}

/// Aggregated outcome of a pipeline run.
pub struct PipelineReport {
    pub records: u64,
    pub errors: Vec<(u64 /* 1-indexed line */, LineError)>,
    pub stats: Option<Stats>,
}

/// Drive the NDJSON parallel pipeline.
///
/// `f` is invoked once per non-blank input line. It receives the raw
/// line bytes (without trailing newline) and a mutable handle to this
/// worker's `StatsCollector`. It returns either the bytes to write to
/// output (in input order) or a `LineError` for the reorder stage to
/// record.
///
/// `f` MUST call `collector.begin_record()` at the start of each
/// invocation and `collector.end_record(valid)` before returning.
/// Splitting this responsibility between closure and pipeline would
/// leak `StatsCollector` internals into the pipeline.
pub fn run_ndjson_pipeline<F>(
    input: impl Read + Send + 'static,
    output: impl Write + Send + 'static,
    f: F,
    opts: NdjsonPipelineOptions,
) -> Result<PipelineReport>
where
    F: Fn(&[u8], &mut StatsCollector) -> Result<Vec<u8>, LineError>
        + Send
        + Sync
        + 'static;
```

**Why closure receives `&mut StatsCollector`:** stats collection must
see every event. If the pipeline parsed lines internally (for stats)
*and* the closure re-parsed (for output), work would double. Passing
a per-worker collector into the closure is the only way to observe
parse events without a global lock.

**Why `Send + 'static` bounds on `input`/`output`:** splitter and
reorder stages each own their side of the I/O on a dedicated thread.

## 4. Internal modules

```
crates/jfmt-core/src/ndjson/
  mod.rs         NdjsonPipelineOptions + PipelineReport + run_ndjson_pipeline
  splitter.rs    single thread: reads BufReader, splits on \n, pushes
                 (seq, Vec<u8>) into bounded channel
  worker.rs      N threads: pull (seq, Vec<u8>), call f with owned
                 StatsCollector, push (seq, Result<Vec<u8>, LineError>)
                 into out-channel
  reorder.rs     single thread: drains out-channel into min-heap by
                 seq; writes contiguous Ok chunks to output; records
                 Err into PipelineReport.errors; honors fail_fast
  merge.rs       Stats::merge(other: Stats): commutative merge of two
                 collectors' accumulated Stats
```

### 4.1 Lifecycle

1. Main thread parses `opts`, resolves `threads` (0 → `num_cpus::get_physical()`),
   resolves `channel_capacity` (0 → `max(1, threads * 4)`).
2. Create two bounded channels (`crossbeam-channel`): `in_rx`
   (splitter → workers), `out_rx` (workers → reorder). Shared cancel
   flag `Arc<AtomicBool>`.
3. Spawn splitter thread, N worker threads, reorder thread.
4. Wait on all join handles; collect errors if any thread panicked.
5. Sum per-worker `StatsCollector`s via `Stats::merge` if
   `collect_stats`. Return `PipelineReport`.

### 4.2 Cancellation (fail_fast)

- Reorder thread sets `cancel` to true on first `Err` if
  `opts.fail_fast`.
- Splitter checks `cancel` between line reads and returns, dropping
  its `in_tx`.
- Workers see channel closed or `cancel==true`, return.
- Reorder drains remaining outputs and exits.

### 4.3 Ordering guarantees

- Output stream receives lines in **input order**.
- `PipelineReport.errors` is sorted by line before return (reorder
  appends in completion order; we sort on exit).
- Stats is accumulation-order-independent.

## 5. Integration with existing commands

| Subcommand | closure signature                                               | Default fail_fast |
|------------|-----------------------------------------------------------------|-------------------|
| pretty     | `fn(line, &mut _) -> Ok(pretty_bytes)`                          | `true`            |
| minify     | `fn(line, &mut _) -> Ok(minified_bytes)`                        | `true`            |
| validate   | `fn(line, &mut stats) -> Ok(vec![]) or Err(LineError)`          | `args.fail_fast`  |

- pretty/minify ignore `StatsCollector` (pass through, no `begin_record`).
- validate closes over nothing; each worker's collector is fed events
  via `begin_record` → observe events as it parses → `end_record`.
- pretty/minify default `fail_fast=true`: a mid-file parse error
  produces invalid output from that point on, so continuing is worse
  than aborting.
- validate keeps M2's semantic: `fail_fast` from the user flag,
  default false.

## 6. CLI additions

New global flag at the `Cli` top level:

```
--threads N    # 0 = auto (physical cores), 1 = serial, >=2 parallel
```

Available on all subcommands. When `--ndjson` is not set, the flag is
silently accepted but ignored (single-doc mode has no parallelism to
exploit).

## 7. Deletion list

- `crates/jfmt-core/src/validate/ndjson.rs` — deleted.
- Its 6 unit tests migrate to
  `crates/jfmt-core/tests/ndjson_pipeline.rs` (integration test
  location), rewritten against `run_ndjson_pipeline` with
  `threads: 1`.
- `crates/jfmt-core/src/validate/mod.rs` drops the `ndjson` module
  and `LineError` / `NdjsonOptions` / `NdjsonReport` re-exports.
  `LineError` relocates to `crates/jfmt-core/src/ndjson/mod.rs`.
- CLI validate command rewritten to route through
  `run_ndjson_pipeline`; existing CLI e2e tests should pass unchanged
  (they assert observable behavior, not module structure).

## 8. New dependencies

Both pinned to keep MSRV 1.75 compatibility — exact versions verified
during plan-writing:

- `crossbeam-channel` — bounded MPMC channels + `select!`
- `num_cpus` — physical core detection

If either dep has crept onto edition2024 and a MSRV-1.75 version isn't
available, fall back to `std::sync::mpsc` (bounded via manual
semaphore) and hardcoded default `threads=4`. Document the fallback
in the plan.

## 9. Testing strategy

### 9.1 Unit tests (in each module)

- `splitter`: splits on `\n`, skips blank lines, assigns monotonic
  seq, respects cancel.
- `worker`: single-thread in-process invocation of one closure call.
- `reorder`: given scrambled input, produces sorted output; records
  errors; honors `fail_fast`.
- `merge`: `Stats::merge` on two collectors matches a single
  collector fed the same events in sequence.

### 9.2 Integration tests (`crates/jfmt-core/tests/ndjson_pipeline.rs`)

Port all 6 M2 ndjson tests against `run_ndjson_pipeline`. Add:

- **Parallel == serial output**: 10k synthetic lines, run with
  `threads=1` and `threads=8`, output streams are byte-identical.
- **Parallel stats == serial stats**: same input, `collect_stats=true`,
  `PipelineReport.stats` equal.
- **Fail-fast stops early**: 1M lines, bad line at position 100, with
  `fail_fast=true`, measure wall-clock is < 1s (serial would need
  longer).
- **Channel backpressure**: synthetic slow closure (`thread::sleep(10ms)`)
  with `channel_capacity=2` — no OOM, input thread blocks correctly.

### 9.3 CLI e2e

- `jfmt pretty --ndjson --threads 4 fixture.ndjson` output matches
  `jfmt pretty --ndjson --threads 1 fixture.ndjson`.
- `jfmt minify --ndjson file.ndjson | jfmt pretty --ndjson` round-trips
  correctly (pipe between two pipelines).

### 9.4 Property test

- For arbitrary Vec of serde_json Values joined by `\n`, pipeline
  output at `threads=N` equals pipeline output at `threads=1` for
  any `N ∈ [1, 8]`.

## 10. Risks

1. **Closure + StatsCollector API is a new pattern.** If implementation
   reveals it's awkward (e.g., worker can't hold mutable state across
   calls when closure is `Fn` not `FnMut`), fall back to per-worker
   `Cell<StatsCollector>` or `thread_local!`. Plan documents the
   fallback.
2. **Min-heap in reorder unbounded.** If N workers are much faster
   than the output Write, the heap can grow. Cap by the out-channel
   capacity (workers block pushing if out-channel is full). Formalize
   the cap in `reorder.rs` and assert heap size ≤ channel_capacity +
   threads in debug builds.
3. **MSRV pins.** Same class of issue as M1/M2 — `crossbeam-channel`
   and `num_cpus` might need exact-version pins. Plan's Task 0
   verifies compatibility before any other work.
4. **Panic safety.** A panicking worker must not deadlock the
   pipeline. Workers catch `std::panic::catch_unwind` and propagate
   as a synthesized `LineError { message: "worker panic: …" }`.

## 11. Exit criteria

- `cargo test --workspace` passes (existing 84 + ~15 M3 tests).
- `cargo clippy --workspace --all-targets -- -D warnings` clean.
- `jfmt {pretty,minify,validate} --ndjson` produces byte-identical
  output to M2 at `--threads 1`, and faster wall-clock at
  `--threads >=2` on a large NDJSON fixture (smoke-bench in the plan,
  not a formal benchmark — that's M6).
- Tag `v0.0.3`, update Phase 1 spec milestone table.
