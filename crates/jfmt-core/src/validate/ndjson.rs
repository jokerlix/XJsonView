//! Serial NDJSON validator. Reports errors per line.

use crate::error::Error;
use crate::parser::EventReader;
use crate::validate::stats::StatsCollector;
use std::io::{BufRead, BufReader, Read};

/// One reported per-line error in NDJSON mode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineError {
    /// 1-indexed line number (blank lines still advance the counter).
    pub line: u64,
    /// Byte offset inside that line where struson failed.
    pub offset: u64,
    pub column: Option<u64>,
    pub message: String,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct NdjsonOptions {
    /// Stop at first bad line.
    pub fail_fast: bool,
    /// Collect per-record statistics.
    pub collect_stats: bool,
}

/// Result of validating an NDJSON stream.
pub struct NdjsonReport {
    pub errors: Vec<LineError>,
    pub stats: Option<crate::validate::Stats>,
}

/// Read `reader` line-by-line and validate each as its own JSON value.
/// Empty / whitespace-only lines are skipped (no error, no stats contribution).
pub fn validate_ndjson<R: Read>(reader: R, opts: NdjsonOptions) -> std::io::Result<NdjsonReport> {
    let br = BufReader::new(reader);
    let mut collector = opts.collect_stats.then(StatsCollector::default);
    let mut errors = Vec::new();
    for (idx, line) in br.lines().enumerate() {
        let line_no: u64 = idx as u64 + 1;
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }

        if let Some(c) = collector.as_mut() {
            c.begin_record();
        }

        let mut parser = EventReader::new(line.as_bytes());
        let mut ok = true;
        let mut fail: Option<LineError> = None;
        loop {
            match parser.next_event() {
                Ok(None) => break,
                Ok(Some(ev)) => {
                    if let Some(c) = collector.as_mut() {
                        c.observe(&ev);
                    }
                }
                Err(Error::Syntax {
                    offset,
                    column,
                    message,
                    ..
                }) => {
                    fail = Some(LineError {
                        line: line_no,
                        offset,
                        column,
                        message,
                    });
                    ok = false;
                    break;
                }
                Err(Error::Io(io)) => return Err(io),
                Err(Error::State(s)) => {
                    fail = Some(LineError {
                        line: line_no,
                        offset: 0,
                        column: None,
                        message: format!("invalid state: {s}"),
                    });
                    ok = false;
                    break;
                }
            }
        }

        // If the primary parse succeeded, verify no trailing garbage on the line.
        if ok {
            if let Err(Error::Syntax {
                offset,
                column,
                message,
                ..
            }) = parser.finish()
            {
                fail = Some(LineError {
                    line: line_no,
                    offset,
                    column,
                    message,
                });
                ok = false;
            }
        }

        if let Some(le) = fail {
            errors.push(le);
        }
        if let Some(c) = collector.as_mut() {
            c.end_record(ok);
        }
        if !ok && opts.fail_fast {
            break;
        }
    }

    Ok(NdjsonReport {
        errors,
        stats: collector.map(|c| c.finish()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_all_valid_lines() {
        let input = b"1\n\"hi\"\n{\"a\":1}\n";
        let r = validate_ndjson(input.as_slice(), NdjsonOptions::default()).unwrap();
        assert!(r.errors.is_empty(), "{:?}", r.errors);
    }

    #[test]
    fn reports_bad_line_keeps_going() {
        let input = b"1\n{bad}\n3\n";
        let r = validate_ndjson(input.as_slice(), NdjsonOptions::default()).unwrap();
        assert_eq!(r.errors.len(), 1);
        assert_eq!(r.errors[0].line, 2);
    }

    #[test]
    fn fail_fast_stops_on_first_error() {
        let input = b"{bad1}\n{bad2}\n1\n";
        let r = validate_ndjson(
            input.as_slice(),
            NdjsonOptions {
                fail_fast: true,
                collect_stats: false,
            },
        )
        .unwrap();
        assert_eq!(r.errors.len(), 1);
        assert_eq!(r.errors[0].line, 1);
    }

    #[test]
    fn skips_blank_lines() {
        let input = b"1\n\n\n2\n";
        let r = validate_ndjson(input.as_slice(), NdjsonOptions::default()).unwrap();
        assert!(r.errors.is_empty());
        let r = validate_ndjson(
            input.as_slice(),
            NdjsonOptions {
                fail_fast: false,
                collect_stats: true,
            },
        )
        .unwrap();
        assert_eq!(r.stats.as_ref().unwrap().records, 2);
    }

    #[test]
    fn stats_count_top_level_types_across_lines() {
        let input = b"1\n\"hi\"\n{\"a\":1}\n[1,2]\n";
        let r = validate_ndjson(
            input.as_slice(),
            NdjsonOptions {
                fail_fast: false,
                collect_stats: true,
            },
        )
        .unwrap();
        let s = r.stats.unwrap();
        assert_eq!(s.records, 4);
        assert_eq!(s.top_level_types.get("number"), Some(&1));
        assert_eq!(s.top_level_types.get("string"), Some(&1));
        assert_eq!(s.top_level_types.get("object"), Some(&1));
        assert_eq!(s.top_level_types.get("array"), Some(&1));
    }

    #[test]
    fn detects_trailing_garbage_on_line() {
        let input = b"{\"a\":1} junk\n";
        let r = validate_ndjson(input.as_slice(), NdjsonOptions::default()).unwrap();
        assert_eq!(r.errors.len(), 1);
        assert_eq!(r.errors[0].line, 1);
    }
}
