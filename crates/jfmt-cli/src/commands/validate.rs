use crate::cli::ValidateArgs;
use crate::exit::ExitCode;
use crate::SilentExit;
use anyhow::{Context, Result};
use jfmt_core::parser::EventReader;
use jfmt_core::validate::{validate_ndjson, NdjsonOptions};
use jfmt_core::{validate_syntax, Stats, StatsCollector};
use std::fs::File;
use std::io::{BufWriter, Write};

pub fn run(args: ValidateArgs) -> Result<()> {
    let input = jfmt_io::open_input(&args.common.input_spec()).context("opening input")?;
    let collect_stats = args.stats || args.stats_json.is_some();

    let (any_bad, stats) = if args.common.ndjson {
        let report = validate_ndjson(
            input,
            NdjsonOptions {
                fail_fast: args.fail_fast,
                collect_stats,
            },
        )
        .context("reading input")?;

        for le in &report.errors {
            let col = le
                .column
                .map(|c| format!("col {c} "))
                .unwrap_or_default();
            eprintln!(
                "line {}: {}syntax error at byte {}: {}",
                le.line, col, le.offset, le.message
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
        // Per-line errors already went to stderr; just set the exit code.
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
