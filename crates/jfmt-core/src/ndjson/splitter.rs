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
        assert_eq!(items[1], (3, b"2".to_vec()));
        assert_eq!(items[2], (5, b"3".to_vec()));
    }

    #[test]
    fn stops_early_on_cancel() {
        let (tx, rx) = crossbeam_channel::bounded(1);
        let cancel = Arc::new(AtomicBool::new(true));
        let input = b"1\n2\n3\n".to_vec();
        let count = split_lines(input.as_slice(), tx, cancel).unwrap();
        assert_eq!(count, 0);
        drop(rx);
    }

    #[test]
    fn stops_when_receiver_drops() {
        let (tx, rx) = crossbeam_channel::bounded(1);
        let input = b"1\n2\n3\n".to_vec();
        let cancel = Arc::new(AtomicBool::new(false));
        let handle =
            std::thread::spawn(move || split_lines(input.as_slice(), tx, cancel).unwrap());
        let _ = rx.recv().unwrap();
        drop(rx);
        let count = handle.join().unwrap();
        assert!(count >= 1);
    }
}
