//! Per-worker loop. Owns one StatsCollector for the lifetime of the
//! thread; merged into the final report after join.

use crate::ndjson::splitter::LineItem;
use crate::ndjson::LineError;
use crate::validate::StatsCollector;
use crossbeam_channel::{Receiver, Sender};
use std::panic::AssertUnwindSafe;

/// One unit of completed work sent to the reorder stage.
pub type WorkerOutput = (u64, Result<Vec<Vec<u8>>, LineError>);

/// Run one worker loop. Returns the collector so the caller can merge it.
pub fn run_worker<F>(
    rx: Receiver<LineItem>,
    tx: Sender<WorkerOutput>,
    f: std::sync::Arc<F>,
) -> StatsCollector
where
    F: Fn(&[u8], &mut StatsCollector) -> Result<Vec<Vec<u8>>, LineError> + Send + Sync + 'static,
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
            Ok::<_, LineError>(vec![bytes.to_ascii_uppercase()])
        });
        run_worker(in_rx, out_tx, f);
        let collected: Vec<_> = out_rx.iter().collect();
        assert_eq!(
            collected,
            vec![(1, Ok(vec![b"HI".to_vec()])), (2, Ok(vec![b"!!".to_vec()])),]
        );
    }

    #[test]
    fn catches_panics_as_line_errors() {
        let (in_tx, in_rx) = unbounded::<LineItem>();
        let (out_tx, out_rx) = unbounded::<WorkerOutput>();
        in_tx.send((42, b"trigger".to_vec())).unwrap();
        drop(in_tx);

        let f = Arc::new(
            |_b: &[u8], _c: &mut StatsCollector| -> Result<Vec<Vec<u8>>, LineError> {
                panic!("boom")
            },
        );
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

        let f = Arc::new(
            |_b: &[u8], _c: &mut StatsCollector| -> Result<Vec<Vec<u8>>, LineError> {
                Err(LineError {
                    line: 7,
                    offset: 0,
                    column: None,
                    message: "nope".into(),
                })
            },
        );
        run_worker(in_rx, out_tx, f);
        let items: Vec<_> = out_rx.iter().collect();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].0, 7);
        assert!(items[0].1.is_err());
    }
}
