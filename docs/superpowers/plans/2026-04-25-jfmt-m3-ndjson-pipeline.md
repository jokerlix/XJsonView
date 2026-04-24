# jfmt M3 — NDJSON Parallel Pipeline Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace M2's serial NDJSON path with a multi-threaded splitter / worker / reorder pipeline in `jfmt-core`, routed under pretty / minify / validate.

**Architecture:** Single public entry point `run_ndjson_pipeline` drives a bounded crossbeam channel from a splitter thread to N worker threads to a single reorder thread, which writes output in input order. Per-worker `StatsCollector`s merge at the end via new `Stats::merge`.

**Tech Stack:** Rust 1.75, existing (struson, clap, thiserror, serde, anyhow), new: `crossbeam-channel`, `num_cpus`.

---

## Scope Boundaries

**In-scope:**
- `crates/jfmt-core/src/ndjson/` module: `splitter`, `worker`, `reorder`, `mod`.
- `Stats::merge` for combining per-worker stats.
- Global `--threads N` flag on the CLI.
- Wiring pretty/minify/validate `--ndjson` through the pipeline.
- Deleting M2's serial `validate/ndjson.rs`; migrating its tests.
- Tag `v0.0.3`, update Phase 1 spec milestone table.

**Out of scope:**
- `jfmt filter` → M4.
- Schema violation histograms → M5.
- Progress bar → M6.
- Platform-specific async I/O → M6 or later.

## File Structure

```
Cargo.toml                          # Modify: add crossbeam-channel, num_cpus pins
crates/jfmt-core/Cargo.toml         # Modify: deps
crates/jfmt-core/src/
  lib.rs                            # Modify: pub mod ndjson; re-exports; drop validate::ndjson
  ndjson/
    mod.rs                          # Create: run_ndjson_pipeline + types
    splitter.rs                     # Create: single-thread \n splitter with seq numbers
    worker.rs                       # Create: per-worker loop with panic catch
    reorder.rs                      # Create: min-heap reorder + fail_fast write
  validate/
    mod.rs                          # Modify: drop ndjson module + re-exports
    ndjson.rs                       # DELETE
    stats.rs                        # Modify: add Stats::merge
crates/jfmt-core/tests/
  ndjson_pipeline.rs                # Create: integration tests (migrated M2 tests + parallel)

crates/jfmt-cli/src/
  cli.rs                            # Modify: add global --threads, ValidateArgs keeps fail_fast
  main.rs                           # Modify: pass threads into command args
  commands/
    pretty.rs                       # Modify: --ndjson routes through pipeline
    minify.rs                       # Modify: --ndjson routes through pipeline
    validate.rs                     # Modify: NDJSON branch uses pipeline

crates/jfmt-io/src/
  output.rs                         # Modify: Box<dyn Write + Send> (pipeline needs Send)

crates/jfmt-cli/tests/
  cli_validate.rs                   # Modify: --threads variants
  cli_pretty.rs                     # Modify: add --ndjson --threads cases
  cli_minify.rs                     # Modify: add --ndjson --threads cases
  fixtures/ndjson-many.ndjson       # Create: larger fixture for parallel-vs-serial

README.md                           # Modify: --threads, parallel note
docs/superpowers/specs/
  2026-04-23-jfmt-phase1-design.md  # Modify: mark M3 shipped
```

---

## Task 1: Pin new dependencies and smoke-build

**Files:**
- Modify: `Cargo.toml`
- Modify: `crates/jfmt-core/Cargo.toml`

Like M1/M2, new crates can transitively pull edition2024 deps. Task 0
verifies the exact versions we need compile on Rust 1.75 before any
other work depends on them.

- [ ] **Step 1: Add workspace deps with `=` pins**

Edit `Cargo.toml` `[workspace.dependencies]`:

```toml
# Concurrency — pinned to stay MSRV 1.75 compatible.
crossbeam-channel = "=0.5.13"
num_cpus = "=1.16.0"
```

(0.5.13 / 1.16.0 are the last known MSRV-1.75 compatible releases at
the time of writing. If `cargo build` in Step 3 fails with
`edition2024`, downgrade further — crossbeam-channel 0.5.8 and
num_cpus 1.15.0 are safe fallbacks.)

- [ ] **Step 2: Wire into jfmt-core**

Edit `crates/jfmt-core/Cargo.toml`:

```toml
[dependencies]
struson = { workspace = true }
thiserror = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }
crossbeam-channel = { workspace = true }
num_cpus = { workspace = true }
```

- [ ] **Step 3: Verify compile**

Run: `cargo build --workspace`
Expected: compiles. If an edition2024 error surfaces, pin to the
fallback versions noted in Step 1 and commit the reason in the commit
message.

- [ ] **Step 4: Commit**

```bash
git add Cargo.toml Cargo.lock crates/jfmt-core/Cargo.toml
git commit -m "chore(deps): add crossbeam-channel + num_cpus for M3"
```

---

## Task 2: `Stats::merge`

**Files:**
- Modify: `crates/jfmt-core/src/validate/stats.rs`

- [ ] **Step 1: Write the failing test**

Append inside the existing `mod tests` block in `stats.rs`:

```rust
    #[test]
    fn merge_sums_counts_and_unions_keys() {
        let mut a = Stats {
            records: 2,
            valid: 2,
            invalid: 0,
            max_depth: 3,
            ..Default::default()
        };
        a.top_level_types.insert("object".into(), 2);
        a.top_level_keys.insert("x".into(), 2);
        a.top_level_keys.insert("only_in_a".into(), 1);
        a.top_level_keys_truncated = 1;

        let mut b = Stats {
            records: 1,
            valid: 0,
            invalid: 1,
            max_depth: 5,
            ..Default::default()
        };
        b.top_level_types.insert("object".into(), 1);
        b.top_level_types.insert("array".into(), 1);
        b.top_level_keys.insert("x".into(), 3);
        b.top_level_keys.insert("only_in_b".into(), 2);
        b.top_level_keys_truncated = 4;

        a.merge(b);

        assert_eq!(a.records, 3);
        assert_eq!(a.valid, 2);
        assert_eq!(a.invalid, 1);
        assert_eq!(a.max_depth, 5);
        assert_eq!(a.top_level_types.get("object"), Some(&3));
        assert_eq!(a.top_level_types.get("array"), Some(&1));
        assert_eq!(a.top_level_keys.get("x"), Some(&5));
        assert_eq!(a.top_level_keys.get("only_in_a"), Some(&1));
        assert_eq!(a.top_level_keys.get("only_in_b"), Some(&2));
        assert_eq!(a.top_level_keys_truncated, 5);
    }
```

- [ ] **Step 2: Implement `merge`**

Add to `impl Stats` (new `impl` block after the struct, before
`Display`):

```rust
impl Stats {
    /// Merge `other` into `self`. Commutative. Cap on
    /// `top_level_keys` is a per-pass guard, not a post-merge
    /// invariant — merged maps may exceed any individual collector's
    /// cap.
    pub fn merge(&mut self, other: Stats) {
        self.records += other.records;
        self.valid += other.valid;
        self.invalid += other.invalid;
        if other.max_depth > self.max_depth {
            self.max_depth = other.max_depth;
        }
        for (k, v) in other.top_level_types {
            *self.top_level_types.entry(k).or_insert(0) += v;
        }
        for (k, v) in other.top_level_keys {
            *self.top_level_keys.entry(k).or_insert(0) += v;
        }
        self.top_level_keys_truncated += other.top_level_keys_truncated;
    }
}
```

- [ ] **Step 3: Run**

Run: `cargo test -p jfmt-core stats`
Expected: 13 passed (12 existing + 1 new).

- [ ] **Step 4: Commit**

```bash
git add crates/jfmt-core/src/validate/stats.rs
git commit -m "feat(core): add Stats::merge for per-worker aggregation"
```

---

## Task 3: NDJSON module skeleton + `LineError` relocation

**Files:**
- Create: `crates/jfmt-core/src/ndjson/mod.rs`
- Modify: `crates/jfmt-core/src/lib.rs`

M2's `LineError` lives under `validate::ndjson`. Per the design doc,
it moves to the new `ndjson` module. We move it now (before deleting
`validate/ndjson.rs` in Task 8) so later tasks can depend on the new
location.

- [ ] **Step 1: Create the module root with types + a stub function**

```rust
// crates/jfmt-core/src/ndjson/mod.rs
//! NDJSON parallel pipeline: splitter → workers → reorder.
//!
//! Public entry point is [`run_ndjson_pipeline`]. Callers provide a
//! closure invoked once per non-blank input line; its output is
//! written to the `output` stream in input order.

use crate::validate::{Stats, StatsCollector};
use std::io::{Read, Write};

pub mod splitter;
pub mod worker;
pub mod reorder;

/// One reported per-line error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineError {
    /// 1-indexed line number (blank lines still advance the counter).
    pub line: u64,
    /// Byte offset inside that line where the error occurred.
    pub offset: u64,
    pub column: Option<u64>,
    pub message: String,
}

#[derive(Debug, Clone, Copy)]
pub struct NdjsonPipelineOptions {
    /// Worker thread count. 0 = auto-detect via num_cpus physical cores.
    pub threads: usize,
    /// Bounded channel depth (in→worker and worker→out). 0 = auto
    /// (max(1, threads * 4)).
    pub channel_capacity: usize,
    /// If true, the first per-line error stops the pipeline; otherwise
    /// errors accumulate and successful lines continue to stream out.
    pub fail_fast: bool,
    /// If true, merge per-worker StatsCollectors into the report.
    pub collect_stats: bool,
}

impl Default for NdjsonPipelineOptions {
    fn default() -> Self {
        Self {
            threads: 0,
            channel_capacity: 0,
            fail_fast: false,
            collect_stats: false,
        }
    }
}

/// Aggregated outcome of a pipeline run.
#[derive(Debug, Default)]
pub struct PipelineReport {
    pub records: u64,
    pub errors: Vec<(u64, LineError)>,
    pub stats: Option<Stats>,
}

/// Drive the NDJSON parallel pipeline.
///
/// `f` is invoked once per non-blank input line. It receives the raw
/// line bytes (without trailing newline) and a mutable handle to this
/// worker's `StatsCollector`. It returns either the bytes to write to
/// output (in input order) or a `LineError`.
///
/// `f` MUST call `collector.begin_record()` at the start of each
/// invocation and `collector.end_record(valid)` before returning.
pub fn run_ndjson_pipeline<R, W, F>(
    input: R,
    output: W,
    f: F,
    opts: NdjsonPipelineOptions,
) -> std::io::Result<PipelineReport>
where
    R: Read + Send + 'static,
    W: Write + Send + 'static,
    F: Fn(&[u8], &mut StatsCollector) -> Result<Vec<u8>, LineError>
        + Send
        + Sync
        + 'static,
{
    // Real implementation lands in Task 7.
    let _ = (input, output, f, opts);
    Ok(PipelineReport::default())
}
```

- [ ] **Step 2: Create empty sub-module files (so `pub mod` works)**

```rust
// crates/jfmt-core/src/ndjson/splitter.rs
//! Single-thread line splitter — implementation lands in Task 4.
```

```rust
// crates/jfmt-core/src/ndjson/worker.rs
//! Per-worker loop — implementation lands in Task 5.
```

```rust
// crates/jfmt-core/src/ndjson/reorder.rs
//! Reorder + output stage — implementation lands in Task 6.
```

- [ ] **Step 3: Wire into lib.rs**

Edit `crates/jfmt-core/src/lib.rs`:

```rust
pub mod error;
pub mod escape;
pub mod event;
pub mod ndjson;             // NEW
pub mod parser;
pub mod transcode;
pub mod validate;
pub mod writer;

pub use error::{Error, Result};
pub use event::{Event, Scalar};
pub use ndjson::{              // NEW
    run_ndjson_pipeline, LineError, NdjsonPipelineOptions, PipelineReport,
};
pub use parser::EventReader;
pub use transcode::transcode;
pub use validate::{validate_syntax, Stats, StatsCollector, StatsConfig};
pub use writer::{EventWriter, MinifyWriter, PrettyConfig, PrettyWriter};
```

Note: `validate_ndjson`, `NdjsonOptions`, `NdjsonReport` re-exports
from M2 are intentionally removed here. Task 8 deletes the
`validate::ndjson` module to match.

- [ ] **Step 4: Build**

Run: `cargo build -p jfmt-core`
Expected: compiles with a dead-code warning on the `_ = (input, ...)`
discard — that's fine until Task 7.

- [ ] **Step 5: Commit**

```bash
git add crates/jfmt-core/src/ndjson crates/jfmt-core/src/lib.rs
git commit -m "feat(core): scaffold ndjson module and public API"
```

---

## Task 4: Splitter

**Files:**
- Modify: `crates/jfmt-core/src/ndjson/splitter.rs`

The splitter reads the input, splits on `\n`, and pushes
`(seq, Vec<u8>)` pairs into the bounded worker channel. Blank lines
are skipped (do not increment the record count but DO advance the
line counter for error reporting).

- [ ] **Step 1: Write failing tests**

```rust
// crates/jfmt-core/src/ndjson/splitter.rs
//! Single-thread line splitter.

use crossbeam_channel::Sender;
use std::io::{BufRead, BufReader, Read};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// One unit of work handed to a worker: the 1-indexed input line
/// number + the raw line bytes (no trailing newline).
pub type LineItem = (u64, Vec<u8>);

/// Drain `reader` line-by-line into `tx`. Returns the number of
/// non-blank lines that were sent. Stops early (dropping `tx`) if
/// `cancel` becomes true.
pub fn split_lines<R: Read>(
    reader: R,
    tx: Sender<LineItem>,
    cancel: Arc<AtomicBool>,
) -> std::io::Result<u64> {
    let br = BufReader::new(reader);
    let mut sent: u64 = 0;
    for (idx, line) in br.lines().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let line_no = idx as u64 + 1;
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        if tx.send((line_no, line.into_bytes())).is_err() {
            // Receiver dropped (pipeline shutting down).
            break;
        }
        sent += 1;
    }
    Ok(sent)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossbeam_channel::unbounded;

    #[test]
    fn sends_one_item_per_nonblank_line() {
        let (tx, rx) = unbounded();
        let input = b"1\n\n2\n\n3\n".to_vec();
        let cancel = Arc::new(AtomicBool::new(false));
        let count = split_lines(input.as_slice(), tx, cancel).unwrap();
        assert_eq!(count, 3);
        let items: Vec<LineItem> = rx.iter().collect();
        assert_eq!(items.len(), 3);
        assert_eq!(items[0], (1, b"1".to_vec()));
        assert_eq!(items[1], (3, b"2".to_vec())); // blank at line 2 skipped
        assert_eq!(items[2], (5, b"3".to_vec())); // blank at line 4 skipped
    }

    #[test]
    fn stops_early_on_cancel() {
        let (tx, rx) = crossbeam_channel::bounded(1);
        let cancel = Arc::new(AtomicBool::new(true));
        // Cancel is already set; no lines should be sent even though
        // the input has lines.
        let input = b"1\n2\n3\n".to_vec();
        let count = split_lines(input.as_slice(), tx, cancel).unwrap();
        assert_eq!(count, 0);
        drop(rx);
    }

    #[test]
    fn stops_when_receiver_drops() {
        // Bounded(1) with no consumer: after the first send fills the
        // channel, the receiver drop causes the second send to fail
        // and the splitter to return.
        let (tx, rx) = crossbeam_channel::bounded(1);
        // Receive and drop the first item so the channel has room for
        // a send; then drop rx to force the next send to fail.
        let input = b"1\n2\n3\n".to_vec();
        let cancel = Arc::new(AtomicBool::new(false));
        let handle = std::thread::spawn(move || {
            split_lines(input.as_slice(), tx, cancel).unwrap()
        });
        let _ = rx.recv().unwrap();
        drop(rx);
        let count = handle.join().unwrap();
        // At least one was sent; splitter returned early when rx
        // closed. We don't assert an exact count because of timing.
        assert!(count >= 1);
    }
}
```

- [ ] **Step 2: Run tests**

Run: `cargo test -p jfmt-core splitter`
Expected: 3 passed.

- [ ] **Step 3: Commit**

```bash
git add crates/jfmt-core/src/ndjson/splitter.rs
git commit -m "feat(core): add ndjson splitter thread"
```

---

## Task 5: Worker

**Files:**
- Modify: `crates/jfmt-core/src/ndjson/worker.rs`

Each worker holds its own `StatsCollector`, pulls work items from
the in-channel, invokes the closure under `catch_unwind`, and pushes
`(seq, Result<Vec<u8>, LineError>)` into the out-channel.

- [ ] **Step 1: Write failing tests**

```rust
// crates/jfmt-core/src/ndjson/worker.rs
//! Per-worker loop. Owns one StatsCollector for the lifetime of the
//! thread; merged into the final report after join.

use crate::ndjson::splitter::LineItem;
use crate::ndjson::LineError;
use crate::validate::StatsCollector;
use crossbeam_channel::{Receiver, Sender};
use std::panic::AssertUnwindSafe;

/// One unit of completed work sent to the reorder stage.
pub type WorkerOutput = (u64, Result<Vec<u8>, LineError>);

/// Run one worker loop. Returns the collector so the caller can
/// merge it.
pub fn run_worker<F>(
    rx: Receiver<LineItem>,
    tx: Sender<WorkerOutput>,
    f: std::sync::Arc<F>,
) -> StatsCollector
where
    F: Fn(&[u8], &mut StatsCollector) -> Result<Vec<u8>, LineError> + Send + Sync + 'static,
{
    let mut collector = StatsCollector::default();
    while let Ok((seq, bytes)) = rx.recv() {
        let result = match std::panic::catch_unwind(AssertUnwindSafe(|| f(&bytes, &mut collector)))
        {
            Ok(r) => r,
            Err(_) => Err(LineError {
                line: seq,
                offset: 0,
                column: None,
                message: "worker panic while processing line".into(),
            }),
        };
        if tx.send((seq, result)).is_err() {
            break;
        }
    }
    collector
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossbeam_channel::unbounded;
    use std::sync::Arc;

    #[test]
    fn forwards_results_in_submission_order_for_single_worker() {
        let (in_tx, in_rx) = unbounded::<LineItem>();
        let (out_tx, out_rx) = unbounded::<WorkerOutput>();
        in_tx.send((1, b"hi".to_vec())).unwrap();
        in_tx.send((2, b"!!".to_vec())).unwrap();
        drop(in_tx);

        let f = Arc::new(|bytes: &[u8], _c: &mut StatsCollector| {
            Ok::<_, LineError>(bytes.to_ascii_uppercase())
        });
        run_worker(in_rx, out_tx, f);
        let collected: Vec<_> = out_rx.iter().collect();
        assert_eq!(collected, vec![
            (1, Ok(b"HI".to_vec())),
            (2, Ok(b"!!".to_vec())),
        ]);
    }

    #[test]
    fn catches_panics_as_line_errors() {
        let (in_tx, in_rx) = unbounded::<LineItem>();
        let (out_tx, out_rx) = unbounded::<WorkerOutput>();
        in_tx.send((42, b"trigger".to_vec())).unwrap();
        drop(in_tx);

        let f = Arc::new(|_b: &[u8], _c: &mut StatsCollector| -> Result<Vec<u8>, LineError> {
            panic!("boom")
        });
        run_worker(in_rx, out_tx, f);
        let items: Vec<_> = out_rx.iter().collect();
        assert_eq!(items.len(), 1);
        let (seq, res) = &items[0];
        assert_eq!(*seq, 42);
        let e = res.as_ref().unwrap_err();
        assert!(e.message.contains("panic"), "got {}", e.message);
        assert_eq!(e.line, 42);
    }

    #[test]
    fn forwards_closure_errors() {
        let (in_tx, in_rx) = unbounded::<LineItem>();
        let (out_tx, out_rx) = unbounded::<WorkerOutput>();
        in_tx.send((7, b"x".to_vec())).unwrap();
        drop(in_tx);

        let f = Arc::new(|_b: &[u8], _c: &mut StatsCollector| -> Result<Vec<u8>, LineError> {
            Err(LineError {
                line: 7,
                offset: 0,
                column: None,
                message: "nope".into(),
            })
        });
        run_worker(in_rx, out_tx, f);
        let items: Vec<_> = out_rx.iter().collect();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].0, 7);
        assert!(items[0].1.is_err());
    }
}
```

- [ ] **Step 2: Run**

Run: `cargo test -p jfmt-core worker`
Expected: 3 passed.

- [ ] **Step 3: Commit**

```bash
git add crates/jfmt-core/src/ndjson/worker.rs
git commit -m "feat(core): add ndjson worker thread with panic catch"
```

---

## Task 6: Reorder + writer

**Files:**
- Modify: `crates/jfmt-core/src/ndjson/reorder.rs`

Reorder consumes `WorkerOutput`s from N workers (arriving
out-of-order), buffers in a min-heap keyed by seq, and writes
contiguous `Ok` chunks to output followed by `\n`. `Err` records go
into `errors`. On first `Err` with `fail_fast=true`, sets the cancel
flag and drains any already-pushed results.

- [ ] **Step 1: Write failing tests**

```rust
// crates/jfmt-core/src/ndjson/reorder.rs
//! Reorder buffer + output writer.

use crate::ndjson::worker::WorkerOutput;
use crate::ndjson::LineError;
use crossbeam_channel::Receiver;
use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

pub struct ReorderOutcome {
    /// Sorted by line number.
    pub errors: Vec<(u64, LineError)>,
    /// Total number of items processed (Ok + Err).
    pub processed: u64,
}

/// Drain `rx`, emit `Ok(bytes)` followed by `\n` to `out` in seq
/// order. Collect `Err`s. On first `Err` when `fail_fast`, set
/// `cancel` and finish draining already-queued items.
pub fn run_reorder<W: Write>(
    mut out: W,
    rx: Receiver<WorkerOutput>,
    cancel: Arc<AtomicBool>,
    fail_fast: bool,
) -> std::io::Result<ReorderOutcome> {
    // Use Reverse to turn BinaryHeap (max-heap) into a min-heap.
    let mut pending: BinaryHeap<Reverse<(u64, Result<Vec<u8>, LineError>)>> = BinaryHeap::new();
    let mut next_seq: u64 = 1;
    let mut errors: Vec<(u64, LineError)> = Vec::new();
    let mut processed: u64 = 0;

    while let Ok(item) = rx.recv() {
        processed += 1;
        pending.push(Reverse(item));

        // Drain any contiguous head items.
        while let Some(Reverse((seq, _))) = pending.peek() {
            // Accept in-order. If there's a gap (missing seqs from
            // skipped blank lines OR later workers), we can still
            // emit because seq numbers come from the splitter and
            // gaps mean the splitter skipped that line — the worker
            // never saw it, so the reorder stage won't see it either.
            // Accept ANY item whose seq >= next_seq; advance
            // next_seq past it.
            if *seq < next_seq {
                // Duplicate (shouldn't happen). Drop.
                let _ = pending.pop();
                continue;
            }
            if *seq > next_seq {
                // We might just need to advance next_seq to match
                // the head (because of blank-line skips).
                next_seq = *seq;
            }
            let Reverse((seq, res)) = pending.pop().unwrap();
            match res {
                Ok(bytes) => {
                    out.write_all(&bytes)?;
                    out.write_all(b"\n")?;
                }
                Err(e) => {
                    errors.push((seq, e));
                    if fail_fast {
                        cancel.store(true, Ordering::Relaxed);
                    }
                }
            }
            next_seq = seq + 1;
        }
    }

    errors.sort_by_key(|(line, _)| *line);
    out.flush()?;
    Ok(ReorderOutcome { errors, processed })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossbeam_channel::unbounded;

    #[test]
    fn emits_output_in_seq_order_even_when_input_is_scrambled() {
        let (tx, rx) = unbounded::<WorkerOutput>();
        tx.send((3, Ok(b"C".to_vec()))).unwrap();
        tx.send((1, Ok(b"A".to_vec()))).unwrap();
        tx.send((2, Ok(b"B".to_vec()))).unwrap();
        drop(tx);

        let mut out = Vec::new();
        let cancel = Arc::new(AtomicBool::new(false));
        let r = run_reorder(&mut out, rx, cancel, false).unwrap();
        assert!(r.errors.is_empty());
        assert_eq!(r.processed, 3);
        assert_eq!(out, b"A\nB\nC\n");
    }

    #[test]
    fn collects_errors_and_continues_without_fail_fast() {
        let (tx, rx) = unbounded::<WorkerOutput>();
        tx.send((
            2,
            Err(LineError {
                line: 2,
                offset: 0,
                column: None,
                message: "bad".into(),
            }),
        ))
        .unwrap();
        tx.send((1, Ok(b"ok1".to_vec()))).unwrap();
        tx.send((3, Ok(b"ok3".to_vec()))).unwrap();
        drop(tx);

        let mut out = Vec::new();
        let cancel = Arc::new(AtomicBool::new(false));
        let r = run_reorder(&mut out, rx, cancel.clone(), false).unwrap();
        assert_eq!(r.errors.len(), 1);
        assert_eq!(r.errors[0].0, 2);
        assert_eq!(out, b"ok1\nok3\n");
        assert!(!cancel.load(Ordering::Relaxed));
    }

    #[test]
    fn fail_fast_raises_cancel_and_still_drains() {
        let (tx, rx) = unbounded::<WorkerOutput>();
        tx.send((1, Ok(b"ok1".to_vec()))).unwrap();
        tx.send((
            2,
            Err(LineError {
                line: 2,
                offset: 0,
                column: None,
                message: "bad".into(),
            }),
        ))
        .unwrap();
        // Items produced AFTER the error but queued before we cancel
        // get drained and written (pretty/minify output of valid
        // lines beyond the bad line is still correct JSON; the bad
        // line just produced no output). The splitter stops sending
        // new work after cancel.
        tx.send((3, Ok(b"ok3".to_vec()))).unwrap();
        drop(tx);

        let mut out = Vec::new();
        let cancel = Arc::new(AtomicBool::new(false));
        let r = run_reorder(&mut out, rx, cancel.clone(), true).unwrap();
        assert_eq!(r.errors.len(), 1);
        assert_eq!(out, b"ok1\nok3\n");
        assert!(cancel.load(Ordering::Relaxed));
    }

    #[test]
    fn handles_seq_gaps_from_skipped_blank_lines() {
        // Splitter skipped line 2 (blank), so worker only sees 1 and 3.
        let (tx, rx) = unbounded::<WorkerOutput>();
        tx.send((1, Ok(b"A".to_vec()))).unwrap();
        tx.send((3, Ok(b"C".to_vec()))).unwrap();
        drop(tx);

        let mut out = Vec::new();
        let cancel = Arc::new(AtomicBool::new(false));
        let _ = run_reorder(&mut out, rx, cancel, false).unwrap();
        assert_eq!(out, b"A\nC\n");
    }
}
```

- [ ] **Step 2: Run**

Run: `cargo test -p jfmt-core reorder`
Expected: 4 passed.

- [ ] **Step 3: Commit**

```bash
git add crates/jfmt-core/src/ndjson/reorder.rs
git commit -m "feat(core): add ndjson reorder + writer stage"
```

---

## Task 7: Assemble `run_ndjson_pipeline`

**Files:**
- Modify: `crates/jfmt-core/src/ndjson/mod.rs`

Replace the Task 3 stub with the real assembly: spawn splitter,
workers, reorder, join them, merge stats.

- [ ] **Step 1: Replace the stub body**

Replace the function body in `crates/jfmt-core/src/ndjson/mod.rs`:

```rust
pub fn run_ndjson_pipeline<R, W, F>(
    input: R,
    output: W,
    f: F,
    opts: NdjsonPipelineOptions,
) -> std::io::Result<PipelineReport>
where
    R: Read + Send + 'static,
    W: Write + Send + 'static,
    F: Fn(&[u8], &mut StatsCollector) -> Result<Vec<u8>, LineError>
        + Send
        + Sync
        + 'static,
{
    use crate::ndjson::reorder::run_reorder;
    use crate::ndjson::splitter::split_lines;
    use crate::ndjson::worker::run_worker;
    use crossbeam_channel::bounded;
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;

    let threads = if opts.threads == 0 {
        num_cpus::get_physical().max(1)
    } else {
        opts.threads
    };
    let cap = if opts.channel_capacity == 0 {
        threads.saturating_mul(4).max(1)
    } else {
        opts.channel_capacity
    };

    let (in_tx, in_rx) = bounded::<crate::ndjson::splitter::LineItem>(cap);
    let (out_tx, out_rx) = bounded::<crate::ndjson::worker::WorkerOutput>(cap);
    let cancel = Arc::new(AtomicBool::new(false));
    let f = Arc::new(f);

    // Splitter thread.
    let split_cancel = Arc::clone(&cancel);
    let splitter_handle = std::thread::spawn(move || split_lines(input, in_tx, split_cancel));

    // Worker threads.
    let mut worker_handles = Vec::with_capacity(threads);
    for _ in 0..threads {
        let rx = in_rx.clone();
        let tx = out_tx.clone();
        let f = Arc::clone(&f);
        let h = std::thread::spawn(move || run_worker(rx, tx, f));
        worker_handles.push(h);
    }
    drop(in_rx);
    drop(out_tx);

    // Reorder thread — owns the output writer.
    let reorder_cancel = Arc::clone(&cancel);
    let fail_fast = opts.fail_fast;
    let reorder_handle =
        std::thread::spawn(move || run_reorder(output, out_rx, reorder_cancel, fail_fast));

    // Join splitter first so we know how many records entered.
    let records = splitter_handle
        .join()
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::Other, "splitter panic"))??;

    // Collect per-worker stats.
    let mut merged = if opts.collect_stats {
        Some(StatsCollector::default())
    } else {
        None
    };
    for h in worker_handles {
        let c = h
            .join()
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::Other, "worker panic"))?;
        if let Some(acc) = merged.as_mut() {
            // Merge by consuming the collector's current Stats.
            let s = c.finish();
            // Temporarily extract accumulated Stats to merge.
            // StatsCollector doesn't expose its Stats; use a roundtrip
            // via a scratch collector.
            let scratch_stats = std::mem::replace(acc, StatsCollector::default()).finish();
            let mut total = scratch_stats;
            total.merge(s);
            // Re-seed the accumulator with the merged totals by
            // tricking it: we can't easily push Stats back into a
            // StatsCollector, so keep merged totals in a parallel
            // variable instead. (Code below is adjusted.)
            let _ = total;
            unreachable!("replaced by the simpler pattern below");
        }
    }

    // NOTE: the merge block above is intentionally left unreachable
    // to make the initial commit fail to compile — the next Step
    // replaces it with the clean pattern that uses a single
    // `Stats` accumulator directly.
    let outcome = reorder_handle
        .join()
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::Other, "reorder panic"))??;

    Ok(PipelineReport {
        records,
        errors: outcome.errors,
        stats: None,
    })
}
```

- [ ] **Step 2: Clean up the merge loop (simpler pattern)**

The Step 1 body deliberately included an unreachable merge block to
force the engineer to look at it. Replace the collector loop + the
final `PipelineReport` construction with the clean version:

```rust
    // Collect per-worker stats into a single merged Stats.
    let mut merged_stats: Option<Stats> = if opts.collect_stats {
        Some(Stats::default())
    } else {
        None
    };
    for h in worker_handles {
        let c = h
            .join()
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::Other, "worker panic"))?;
        if let Some(total) = merged_stats.as_mut() {
            total.merge(c.finish());
        }
    }

    let outcome = reorder_handle
        .join()
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::Other, "reorder panic"))??;

    Ok(PipelineReport {
        records,
        errors: outcome.errors,
        stats: merged_stats,
    })
}
```

(Remove the `unreachable!` block and the first merged attempt. The
final function ends with the block above.)

- [ ] **Step 3: Write a smoke test**

Append to `crates/jfmt-core/src/ndjson/mod.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_worker_end_to_end() {
        let input = b"\"a\"\n\"b\"\n\"c\"\n".to_vec();
        let mut output = Vec::new();
        let report = run_ndjson_pipeline(
            input.as_slice().to_vec().as_slice() as &[u8], // convert to owned? see note
            &mut output as &mut Vec<u8>,                    // same
            |line, collector| {
                collector.begin_record();
                // Uppercase the line; ignore parse.
                let out = line.to_ascii_uppercase();
                collector.end_record(true);
                Ok(out)
            },
            NdjsonPipelineOptions {
                threads: 1,
                channel_capacity: 1,
                fail_fast: false,
                collect_stats: false,
            },
        )
        .unwrap();
        assert_eq!(report.records, 3);
        assert!(report.errors.is_empty());
        assert_eq!(output, b"\"A\"\n\"B\"\n\"C\"\n");
    }
}
```

The `'static` bounds on `R` and `W` mean test callers need owned
values. A cleaner smoke test uses `Cursor<Vec<u8>>`:

```rust
    #[test]
    fn single_worker_end_to_end() {
        use std::io::Cursor;
        let input = Cursor::new(b"\"a\"\n\"b\"\n\"c\"\n".to_vec());
        let output: Vec<u8> = Vec::new();
        let (tx_out, rx_out) = crossbeam_channel::bounded::<Vec<u8>>(1);
        // Use a channel to retrieve the output since it's moved into
        // the reorder thread; send it back at end via a custom
        // wrapper. Simpler: use a writer type that records bytes into
        // an Arc<Mutex<Vec<u8>>>.
        use std::sync::{Arc, Mutex};
        #[derive(Clone)]
        struct SharedBuf(Arc<Mutex<Vec<u8>>>);
        impl std::io::Write for SharedBuf {
            fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(b);
                Ok(b.len())
            }
            fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
        }
        let buf = SharedBuf(Arc::new(Mutex::new(output)));
        let report = run_ndjson_pipeline(
            input,
            buf.clone(),
            |line, collector| {
                collector.begin_record();
                let out = line.to_ascii_uppercase();
                collector.end_record(true);
                Ok(out)
            },
            NdjsonPipelineOptions {
                threads: 1,
                channel_capacity: 1,
                fail_fast: false,
                collect_stats: false,
            },
        )
        .unwrap();
        assert_eq!(report.records, 3);
        assert!(report.errors.is_empty());
        let final_bytes = buf.0.lock().unwrap().clone();
        assert_eq!(final_bytes, b"\"A\"\n\"B\"\n\"C\"\n");
        let _ = (tx_out, rx_out);
    }
```

(Drop the unused `tx_out`/`rx_out` in the final commit; they were
there to illustrate the alternative. The `SharedBuf` pattern is what
stays.)

- [ ] **Step 4: Run**

Run: `cargo test -p jfmt-core ndjson::tests`
Expected: 1 passed (the smoke test).

- [ ] **Step 5: Commit**

```bash
git add crates/jfmt-core/src/ndjson/mod.rs
git commit -m "feat(core): assemble run_ndjson_pipeline"
```

---

## Task 8: Delete M2 serial NDJSON; migrate tests

**Files:**
- Delete: `crates/jfmt-core/src/validate/ndjson.rs`
- Modify: `crates/jfmt-core/src/validate/mod.rs`
- Create: `crates/jfmt-core/tests/ndjson_pipeline.rs`

- [ ] **Step 1: Drop the serial module from `validate/mod.rs`**

Replace `crates/jfmt-core/src/validate/mod.rs` content:

```rust
//! Validation and streaming statistics.

pub mod stats;
pub mod syntax;

pub use stats::{Stats, StatsCollector, StatsConfig, ValueKind};
pub use syntax::validate_syntax;
```

- [ ] **Step 2: Delete the file**

```bash
git rm crates/jfmt-core/src/validate/ndjson.rs
```

- [ ] **Step 3: Migrate the 6 M2 ndjson tests against the new pipeline**

Create `crates/jfmt-core/tests/ndjson_pipeline.rs`:

```rust
//! Integration tests for run_ndjson_pipeline. Supersedes the M2
//! serial NDJSON unit tests.

use jfmt_core::parser::EventReader;
use jfmt_core::{
    run_ndjson_pipeline, Error, LineError, NdjsonPipelineOptions, StatsCollector,
};
use std::io::{Cursor, Write};
use std::sync::{Arc, Mutex};

/// A Write sink that records bytes into an Arc<Mutex<Vec<u8>>> so
/// tests can inspect the output after the pipeline moves it.
#[derive(Clone)]
struct SharedBuf(Arc<Mutex<Vec<u8>>>);

impl SharedBuf {
    fn new() -> Self {
        Self(Arc::new(Mutex::new(Vec::new())))
    }
    fn take(&self) -> Vec<u8> {
        std::mem::take(&mut *self.0.lock().unwrap())
    }
}

impl Write for SharedBuf {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// A validate-style closure: parses each line; on success drains
/// events into the collector and returns an empty Vec. Failures
/// become LineErrors.
fn validate_closure(
    line: &[u8],
    collector: &mut StatsCollector,
) -> Result<Vec<u8>, LineError> {
    collector.begin_record();
    let mut p = EventReader::new(line);
    loop {
        match p.next_event() {
            Ok(None) => break,
            Ok(Some(ev)) => collector.observe(&ev),
            Err(Error::Syntax {
                offset,
                column,
                message,
                ..
            }) => {
                collector.end_record(false);
                return Err(LineError {
                    line: 0, // reorder doesn't rewrite; caller sees splitter's seq
                    offset,
                    column,
                    message,
                });
            }
            Err(e) => {
                collector.end_record(false);
                return Err(LineError {
                    line: 0,
                    offset: 0,
                    column: None,
                    message: format!("{e}"),
                });
            }
        }
    }
    if let Err(Error::Syntax {
        offset,
        column,
        message,
        ..
    }) = p.finish()
    {
        collector.end_record(false);
        return Err(LineError {
            line: 0,
            offset,
            column,
            message,
        });
    }
    collector.end_record(true);
    Ok(Vec::new())
}

#[test]
fn accepts_all_valid_lines() {
    let buf = SharedBuf::new();
    let input = Cursor::new(b"1\n\"hi\"\n{\"a\":1}\n".to_vec());
    let report = run_ndjson_pipeline(
        input,
        buf.clone(),
        validate_closure,
        NdjsonPipelineOptions {
            threads: 1,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert_eq!(report.records, 3);
}

#[test]
fn reports_bad_line_keeps_going() {
    let buf = SharedBuf::new();
    let input = Cursor::new(b"1\n{bad}\n3\n".to_vec());
    let report = run_ndjson_pipeline(
        input,
        buf,
        validate_closure,
        NdjsonPipelineOptions {
            threads: 1,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(report.errors.len(), 1);
    // The seq number from the splitter is line 2 (blank lines don't
    // exist in this input — line 2 was the bad one).
    assert_eq!(report.errors[0].0, 2);
}

#[test]
fn fail_fast_stops_on_first_error() {
    let buf = SharedBuf::new();
    let input = Cursor::new(b"{bad1}\n{bad2}\n1\n".to_vec());
    let report = run_ndjson_pipeline(
        input,
        buf,
        validate_closure,
        NdjsonPipelineOptions {
            threads: 1,
            fail_fast: true,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(report.errors.len(), 1);
    assert_eq!(report.errors[0].0, 1);
}

#[test]
fn skips_blank_lines() {
    let buf = SharedBuf::new();
    let input = Cursor::new(b"1\n\n\n2\n".to_vec());
    let report = run_ndjson_pipeline(
        input,
        buf,
        validate_closure,
        NdjsonPipelineOptions {
            threads: 1,
            collect_stats: true,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(report.errors.is_empty());
    assert_eq!(report.records, 2);
    assert_eq!(report.stats.as_ref().unwrap().records, 2);
}

#[test]
fn stats_count_top_level_types_across_lines() {
    let buf = SharedBuf::new();
    let input = Cursor::new(b"1\n\"hi\"\n{\"a\":1}\n[1,2]\n".to_vec());
    let report = run_ndjson_pipeline(
        input,
        buf,
        validate_closure,
        NdjsonPipelineOptions {
            threads: 1,
            collect_stats: true,
            ..Default::default()
        },
    )
    .unwrap();
    let s = report.stats.unwrap();
    assert_eq!(s.records, 4);
    assert_eq!(s.top_level_types.get("number"), Some(&1));
    assert_eq!(s.top_level_types.get("string"), Some(&1));
    assert_eq!(s.top_level_types.get("object"), Some(&1));
    assert_eq!(s.top_level_types.get("array"), Some(&1));
}

#[test]
fn detects_trailing_garbage_on_line() {
    let buf = SharedBuf::new();
    let input = Cursor::new(b"{\"a\":1} junk\n".to_vec());
    let report = run_ndjson_pipeline(
        input,
        buf,
        validate_closure,
        NdjsonPipelineOptions {
            threads: 1,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(report.errors.len(), 1);
}
```

- [ ] **Step 4: Run**

Run: `cargo test -p jfmt-core --test ndjson_pipeline`
Expected: 6 passed.

Run: `cargo test -p jfmt-core` (full crate)
Expected: everything passes; no references to the removed module.

- [ ] **Step 5: Commit**

```bash
git add crates/jfmt-core
git commit -m "refactor(core): migrate M2 serial ndjson tests to pipeline"
```

---

## Task 9: Parallel correctness tests (property + scramble)

**Files:**
- Modify: `crates/jfmt-core/tests/ndjson_pipeline.rs`

- [ ] **Step 1: Add deterministic scramble test**

Append to `crates/jfmt-core/tests/ndjson_pipeline.rs`:

```rust
#[test]
fn parallel_output_matches_serial_byte_for_byte() {
    // 1000 lines, each a small object. Run at threads=1 and
    // threads=8 and confirm bytes are identical.
    let mut input = Vec::new();
    for i in 0..1000u32 {
        input.extend_from_slice(format!("{{\"i\":{i}}}\n").as_bytes());
    }
    let passthrough = |line: &[u8], c: &mut StatsCollector| -> Result<Vec<u8>, LineError> {
        c.begin_record();
        c.end_record(true);
        Ok(line.to_vec())
    };

    let buf1 = SharedBuf::new();
    let buf8 = SharedBuf::new();
    let report1 = run_ndjson_pipeline(
        Cursor::new(input.clone()),
        buf1.clone(),
        passthrough,
        NdjsonPipelineOptions {
            threads: 1,
            ..Default::default()
        },
    )
    .unwrap();
    let report8 = run_ndjson_pipeline(
        Cursor::new(input.clone()),
        buf8.clone(),
        passthrough,
        NdjsonPipelineOptions {
            threads: 8,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(report1.records, 1000);
    assert_eq!(report8.records, 1000);
    assert_eq!(buf1.take(), buf8.take());
}

#[test]
fn parallel_stats_match_serial_stats() {
    let mut input = Vec::new();
    input.extend_from_slice(b"{\"a\":1}\n");
    input.extend_from_slice(b"{\"a\":2,\"b\":3}\n");
    input.extend_from_slice(b"[1,2,3]\n");
    input.extend_from_slice(b"\"hi\"\n");

    let buf1 = SharedBuf::new();
    let buf4 = SharedBuf::new();
    let r1 = run_ndjson_pipeline(
        Cursor::new(input.clone()),
        buf1,
        validate_closure,
        NdjsonPipelineOptions {
            threads: 1,
            collect_stats: true,
            ..Default::default()
        },
    )
    .unwrap();
    let r4 = run_ndjson_pipeline(
        Cursor::new(input.clone()),
        buf4,
        validate_closure,
        NdjsonPipelineOptions {
            threads: 4,
            collect_stats: true,
            ..Default::default()
        },
    )
    .unwrap();
    let s1 = r1.stats.unwrap();
    let s4 = r4.stats.unwrap();
    assert_eq!(s1.records, s4.records);
    assert_eq!(s1.valid, s4.valid);
    assert_eq!(s1.invalid, s4.invalid);
    assert_eq!(s1.top_level_types, s4.top_level_types);
    assert_eq!(s1.top_level_keys, s4.top_level_keys);
    assert_eq!(s1.max_depth, s4.max_depth);
}
```

- [ ] **Step 2: Run**

Run: `cargo test -p jfmt-core --test ndjson_pipeline`
Expected: 8 passed.

- [ ] **Step 3: Commit**

```bash
git add crates/jfmt-core/tests/ndjson_pipeline.rs
git commit -m "test(core): verify parallel pipeline matches serial"
```

---

## Task 10: `jfmt-io` output supports `Send`

**Files:**
- Modify: `crates/jfmt-io/src/output.rs`

The pipeline's reorder thread owns the writer, so it needs
`Write + Send + 'static`. M1's `open_output` returns
`Box<dyn Write>` using `io::stdout().lock()` which is lifetime-bound.
Change to `Box<dyn Write + Send>` and use `io::stdout()` (owned).

- [ ] **Step 1: Update `open_output` signature + stdout path**

Edit `crates/jfmt-io/src/output.rs`:

```rust
pub fn open_output(spec: &OutputSpec) -> io::Result<Box<dyn Write + Send>> {
    let raw: Box<dyn Write + Send> = match &spec.path {
        Some(p) => Box::new(File::create(p)?),
        None => Box::new(io::stdout()),
    };

    let compression = spec
        .compression
        .unwrap_or_else(|| match spec.path.as_deref() {
            Some(p) => Compression::from_path(p),
            None => Compression::None,
        });

    let encoded: Box<dyn Write + Send> = match compression {
        Compression::None => raw,
        Compression::Gzip => Box::new(GzEncoder::new(
            raw,
            flate2::Compression::new(spec.gzip_level),
        )),
        Compression::Zstd => {
            Box::new(zstd::stream::Encoder::new(raw, spec.zstd_level)?.auto_finish())
        }
    };

    Ok(Box::new(BufWriter::with_capacity(64 * 1024, encoded)))
}
```

- [ ] **Step 2: Run existing tests**

Run: `cargo test -p jfmt-io`
Expected: 9 passed (no test changes needed; trait bound is a
supertrait).

- [ ] **Step 3: Verify CLI still compiles**

Run: `cargo build -p jfmt-cli`
Expected: compiles.

- [ ] **Step 4: Commit**

```bash
git add crates/jfmt-io/src/output.rs
git commit -m "refactor(io): return Write+Send so pipeline can own output"
```

---

## Task 11: Global `--threads` CLI flag

**Files:**
- Modify: `crates/jfmt-cli/src/cli.rs`

- [ ] **Step 1: Add a top-level flag**

Edit `crates/jfmt-cli/src/cli.rs`. Extend the `Cli` struct:

```rust
#[derive(Debug, Parser)]
#[command(name = "jfmt", version, about = "Streaming JSON/NDJSON formatter")]
pub struct Cli {
    /// Worker threads for --ndjson pipelines. 0 = physical cores;
    /// 1 = serial; >=2 = parallel. Ignored in single-document mode.
    #[arg(long = "threads", global = true, default_value_t = 0)]
    pub threads: usize,

    #[command(subcommand)]
    pub command: Command,
}
```

- [ ] **Step 2: Verify `--help` prints the flag**

Run: `cargo build -p jfmt-cli && ./target/debug/jfmt --help | grep threads`
Expected: line containing `--threads`.

- [ ] **Step 3: Commit**

```bash
git add crates/jfmt-cli/src/cli.rs
git commit -m "feat(cli): add global --threads flag"
```

---

## Task 12: Route `jfmt pretty --ndjson` through pipeline

**Files:**
- Modify: `crates/jfmt-cli/src/commands/pretty.rs`

- [ ] **Step 1: Rewrite the command**

Replace `crates/jfmt-cli/src/commands/pretty.rs`:

```rust
use crate::cli::PrettyArgs;
use anyhow::Context;
use jfmt_core::{
    run_ndjson_pipeline, transcode, LineError, NdjsonPipelineOptions, PrettyConfig,
    PrettyWriter, StatsCollector,
};

pub fn run(args: PrettyArgs, threads: usize) -> anyhow::Result<()> {
    let cfg = PrettyConfig {
        indent: args.indent,
        use_tabs: args.tabs,
        newline: "\n",
    };

    if args.common.ndjson {
        let input =
            jfmt_io::open_input(&args.common.input_spec()).context("opening input")?;
        let output =
            jfmt_io::open_output(&args.common.output_spec()).context("opening output")?;
        let opts = NdjsonPipelineOptions {
            threads,
            fail_fast: true, // pretty: abort on bad line
            collect_stats: false,
            ..Default::default()
        };
        let cfg_for_closure = cfg;
        let closure = move |line: &[u8], _c: &mut StatsCollector| -> Result<Vec<u8>, LineError> {
            let mut out = Vec::with_capacity(line.len() * 2);
            let writer = PrettyWriter::with_config(&mut out, cfg_for_closure);
            match transcode(line, writer) {
                Ok(()) => {
                    // Pretty emits a trailing newline; strip it because
                    // the reorder stage adds one per record.
                    if out.ends_with(b"\n") {
                        out.pop();
                    }
                    Ok(out)
                }
                Err(e) => match e {
                    jfmt_core::Error::Syntax {
                        offset,
                        column,
                        message,
                        ..
                    } => Err(LineError {
                        line: 0,
                        offset,
                        column,
                        message,
                    }),
                    other => Err(LineError {
                        line: 0,
                        offset: 0,
                        column: None,
                        message: format!("{other}"),
                    }),
                },
            }
        };
        let report = run_ndjson_pipeline(input, output, closure, opts)
            .context("pretty-printing failed")?;
        for (seq, le) in &report.errors {
            eprintln!(
                "line {seq}: syntax error at byte {}: {}",
                le.offset, le.message
            );
        }
        if !report.errors.is_empty() {
            return Err(anyhow::Error::from(crate::SilentExit(
                crate::exit::ExitCode::SyntaxError,
            )));
        }
        return Ok(());
    }

    // Single-document mode — unchanged.
    let input = jfmt_io::open_input(&args.common.input_spec()).context("opening input")?;
    let output = jfmt_io::open_output(&args.common.output_spec()).context("opening output")?;
    let writer = PrettyWriter::with_config(output, cfg);
    transcode(input, writer).context("pretty-printing failed")?;
    Ok(())
}
```

- [ ] **Step 2: Pass `threads` from main.rs**

Edit `crates/jfmt-cli/src/main.rs`. Update `run`:

```rust
fn run(cli: Cli) -> anyhow::Result<()> {
    let threads = cli.threads;
    match cli.command {
        Command::Pretty(args) => commands::pretty::run(args, threads),
        Command::Minify(args) => commands::minify::run(args, threads),
        Command::Validate(args) => commands::validate::run(args, threads),
    }
}
```

(Tasks 13 and 14 update `minify::run` and `validate::run` to accept
`threads`; for now give them the same `_threads` parameter signature
to keep the binary compiling.)

Edit `crates/jfmt-cli/src/commands/minify.rs`:

```rust
pub fn run(args: MinifyArgs, _threads: usize) -> anyhow::Result<()> {
    // body unchanged
}
```

Edit `crates/jfmt-cli/src/commands/validate.rs`:

```rust
pub fn run(args: ValidateArgs, _threads: usize) -> anyhow::Result<()> {
    // body unchanged
}
```

- [ ] **Step 3: Smoke-test**

Run:
```bash
cargo build -p jfmt-cli
printf '{"a":1}\n{"b":2}\n' | ./target/debug/jfmt pretty --ndjson --threads 2
```
Expected:
```
{
  "a": 1
}
{
  "b": 2
}
```

- [ ] **Step 4: Commit**

```bash
git add crates/jfmt-cli
git commit -m "feat(cli): route 'pretty --ndjson' through pipeline"
```

---

## Task 13: Route `jfmt minify --ndjson` through pipeline

**Files:**
- Modify: `crates/jfmt-cli/src/commands/minify.rs`

- [ ] **Step 1: Rewrite**

Replace `crates/jfmt-cli/src/commands/minify.rs`:

```rust
use crate::cli::MinifyArgs;
use anyhow::Context;
use jfmt_core::{
    run_ndjson_pipeline, transcode, LineError, MinifyWriter, NdjsonPipelineOptions,
    StatsCollector,
};

pub fn run(args: MinifyArgs, threads: usize) -> anyhow::Result<()> {
    if args.common.ndjson {
        let input =
            jfmt_io::open_input(&args.common.input_spec()).context("opening input")?;
        let output =
            jfmt_io::open_output(&args.common.output_spec()).context("opening output")?;
        let opts = NdjsonPipelineOptions {
            threads,
            fail_fast: true,
            collect_stats: false,
            ..Default::default()
        };
        let closure = |line: &[u8], _c: &mut StatsCollector| -> Result<Vec<u8>, LineError> {
            let mut out = Vec::with_capacity(line.len());
            let writer = MinifyWriter::new(&mut out);
            match transcode(line, writer) {
                Ok(()) => Ok(out),
                Err(e) => match e {
                    jfmt_core::Error::Syntax {
                        offset,
                        column,
                        message,
                        ..
                    } => Err(LineError {
                        line: 0,
                        offset,
                        column,
                        message,
                    }),
                    other => Err(LineError {
                        line: 0,
                        offset: 0,
                        column: None,
                        message: format!("{other}"),
                    }),
                },
            }
        };
        let report = run_ndjson_pipeline(input, output, closure, opts)
            .context("minify failed")?;
        for (seq, le) in &report.errors {
            eprintln!(
                "line {seq}: syntax error at byte {}: {}",
                le.offset, le.message
            );
        }
        if !report.errors.is_empty() {
            return Err(anyhow::Error::from(crate::SilentExit(
                crate::exit::ExitCode::SyntaxError,
            )));
        }
        return Ok(());
    }

    let input = jfmt_io::open_input(&args.common.input_spec()).context("opening input")?;
    let output = jfmt_io::open_output(&args.common.output_spec()).context("opening output")?;
    let writer = MinifyWriter::new(output);
    transcode(input, writer).context("minifying failed")?;
    Ok(())
}
```

- [ ] **Step 2: Smoke-test**

Run:
```bash
cargo build -p jfmt-cli
printf '  { "a" : 1 }\n [ 1,  2 ]\n' | ./target/debug/jfmt minify --ndjson --threads 2
```
Expected:
```
{"a":1}
[1,2]
```

- [ ] **Step 3: Commit**

```bash
git add crates/jfmt-cli/src/commands/minify.rs
git commit -m "feat(cli): route 'minify --ndjson' through pipeline"
```

---

## Task 14: Route `jfmt validate --ndjson` through pipeline

**Files:**
- Modify: `crates/jfmt-cli/src/commands/validate.rs`

- [ ] **Step 1: Rewrite the NDJSON branch**

Replace `crates/jfmt-cli/src/commands/validate.rs`:

```rust
use crate::cli::ValidateArgs;
use crate::exit::ExitCode;
use crate::SilentExit;
use anyhow::{Context, Result};
use jfmt_core::parser::EventReader;
use jfmt_core::{
    run_ndjson_pipeline, validate_syntax, Error, LineError, NdjsonPipelineOptions, Stats,
    StatsCollector,
};
use std::fs::File;
use std::io::{BufWriter, Write};

pub fn run(args: ValidateArgs, threads: usize) -> Result<()> {
    let input = jfmt_io::open_input(&args.common.input_spec()).context("opening input")?;
    let collect_stats = args.stats || args.stats_json.is_some();

    let (any_bad, stats) = if args.common.ndjson {
        // Route through the parallel pipeline; discard its data output.
        let sink = std::io::sink();
        let opts = NdjsonPipelineOptions {
            threads,
            fail_fast: args.fail_fast,
            collect_stats,
            ..Default::default()
        };
        let closure = |line: &[u8], c: &mut StatsCollector| -> Result<Vec<u8>, LineError> {
            c.begin_record();
            let mut p = EventReader::new(line);
            loop {
                match p.next_event() {
                    Ok(None) => break,
                    Ok(Some(ev)) => c.observe(&ev),
                    Err(Error::Syntax {
                        offset,
                        column,
                        message,
                        ..
                    }) => {
                        c.end_record(false);
                        return Err(LineError {
                            line: 0,
                            offset,
                            column,
                            message,
                        });
                    }
                    Err(e) => {
                        c.end_record(false);
                        return Err(LineError {
                            line: 0,
                            offset: 0,
                            column: None,
                            message: format!("{e}"),
                        });
                    }
                }
            }
            if let Err(Error::Syntax {
                offset,
                column,
                message,
                ..
            }) = p.finish()
            {
                c.end_record(false);
                return Err(LineError {
                    line: 0,
                    offset,
                    column,
                    message,
                });
            }
            c.end_record(true);
            Ok(Vec::new())
        };
        let report = run_ndjson_pipeline(input, sink, closure, opts).context("reading input")?;

        for (seq, le) in &report.errors {
            let col = le
                .column
                .map(|c| format!("col {c} "))
                .unwrap_or_default();
            eprintln!(
                "line {seq}: {}syntax error at byte {}: {}",
                col, le.offset, le.message
            );
        }
        (!report.errors.is_empty(), report.stats)
    } else if collect_stats {
        let mut c = StatsCollector::default();
        c.begin_record();
        let mut r = EventReader::new(input);
        let parse_result = loop {
            match r.next_event() {
                Ok(None) => break Ok(()),
                Ok(Some(ev)) => c.observe(&ev),
                Err(e) => break Err(e),
            }
        };
        let finish_result = parse_result.and_then(|_| r.finish());
        match finish_result {
            Ok(()) => {
                c.end_record(true);
                (false, Some(c.finish()))
            }
            Err(e) => {
                c.end_record(false);
                let _ = c.finish();
                return Err(anyhow::Error::from(e).context("validation failed"));
            }
        }
    } else {
        validate_syntax(input).context("validation failed")?;
        (false, None)
    };

    if let Some(s) = stats.as_ref() {
        if args.stats {
            eprint!("{s}");
        }
        if let Some(path) = args.stats_json.as_ref() {
            write_stats_json(path, s).context("writing --stats-json")?;
        }
    }

    if any_bad {
        return Err(anyhow::Error::from(SilentExit(ExitCode::SyntaxError)));
    }
    Ok(())
}

fn write_stats_json(path: &std::path::Path, stats: &Stats) -> std::io::Result<()> {
    let f = File::create(path)?;
    let mut w = BufWriter::new(f);
    serde_json::to_writer_pretty(&mut w, stats)?;
    w.write_all(b"\n")?;
    Ok(())
}
```

- [ ] **Step 2: Smoke-test both branches**

```bash
cargo build -p jfmt-cli
echo '{"a":1}' | ./target/debug/jfmt validate             # exit 0
printf '{"ok":1}\n{bad\n{"ok":2}\n' | ./target/debug/jfmt validate --ndjson --threads 2 2>&1
# expected:
# line 2: col 1 syntax error at byte 1: ExpectingMemberNameOrObjectEnd
# exit 2
echo $?
```

- [ ] **Step 3: Commit**

```bash
git add crates/jfmt-cli/src/commands/validate.rs
git commit -m "feat(cli): route 'validate --ndjson' through pipeline"
```

---

## Task 15: CLI e2e — parity across `--threads`

**Files:**
- Create: `crates/jfmt-cli/tests/fixtures/ndjson-many.ndjson`
- Modify: `crates/jfmt-cli/tests/cli_validate.rs`
- Modify: `crates/jfmt-cli/tests/cli_pretty.rs`
- Modify: `crates/jfmt-cli/tests/cli_minify.rs`

- [ ] **Step 1: Generate a larger NDJSON fixture (committed as text)**

Create `crates/jfmt-cli/tests/fixtures/ndjson-many.ndjson` with 100
lines:

Write a tiny helper block that generates the fixture at test-build
time instead — avoid committing 100 near-identical lines. Append to
one of the test files a `fn ndjson_many()` that regenerates the
fixture on first use. For simplicity, write the fixture directly
here — the file is small enough (~2KB).

Use this content (100 lines of `{"i":N,"label":"row_N"}`):

```
{"i":0,"label":"row_0"}
{"i":1,"label":"row_1"}
```
… repeating up to
```
{"i":99,"label":"row_99"}
```

- [ ] **Step 2: Add parity tests to each CLI test file**

Append to `crates/jfmt-cli/tests/cli_pretty.rs`:

```rust
#[test]
fn pretty_ndjson_parallel_matches_serial() {
    let bin = Command::cargo_bin("jfmt").unwrap();
    let serial = bin
        .clone()
        .arg("pretty")
        .arg("--ndjson")
        .arg("--threads")
        .arg("1")
        .arg(fixture("ndjson-many.ndjson"))
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let parallel = Command::cargo_bin("jfmt")
        .unwrap()
        .arg("pretty")
        .arg("--ndjson")
        .arg("--threads")
        .arg("4")
        .arg(fixture("ndjson-many.ndjson"))
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    assert_eq!(serial, parallel);
}
```

Note: `Command::cargo_bin(...)?.clone()` — `assert_cmd::Command` is
`Clone`, but the output is captured only on `.assert()`. The snippet
above uses two separate `cargo_bin` invocations, which is clearer.

Append to `crates/jfmt-cli/tests/cli_minify.rs`:

```rust
#[test]
fn minify_ndjson_parallel_matches_serial() {
    let serial = Command::cargo_bin("jfmt")
        .unwrap()
        .arg("minify")
        .arg("--ndjson")
        .arg("--threads")
        .arg("1")
        .arg(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ndjson-many.ndjson"))
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let parallel = Command::cargo_bin("jfmt")
        .unwrap()
        .arg("minify")
        .arg("--ndjson")
        .arg("--threads")
        .arg("4")
        .arg(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ndjson-many.ndjson"))
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    assert_eq!(serial, parallel);
}
```

Append to `crates/jfmt-cli/tests/cli_validate.rs`:

```rust
#[test]
fn validate_ndjson_parallel_matches_serial_stats() {
    use std::process::Command as StdCommand;
    // We need both --stats-json outputs to compare.
    let dir = tempfile::tempdir().unwrap();
    let s1 = dir.path().join("s1.json");
    let s4 = dir.path().join("s4.json");

    for (path, threads) in [(&s1, "1"), (&s4, "4")] {
        Command::cargo_bin("jfmt")
            .unwrap()
            .arg("validate")
            .arg("--ndjson")
            .arg("--threads")
            .arg(threads)
            .arg("--stats-json")
            .arg(path)
            .arg(fixture("ndjson-many.ndjson"))
            .assert()
            .success();
    }

    let v1: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&s1).unwrap()).unwrap();
    let v4: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&s4).unwrap()).unwrap();
    assert_eq!(v1, v4);
    // Don't leave StdCommand unused:
    let _ = StdCommand::new("true");
}
```

(Strip unused imports at commit time.)

- [ ] **Step 3: Update the ndjson-mixed stats test**

The M2 test `validate_ndjson_stats_counts_valid_and_invalid` should
still pass unchanged (parallel pipeline at default threads runs the
same closure). Run the full suite:

Run: `cargo test -p jfmt-cli`
Expected: all existing + 3 new = 18 + 3 = 21 passed (approx — may
differ if pretty/minify had 7/3 tests).

- [ ] **Step 4: Commit**

```bash
git add crates/jfmt-cli/tests
git commit -m "test(cli): add parallel-vs-serial parity tests"
```

---

## Task 16: README + spec update

**Files:**
- Modify: `README.md`
- Modify: `docs/superpowers/specs/2026-04-23-jfmt-phase1-design.md`

- [ ] **Step 1: README**

Edit `README.md`:

- Change status line to:
  ```
  **M3 preview (v0.0.3)** — `pretty`, `minify`, `validate` with a
  multi-threaded NDJSON pipeline.
  ```
- Append a new subsection before `## Exit codes`:

  ```markdown
  ### Parallelism

  The `--ndjson` pipeline runs splitter → N workers → reorder on
  separate threads. Control with:

  ```bash
  jfmt pretty   --ndjson --threads 8 big.ndjson      # 8 workers
  jfmt validate --ndjson --threads 1 big.ndjson      # force serial
  jfmt minify   --ndjson big.ndjson                  # default = physical cores
  ```

  Output is always written in input order. `--threads` is silently
  ignored in single-document mode.
  ```

- [ ] **Step 2: Spec milestone update**

Edit `docs/superpowers/specs/2026-04-23-jfmt-phase1-design.md`, append
under the milestone table:

```markdown
| M3 ✓ | Shipped v0.0.3 on 2026-04-XX (tag `v0.0.3`) |
```

(Fill the actual date at commit time.)

- [ ] **Step 3: Commit**

```bash
git add README.md docs/superpowers/specs
git commit -m "docs: document --threads and mark M3 shipped"
```

---

## Task 17: Ship `v0.0.3`

**Files:**
- Modify: `Cargo.toml`

- [ ] **Step 1: Verify workspace green**

Run: `cargo fmt --all -- --check`
Expected: exit 0.

Run: `cargo clippy --workspace --all-targets -- -D warnings`
Expected: exit 0.

Run: `cargo test --workspace`
Expected: ~100+ tests pass (84 from M2 + ~20 M3 new).

If fmt reports changes, run `cargo fmt --all` and commit as
`chore: apply rustfmt`.

- [ ] **Step 2: Bump version**

```toml
# Cargo.toml (workspace.package)
version = "0.0.3"
```

Run: `cargo build --workspace` to refresh Cargo.lock.

Commit:
```bash
git add Cargo.toml Cargo.lock
git commit -m "chore: bump version to 0.0.3"
```

- [ ] **Step 3: Tag**

```bash
git tag -a v0.0.3 -m "M3: multi-threaded NDJSON pipeline"
```

Do not push without explicit user approval.

---

## Self-Review

### Spec coverage

- §3 public API (`run_ndjson_pipeline`, `NdjsonPipelineOptions`,
  `PipelineReport`, `LineError`) → Task 3 (skeleton) + Task 7
  (assembly). ✓
- §4 internal modules (`splitter`, `worker`, `reorder`, `merge`) →
  Tasks 4, 5, 6 cover splitter/worker/reorder. `merge` = `Stats::merge`
  in Task 2. ✓
- §4.2 cancellation via `Arc<AtomicBool>` → Task 4 (splitter checks),
  Task 6 (reorder sets), Task 7 (wiring). ✓
- §4.3 ordering guarantees → Task 6 (reorder by seq, errors sorted). ✓
- §5 integration table — pretty (Task 12), minify (Task 13), validate
  (Task 14). ✓
- §6 CLI `--threads` flag → Task 11. ✓
- §7 deletion of `validate/ndjson.rs` + test migration → Task 8. ✓
- §8 new deps → Task 1. ✓
- §9 testing:
  - Unit: splitter (Task 4), worker (Task 5), reorder (Task 6),
    merge (Task 2). ✓
  - Integration: migrated + parallel parity + stats parity → Tasks 8,
    9. ✓
  - CLI e2e parity → Task 15. ✓
  - Property test (pipeline(N) == pipeline(1)) → Task 9 covers this
    via the scramble/parallel parity tests; a true proptest is not
    added because the parity tests give high confidence at less cost.
    **Gap acknowledged, not blocking.**
- §10 risk #4 (panic safety) → Task 5 wraps in `catch_unwind`. ✓
- §11 exit criteria → Task 17 verifies all three.

### Placeholder scan

- No "TBD" / "TODO" / "implement later" in steps.
- Task 7 Step 1 intentionally contains an `unreachable!` block for
  pedagogical reasons; Step 2 explicitly replaces it. Not a
  placeholder — the replacement code is given in full.
- Fixture file in Task 15 is described rather than pasted; acceptable
  because it's 100 mechanical lines whose content is spelled out
  (`{"i":N,"label":"row_N"}` for N = 0..99). An engineer can generate
  this with `seq 0 99 | awk '{printf "{\"i\":%s,\"label\":\"row_%s\"}\n", $1, $1}'`
  or equivalent.

### Type consistency

- `LineError` — defined in Task 3 at `crates/jfmt-core/src/ndjson/mod.rs`,
  used by splitter (Task 4 doesn't construct it), worker (Task 5
  constructs on panic), reorder (Task 6 stores), and all three CLI
  commands (Tasks 12-14). Signature fields (`line`, `offset`,
  `column`, `message`) consistent across.
- `NdjsonPipelineOptions` fields consistent: `threads`,
  `channel_capacity`, `fail_fast`, `collect_stats`.
- `PipelineReport` fields: `records`, `errors`, `stats`.
- `LineItem = (u64, Vec<u8>)` and `WorkerOutput = (u64, Result<Vec<u8>, LineError>)`
  — defined in splitter / worker modules, consumed by reorder and
  assembly.

### Scope check

One focused plan, one milestone, one release. Files to touch are
bounded. No gratuitous refactoring.

---

## Execution Handoff

Plan saved to `docs/superpowers/plans/2026-04-25-jfmt-m3-ndjson-pipeline.md`.

**1. Subagent-Driven (recommended)** — fresh subagent per task, review between tasks
**2. Inline Execution** — user-paced via the `jfmt-iterate` skill as in M1/M2

Which approach?
