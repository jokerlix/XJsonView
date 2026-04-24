//! Integration tests for run_ndjson_pipeline. Supersedes the M2
//! serial NDJSON unit tests.

use jfmt_core::parser::EventReader;
use jfmt_core::{
    run_ndjson_pipeline, Error, LineError, NdjsonPipelineOptions, StatsCollector,
};
use std::io::{Cursor, Write};
use std::sync::{Arc, Mutex};

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

fn validate_closure(line: &[u8], collector: &mut StatsCollector) -> Result<Vec<u8>, LineError> {
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
                    line: 0,
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
        buf,
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
