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
fn parallel_output_matches_serial_byte_for_byte() {
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
