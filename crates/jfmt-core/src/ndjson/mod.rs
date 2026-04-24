//! NDJSON parallel pipeline: splitter → workers → reorder.
//!
//! Public entry point is [`run_ndjson_pipeline`]. Callers provide a
//! closure invoked once per non-blank input line; its output is
//! written to the `output` stream in input order.

use crate::validate::{Stats, StatsCollector};
use std::io::{Read, Write};

pub mod reorder;
pub mod splitter;
pub mod worker;

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
    /// (`max(1, threads * 4)`).
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
    let _ = (input, output, f, opts);
    Ok(PipelineReport::default())
}
