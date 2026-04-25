use crate::cli::ValidateArgs;
use crate::exit::ExitCode;
use crate::SilentExit;
use anyhow::{Context, Result};
use jfmt_core::filter::shard::{ShardAccumulator, ShardLocator, TopLevel};
use jfmt_core::parser::EventReader;
use jfmt_core::validate::SchemaValidator;
use jfmt_core::{
    run_ndjson_pipeline, validate_syntax, Error, LineError, NdjsonPipelineOptions, Stats,
    StatsCollector,
};
use std::fs::File;
use std::io::{BufWriter, Write};
use std::sync::Arc;

pub fn run(args: ValidateArgs, threads: usize) -> Result<()> {
    let collect_stats = args.stats || args.stats_json.is_some();

    // Compile the schema, if any. Compile errors -> exit 1.
    let schema = if let Some(path) = &args.schema {
        Some(Arc::new(load_schema(path)?))
    } else {
        None
    };

    let input_spec = args.common.input_spec();

    // Materialize branch (with optional schema). RAM pre-flight if file input.
    if args.materialize {
        if !args.force {
            if let Some(estimate) = super::ram_budget::estimate_peak_ram_bytes(&input_spec) {
                let total = super::ram_budget::system_total_ram_bytes();
                if !super::ram_budget::budget_ok(estimate, total) {
                    eprintln!(
                        "jfmt: estimated peak memory {} bytes exceeds 80% of total RAM ({} bytes); rerun with --force to override",
                        estimate, total
                    );
                    return Err(SilentExit(ExitCode::InputError).into());
                }
            }
        }
        let input = jfmt_io::open_input(&input_spec).context("opening input")?;
        return run_materialize(input, schema, args, collect_stats);
    }

    let input = jfmt_io::open_input(&input_spec).context("opening input")?;
    if args.common.ndjson {
        run_ndjson(input, schema, args, threads, collect_stats)
    } else {
        run_streaming(input, schema, args, collect_stats)
    }
}

fn load_schema(path: &std::path::Path) -> Result<SchemaValidator> {
    use jfmt_core::validate::SchemaError;
    let bytes = std::fs::read(path).map_err(|e| SchemaError::BadSchemaFile {
        path: path.to_path_buf(),
        source: e,
    })?;
    let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(SchemaError::from)?;
    SchemaValidator::compile(&value).map_err(anyhow::Error::from)
}

fn run_ndjson<R: std::io::Read + Send + 'static>(
    input: R,
    schema: Option<Arc<SchemaValidator>>,
    args: ValidateArgs,
    threads: usize,
    collect_stats: bool,
) -> Result<()> {
    // If a schema is in play, force stats collection so schema_fail
    // counts feed into the final exit code under --strict.
    let need_stats = collect_stats || schema.is_some();
    let opts = NdjsonPipelineOptions {
        threads,
        fail_fast: args.fail_fast,
        collect_stats: need_stats,
        ..Default::default()
    };
    let schema_for_closure = schema; // moved into closure
    let closure = move |line: &[u8], c: &mut StatsCollector| -> Result<Vec<Vec<u8>>, LineError> {
        c.begin_record();
        // (1) Syntax pass via EventReader (M2 behaviour).
        let mut p = EventReader::new(line);
        loop {
            match p.next_event() {
                Ok(None) => break,
                Ok(Some(ev)) => c.observe(&ev),
                Err(Error::Syntax {
                    offset,
                    column,
                    message,
                    ..
                }) => {
                    c.end_record(false);
                    return Err(LineError {
                        line: 0,
                        offset,
                        column,
                        message,
                    });
                }
                Err(e) => {
                    c.end_record(false);
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
            c.end_record(false);
            return Err(LineError {
                line: 0,
                offset,
                column,
                message,
            });
        }
        c.end_record(true);
        // (2) Schema pass (only if schema present). Re-parse line as
        //     serde_json::Value (the EventReader pass above rejected
        //     malformed input, so we know it parses now).
        if let Some(s) = schema_for_closure.as_ref() {
            let value: serde_json::Value = serde_json::from_slice(line).map_err(|e| LineError {
                line: 0,
                offset: 0,
                column: None,
                message: format!("post-syntax JSON re-parse failed: {e}"),
            })?;
            let violations = s.validate(&value);
            let paths: Vec<&str> =
                violations.iter().map(|v| v.instance_path.as_str()).collect();
            c.record_schema_outcome(violations.is_empty(), &paths);
            if !violations.is_empty() {
                // Encode violations as the "ok bytes" payload — the
                // reorder buffer flushes them in input order. Each
                // violation gets its own line via reorder's `\n`
                // appender.
                let mut parts = Vec::with_capacity(violations.len());
                for v in &violations {
                    parts.push(
                        format!(
                            "schema: {}: {}: {}",
                            v.instance_path, v.keyword, v.message
                        )
                        .into_bytes(),
                    );
                }
                return Ok(parts);
            }
        }
        Ok(vec![Vec::new()])
    };

    // Custom Write that prefixes each line with `line N: ` based on
    // a counter incremented per `\n`. This sits in place of the
    // /dev/null sink M2 used; reorder buffer guarantees in-order writes.
    let stderr_writer = StderrLineCounter::new();
    let report = run_ndjson_pipeline(input, stderr_writer, closure, opts)
        .context("reading input")?;

    for (seq, le) in &report.errors {
        let col = le.column.map(|c| format!("col {c} ")).unwrap_or_default();
        eprintln!(
            "line {seq}: {}syntax error at byte {}: {}",
            col, le.offset, le.message
        );
    }
    let any_syntax_bad = !report.errors.is_empty();
    let any_schema_bad = report
        .stats
        .as_ref()
        .map(|s| s.schema_fail > 0)
        .unwrap_or(false);

    finalise(report.stats, &args, any_syntax_bad, any_schema_bad)
}

/// A `Write` impl that re-prefixes each line with `line N: ` based on
/// a counter incremented per `\n`. The reorder buffer writes each
/// per-seq payload followed by `\n`, so we just split on `\n` and
/// prefix each non-empty chunk before emitting to stderr.
///
/// Empty chunks (the closure's `vec![Vec::new()]` "all clean" payload)
/// still come through as a `\n`-only write and just bump the counter.
struct StderrLineCounter {
    line: u64,
    pending: Vec<u8>,
}
impl StderrLineCounter {
    fn new() -> Self {
        Self {
            line: 0,
            pending: Vec::new(),
        }
    }
}
impl std::io::Write for StderrLineCounter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let mut start = 0;
        for (i, b) in buf.iter().enumerate() {
            if *b == b'\n' {
                self.line += 1;
                let chunk = &buf[start..i];
                if self.pending.is_empty() && chunk.is_empty() {
                    // Clean line — just bump counter.
                } else {
                    self.pending.extend_from_slice(chunk);
                    eprintln!(
                        "line {}: {}",
                        self.line,
                        String::from_utf8_lossy(&self.pending)
                    );
                    self.pending.clear();
                }
                start = i + 1;
            }
        }
        // Trailing partial chunk (no `\n`): stash for the next call.
        if start < buf.len() {
            self.pending.extend_from_slice(&buf[start..]);
        }
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn run_streaming<R: std::io::Read>(
    input: R,
    schema: Option<Arc<SchemaValidator>>,
    args: ValidateArgs,
    collect_stats: bool,
) -> Result<()> {
    if collect_stats || schema.is_some() {
        let mut c = StatsCollector::default();
        c.begin_record();
        let mut r = EventReader::new(input);
        let mut acc = ShardAccumulator::new();
        let mut top: Option<TopLevel> = None;

        let parse_result: Result<(), jfmt_core::Error> = loop {
            match r.next_event() {
                Ok(None) => break Ok(()),
                Ok(Some(ev)) => {
                    if top.is_none() {
                        top = Some(match &ev {
                            jfmt_core::Event::StartArray => TopLevel::Array,
                            jfmt_core::Event::StartObject => TopLevel::Object,
                            _ => TopLevel::Scalar,
                        });
                        // Reject schema + non-array root in streaming mode.
                        if schema.is_some() && !matches!(top, Some(TopLevel::Array)) {
                            eprintln!(
                                "jfmt: schema validation of non-array root requires --materialize or --ndjson"
                            );
                            return Err(SilentExit(ExitCode::InputError).into());
                        }
                    }
                    c.observe(&ev);
                    if let Some(s) = schema.as_ref() {
                        match acc.push(ev.clone()) {
                            Ok(Some(shard)) => {
                                let violations = s.validate(&shard.value);
                                let where_ = match &shard.locator {
                                    ShardLocator::Index(i) => format!("element {i}"),
                                    _ => String::from("?"),
                                };
                                let paths: Vec<&str> = violations
                                    .iter()
                                    .map(|v| v.instance_path.as_str())
                                    .collect();
                                c.record_schema_outcome(violations.is_empty(), &paths);
                                for v in &violations {
                                    eprintln!(
                                        "{where_}: {}: {}: {}",
                                        v.instance_path, v.keyword, v.message
                                    );
                                    if args.fail_fast {
                                        return finalise(
                                            Some(c.finish()),
                                            &args,
                                            false,
                                            true,
                                        );
                                    }
                                }
                            }
                            Ok(None) => {}
                            Err(e) => {
                                eprintln!("jfmt: shard accumulator: {e}");
                                return Err(SilentExit(ExitCode::InputError).into());
                            }
                        }
                    }
                }
                Err(e) => break Err(e),
            }
        };
        let finish_result = parse_result.and_then(|_| r.finish());
        match finish_result {
            Ok(()) => {
                c.end_record(true);
                let stats = Some(c.finish());
                let any_schema_bad = stats
                    .as_ref()
                    .map(|s| s.schema_fail > 0)
                    .unwrap_or(false);
                finalise(stats, &args, false, any_schema_bad)
            }
            Err(e) => {
                c.end_record(false);
                let _ = c.finish();
                Err(anyhow::Error::from(e).context("validation failed"))
            }
        }
    } else {
        validate_syntax(input).context("validation failed")?;
        finalise(None, &args, false, false)
    }
}

fn run_materialize<R: std::io::Read>(
    input: R,
    schema: Option<Arc<SchemaValidator>>,
    args: ValidateArgs,
    collect_stats: bool,
) -> Result<()> {
    let value: serde_json::Value =
        serde_json::from_reader(input).context("validation failed: parsing input")?;

    let mut stats = if collect_stats {
        Some(Stats {
            records: 1,
            valid: 1,
            ..Stats::default()
        })
    } else {
        None
    };

    let mut any_schema_bad = false;
    if let Some(s) = schema.as_ref() {
        let violations = s.validate(&value);
        any_schema_bad = !violations.is_empty();
        for v in &violations {
            eprintln!(
                "(root): {}: {}: {}",
                v.instance_path, v.keyword, v.message
            );
            if args.fail_fast {
                break;
            }
        }
        if let Some(st) = stats.as_mut() {
            if violations.is_empty() {
                st.schema_pass += 1;
            } else {
                st.schema_fail += 1;
                for vio in &violations {
                    *st.top_violation_paths
                        .entry(vio.instance_path.clone())
                        .or_insert(0) += 1;
                }
            }
        }
    }

    finalise(stats, &args, false, any_schema_bad)
}

fn finalise(
    stats: Option<Stats>,
    args: &ValidateArgs,
    any_syntax_bad: bool,
    any_schema_bad: bool,
) -> Result<()> {
    if let Some(s) = stats.as_ref() {
        if args.stats {
            eprint!("{s}");
        }
        if let Some(path) = args.stats_json.as_ref() {
            write_stats_json(path, s).context("writing --stats-json")?;
        }
    }

    if any_syntax_bad {
        return Err(anyhow::Error::from(SilentExit(ExitCode::SyntaxError)));
    }
    if any_schema_bad && args.strict {
        return Err(anyhow::Error::from(SilentExit(ExitCode::SchemaError)));
    }
    Ok(())
}

fn write_stats_json(path: &std::path::Path, stats: &Stats) -> std::io::Result<()> {
    let f = File::create(path)?;
    let mut w = BufWriter::new(f);
    serde_json::to_writer_pretty(&mut w, stats)?;
    w.write_all(b"\n")?;
    Ok(())
}
