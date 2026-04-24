//! Reorder buffer + output writer.

use crate::ndjson::worker::WorkerOutput;
use crate::ndjson::LineError;
use crossbeam_channel::Receiver;
use std::cmp::Ordering as CmpOrdering;
use std::collections::BinaryHeap;
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Heap entry: orders by seq descending so BinaryHeap (max-heap) pops
/// the smallest seq first. The payload's ordering is never used.
struct Entry {
    seq: u64,
    payload: Result<Vec<u8>, LineError>,
}

impl PartialEq for Entry {
    fn eq(&self, other: &Self) -> bool {
        self.seq == other.seq
    }
}
impl Eq for Entry {}
impl PartialOrd for Entry {
    fn partial_cmp(&self, other: &Self) -> Option<CmpOrdering> {
        Some(self.cmp(other))
    }
}
impl Ord for Entry {
    fn cmp(&self, other: &Self) -> CmpOrdering {
        // Reverse seq order → min-heap by seq.
        other.seq.cmp(&self.seq)
    }
}

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
    let mut pending: BinaryHeap<Entry> = BinaryHeap::new();
    let mut next_seq: u64 = 1;
    let mut errors: Vec<(u64, LineError)> = Vec::new();
    let mut processed: u64 = 0;

    // During streaming, only emit items whose seq == next_seq to
    // avoid skipping over later-arriving earlier items. When the
    // channel closes, drain remaining items (gaps are real — caused
    // by blank lines the splitter skipped).
    while let Ok((seq, payload)) = rx.recv() {
        processed += 1;
        pending.push(Entry { seq, payload });

        while pending.peek().map(|e| e.seq) == Some(next_seq) {
            let Entry { seq, payload } = pending.pop().unwrap();
            emit(&mut out, &mut errors, &cancel, fail_fast, seq, payload)?;
            next_seq = seq + 1;
        }
    }

    // Channel closed; drain the rest in seq order (any gaps are
    // permanent — they came from blank lines).
    while let Some(Entry { seq, payload }) = pending.pop() {
        emit(&mut out, &mut errors, &cancel, fail_fast, seq, payload)?;
    }

    errors.sort_by_key(|(line, _)| *line);
    out.flush()?;
    Ok(ReorderOutcome { errors, processed })
}

fn emit<W: Write>(
    out: &mut W,
    errors: &mut Vec<(u64, LineError)>,
    cancel: &Arc<AtomicBool>,
    fail_fast: bool,
    seq: u64,
    payload: Result<Vec<u8>, LineError>,
) -> std::io::Result<()> {
    match payload {
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
    Ok(())
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
