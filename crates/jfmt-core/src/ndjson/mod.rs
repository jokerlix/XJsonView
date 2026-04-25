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

#[derive(Debug, Clone, Copy, Default)]
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
    F: Fn(&[u8], &mut StatsCollector) -> Result<Vec<Vec<u8>>, LineError> + Send + Sync + 'static,
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

    let split_cancel = Arc::clone(&cancel);
    let splitter_handle = std::thread::spawn(move || split_lines(input, in_tx, split_cancel));

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

    let reorder_cancel = Arc::clone(&cancel);
    let fail_fast = opts.fail_fast;
    let reorder_handle =
        std::thread::spawn(move || run_reorder(output, out_rx, reorder_cancel, fail_fast));

    let records = splitter_handle
        .join()
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::Other, "splitter panic"))??;

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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use std::sync::{Arc as StdArc, Mutex};

    #[derive(Clone)]
    struct SharedBuf(StdArc<Mutex<Vec<u8>>>);
    impl SharedBuf {
        fn new() -> Self {
            Self(StdArc::new(Mutex::new(Vec::new())))
        }
    }
    impl std::io::Write for SharedBuf {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn single_worker_end_to_end() {
        let input = Cursor::new(b"\"a\"\n\"b\"\n\"c\"\n".to_vec());
        let buf = SharedBuf::new();
        let report = run_ndjson_pipeline(
            input,
            buf.clone(),
            |line, collector| {
                collector.begin_record();
                let out = line.to_ascii_uppercase();
                collector.end_record(true);
                Ok(vec![out])
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
        let bytes = buf.0.lock().unwrap().clone();
        assert_eq!(bytes, b"\"A\"\n\"B\"\n\"C\"\n");
    }
}
