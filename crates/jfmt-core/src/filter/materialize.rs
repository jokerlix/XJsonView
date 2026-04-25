//! `jfmt filter --materialize`: load whole document, run jaq once,
//! emit a JSON-value stream.
//!
//! Output framing:
//! - `Compact`:  values separated by `\n`, no trailing newline.
//! - `Pretty(c)`: values separated by `\n\n`, no trailing newline.

use std::io::{Read, Write};

use serde_json::Value;

use super::runtime::run_one;
use super::{Compiled, FilterError, FilterOptions, FilterOutput};
use crate::event::{Event, Scalar};
use crate::writer::{EventWriter, MinifyWriter, PrettyWriter};

/// Outcome of a materialize run.
#[derive(Debug, Default)]
pub struct MaterializeReport {
    /// Number of jq output values emitted.
    pub outputs_emitted: u64,
    /// If `opts.strict` is false, write-side errors are collected
    /// here. The jaq run itself produces a single Result; errors there
    /// propagate as `Err` regardless of `strict`.
    pub runtime_errors: Vec<FilterError>,
}

/// Drive a materialize run: read everything from `reader` into a
/// `serde_json::Value`, run `compiled` against it, and write the
/// 0/1/N output values as a JSON-value stream to `writer`.
pub fn run_materialize<R: Read, W: Write>(
    reader: R,
    writer: W,
    compiled: &Compiled,
    output: FilterOutput,
    opts: FilterOptions,
) -> Result<MaterializeReport, FilterError> {
    // (1) Load the whole document into a Value.
    let input: Value = serde_json::from_reader(reader).map_err(|e| FilterError::Runtime {
        where_: String::from("(load)"),
        msg: format!("parse: {e}"),
    })?;

    // (2) Run jaq once.
    let outputs = match run_one(compiled, input) {
        Ok(o) => o,
        Err(mut e) => {
            if let FilterError::Runtime { where_: w, .. } = &mut e {
                *w = String::from("(materialize)");
            }
            return Err(e);
        }
    };

    let mut report = MaterializeReport {
        outputs_emitted: outputs.len() as u64,
        runtime_errors: Vec::new(),
    };

    // (3) Write the value stream.
    write_value_stream(writer, &outputs, output, &opts, &mut report)?;
    Ok(report)
}

fn write_value_stream<W: Write>(
    mut writer: W,
    values: &[Value],
    output: FilterOutput,
    opts: &FilterOptions,
    report: &mut MaterializeReport,
) -> Result<(), FilterError> {
    let separator: &[u8] = match &output {
        FilterOutput::Compact => b"\n",
        FilterOutput::Pretty(_) => b"\n\n",
    };

    for (i, v) in values.iter().enumerate() {
        if i > 0 {
            writer.write_all(separator)?;
        }
        match render_one_value(v, &output) {
            Ok(bytes) => writer.write_all(&bytes)?,
            Err(e) => {
                if opts.strict {
                    return Err(e);
                } else {
                    report.runtime_errors.push(e);
                }
            }
        }
    }
    Ok(())
}

/// Render a Value into bytes using the chosen formatter, with any
/// trailing newline stripped so the caller can frame the value stream
/// without doubling separators.
fn render_one_value(v: &Value, output: &FilterOutput) -> Result<Vec<u8>, FilterError> {
    let mut buf: Vec<u8> = Vec::new();
    match output {
        FilterOutput::Compact => {
            let mut w = MinifyWriter::new(&mut buf);
            emit_value_events(&mut w, v)?;
            w.finish()?;
        }
        FilterOutput::Pretty(cfg) => {
            let mut w = PrettyWriter::with_config(&mut buf, *cfg);
            emit_value_events(&mut w, v)?;
            w.finish()?;
        }
    }
    while buf.last().copied() == Some(b'\n') {
        buf.pop();
    }
    Ok(buf)
}

/// Emit a Value as a sequence of Events into an EventWriter.
fn emit_value_events<W: EventWriter>(writer: &mut W, v: &Value) -> Result<(), FilterError> {
    match v {
        Value::Null => writer.write_event(&Event::Value(Scalar::Null))?,
        Value::Bool(b) => writer.write_event(&Event::Value(Scalar::Bool(*b)))?,
        Value::Number(n) => writer.write_event(&Event::Value(Scalar::Number(n.to_string())))?,
        Value::String(s) => writer.write_event(&Event::Value(Scalar::String(s.clone())))?,
        Value::Array(items) => {
            writer.write_event(&Event::StartArray)?;
            for it in items {
                emit_value_events(writer, it)?;
            }
            writer.write_event(&Event::EndArray)?;
        }
        Value::Object(map) => {
            writer.write_event(&Event::StartObject)?;
            for (k, v) in map {
                writer.write_event(&Event::Name(k.clone()))?;
                emit_value_events(writer, v)?;
            }
            writer.write_event(&Event::EndObject)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filter::compile;
    use crate::filter::static_check::Mode;

    fn run(expr: &str, input: &str) -> (String, MaterializeReport) {
        let compiled = compile(expr, Mode::Materialize).unwrap();
        let mut out = Vec::<u8>::new();
        let report = run_materialize(
            input.as_bytes(),
            &mut out,
            &compiled,
            FilterOutput::Compact,
            FilterOptions::default(),
        )
        .expect("run_materialize");
        (String::from_utf8(out).unwrap(), report)
    }

    #[test]
    fn length_returns_array_size() {
        let (out, _) = run("length", "[1,2,3]");
        assert_eq!(out, "3");
    }

    #[test]
    fn identity_passes_value() {
        let (out, _) = run(".", "{\"x\":1}");
        // serde_json may sort keys; assert containment.
        assert!(out.contains(r#""x":1"#));
    }

    #[test]
    fn iterate_emits_value_stream() {
        let (out, report) = run(".[]", "[1,2,3]");
        assert_eq!(out, "1\n2\n3");
        assert_eq!(report.outputs_emitted, 3);
    }

    #[test]
    fn empty_output_writes_nothing() {
        let (out, report) = run(".[] | select(. > 100)", "[1,2,3]");
        assert_eq!(out, "");
        assert_eq!(report.outputs_emitted, 0);
    }

    #[test]
    fn single_value_no_separator() {
        let (out, report) = run(".x", r#"{"x":42}"#);
        assert_eq!(out, "42");
        assert_eq!(report.outputs_emitted, 1);
    }

    #[test]
    fn sort_by_works() {
        let (out, _) = run(
            "sort_by(.x) | .[].x",
            r#"[{"x":3},{"x":1},{"x":2}]"#,
        );
        assert_eq!(out, "1\n2\n3");
    }
}
