use crate::cli::MinifyArgs;
use anyhow::Context;
use jfmt_core::{
    run_ndjson_pipeline, transcode, LineError, MinifyWriter, NdjsonPipelineOptions,
    StatsCollector,
};

pub fn run(args: MinifyArgs, threads: usize) -> anyhow::Result<()> {
    if args.common.ndjson {
        let input = jfmt_io::open_input(&args.common.input_spec()).context("opening input")?;
        let output = jfmt_io::open_output(&args.common.output_spec()).context("opening output")?;
        let opts = NdjsonPipelineOptions {
            threads,
            fail_fast: true,
            collect_stats: false,
            ..Default::default()
        };
        let closure = |line: &[u8], _c: &mut StatsCollector| -> Result<Vec<u8>, LineError> {
            let mut out = Vec::with_capacity(line.len());
            let writer = MinifyWriter::new(&mut out);
            match transcode(line, writer) {
                Ok(()) => Ok(out),
                Err(e) => match e {
                    jfmt_core::Error::Syntax {
                        offset,
                        column,
                        message,
                        ..
                    } => Err(LineError {
                        line: 0,
                        offset,
                        column,
                        message,
                    }),
                    other => Err(LineError {
                        line: 0,
                        offset: 0,
                        column: None,
                        message: format!("{other}"),
                    }),
                },
            }
        };
        let report =
            run_ndjson_pipeline(input, output, closure, opts).context("minify failed")?;
        for (seq, le) in &report.errors {
            eprintln!(
                "line {seq}: syntax error at byte {}: {}",
                le.offset, le.message
            );
        }
        if !report.errors.is_empty() {
            return Err(anyhow::Error::from(crate::SilentExit(
                crate::exit::ExitCode::SyntaxError,
            )));
        }
        return Ok(());
    }

    let input = jfmt_io::open_input(&args.common.input_spec()).context("opening input")?;
    let output = jfmt_io::open_output(&args.common.output_spec()).context("opening output")?;
    let writer = MinifyWriter::new(output);
    transcode(input, writer).context("minifying failed")?;
    Ok(())
}
