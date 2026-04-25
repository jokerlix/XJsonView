use crate::cli::ValidateArgs;
use crate::exit::ExitCode;
use crate::SilentExit;
use anyhow::{Context, Result};
use jfmt_core::parser::EventReader;
use jfmt_core::{
    run_ndjson_pipeline, validate_syntax, Error, LineError, NdjsonPipelineOptions, Stats,
    StatsCollector,
};
use std::fs::File;
use std::io::{BufWriter, Write};

pub fn run(args: ValidateArgs, threads: usize) -> Result<()> {
    let input = jfmt_io::open_input(&args.common.input_spec()).context("opening input")?;
    let collect_stats = args.stats || args.stats_json.is_some();

    let (any_bad, stats) = if args.common.ndjson {
        let sink = std::io::sink();
        let opts = NdjsonPipelineOptions {
            threads,
            fail_fast: args.fail_fast,
            collect_stats,
            ..Default::default()
        };
        let closure = |line: &[u8], c: &mut StatsCollector| -> Result<Vec<Vec<u8>>, LineError> {
            c.begin_record();
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
            Ok(vec![Vec::new()])
        };
        let report = run_ndjson_pipeline(input, sink, closure, opts).context("reading input")?;

        for (seq, le) in &report.errors {
            let col = le.column.map(|c| format!("col {c} ")).unwrap_or_default();
            eprintln!(
                "line {seq}: {}syntax error at byte {}: {}",
                col, le.offset, le.message
            );
        }
        (!report.errors.is_empty(), report.stats)
    } else if collect_stats {
        let mut c = StatsCollector::default();
        c.begin_record();
        let mut r = EventReader::new(input);
        let parse_result = loop {
            match r.next_event() {
                Ok(None) => break Ok(()),
                Ok(Some(ev)) => c.observe(&ev),
                Err(e) => break Err(e),
            }
        };
        let finish_result = parse_result.and_then(|_| r.finish());
        match finish_result {
            Ok(()) => {
                c.end_record(true);
                (false, Some(c.finish()))
            }
            Err(e) => {
                c.end_record(false);
                let _ = c.finish();
                return Err(anyhow::Error::from(e).context("validation failed"));
            }
        }
    } else {
        validate_syntax(input).context("validation failed")?;
        (false, None)
    };

    if let Some(s) = stats.as_ref() {
        if args.stats {
            eprint!("{s}");
        }
        if let Some(path) = args.stats_json.as_ref() {
            write_stats_json(path, s).context("writing --stats-json")?;
        }
    }

    if any_bad {
        return Err(anyhow::Error::from(SilentExit(ExitCode::SyntaxError)));
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
