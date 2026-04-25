//! NDJSON pipeline + schema; verify --threads parity in counts.

use jfmt_core::validate::SchemaValidator;
use jfmt_core::{run_ndjson_pipeline, LineError, NdjsonPipelineOptions, StatsCollector};
use serde_json::json;
use std::io::Cursor;
use std::sync::Arc;

fn schema() -> serde_json::Value {
    json!({
        "type": "object",
        "required": ["x"],
        "properties": {"x": {"type": "integer"}}
    })
}

fn run_with_threads(threads: usize, input: &[u8]) -> (u64, u64) {
    let s = Arc::new(SchemaValidator::compile(&schema()).unwrap());
    let s_clone = Arc::clone(&s);
    let closure = move |line: &[u8], c: &mut StatsCollector| -> Result<Vec<Vec<u8>>, LineError> {
        c.begin_record();
        let value: serde_json::Value = match serde_json::from_slice(line) {
            Ok(v) => v,
            Err(e) => {
                c.end_record(false);
                return Err(LineError {
                    line: 0,
                    offset: 0,
                    column: None,
                    message: format!("{e}"),
                });
            }
        };
        c.end_record(true);
        let violations = s_clone.validate(&value);
        let paths: Vec<&str> = violations.iter().map(|v| v.instance_path.as_str()).collect();
        c.record_schema_outcome(violations.is_empty(), &paths);
        Ok(vec![Vec::new()])
    };
    let opts = NdjsonPipelineOptions {
        threads,
        collect_stats: true,
        ..Default::default()
    };
    let report = run_ndjson_pipeline(Cursor::new(input.to_vec()), std::io::sink(), closure, opts)
        .unwrap();
    let stats = report.stats.unwrap();
    (stats.schema_pass, stats.schema_fail)
}

#[test]
fn ndjson_counts_pass_fail() {
    let input = b"{\"x\":1}\n{\"y\":2}\n{\"x\":\"a\"}\n{\"x\":3}\n";
    let (pass, fail) = run_with_threads(1, input);
    assert_eq!(pass, 2);
    assert_eq!(fail, 2);
}

#[test]
fn ndjson_threads_parity_in_counts() {
    let mut input = Vec::new();
    for i in 0..200 {
        if i % 5 == 0 {
            input.extend_from_slice(format!("{{\"y\":{i}}}\n").as_bytes());
        } else {
            input.extend_from_slice(format!("{{\"x\":{i}}}\n").as_bytes());
        }
    }
    let (pass1, fail1) = run_with_threads(1, &input);
    let (pass4, fail4) = run_with_threads(4, &input);
    assert_eq!(pass1, pass4);
    assert_eq!(fail1, fail4);
}
