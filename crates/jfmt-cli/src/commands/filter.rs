use crate::cli::FilterArgs;
use crate::exit::ExitCode;
use crate::SilentExit;
use anyhow::{Context, Result};
use jfmt_core::filter::{
    compile, run_materialize, run_ndjson, run_streaming, FilterError, FilterOptions, FilterOutput,
};
use jfmt_core::PrettyConfig;
use std::sync::atomic::{AtomicBool, Ordering};

/// Whether the streaming-mode hint has been printed in this process.
static HINT_PRINTED: AtomicBool = AtomicBool::new(false);

pub fn run(args: FilterArgs, threads: usize) -> Result<()> {
    use jfmt_core::filter::Mode;

    if args.pretty && args.common.ndjson {
        return Err(anyhow::anyhow!("--pretty conflicts with --ndjson"));
    }

    // Mode pick: --materialize chooses Materialize, otherwise Streaming.
    // --ndjson still compiles in Streaming (each line IS a full document
    // for jaq's purposes).
    let mode = if args.materialize {
        Mode::Materialize
    } else {
        Mode::Streaming
    };
    let compiled = compile(&args.expr, mode).map_err(classify_compile_err)?;

    let opts = FilterOptions {
        strict: args.strict,
    };
    let input_spec = args.common.input_spec();

    if args.materialize {
        // RAM budget pre-flight (file inputs only; stdin returns None).
        if !args.force {
            if let Some(estimate) = estimate_peak_ram_bytes(&input_spec) {
                let total = system_total_ram_bytes();
                if !budget_ok(estimate, total) {
                    eprintln!(
                        "jfmt: estimated peak memory {} bytes exceeds 80% of total RAM ({} bytes); rerun with --force to override",
                        estimate, total
                    );
                    return Err(SilentExit(ExitCode::InputError).into());
                }
            }
        }

        let input = jfmt_io::open_input(&input_spec).context("opening input")?;
        let output = jfmt_io::open_output(&args.common.output_spec()).context("opening output")?;
        let out_choice = if args.pretty {
            let cfg = PrettyConfig {
                indent: args.indent,
                ..PrettyConfig::default()
            };
            FilterOutput::Pretty(cfg)
        } else {
            FilterOutput::Compact
        };
        match run_materialize(input, output, &compiled, out_choice, opts) {
            Ok(_report) => Ok(()),
            Err(e) => Err(classify_runtime_err(e, args.strict)),
        }
    } else if args.common.ndjson {
        let input = jfmt_io::open_input(&input_spec).context("opening input")?;
        let output = jfmt_io::open_output(&args.common.output_spec()).context("opening output")?;
        let report = run_ndjson(input, output, compiled, threads, opts)
            .context("filter NDJSON pipeline")?;
        for (line, e) in &report.errors {
            eprintln!("error: line {line}: {}", e.message);
        }
        if args.strict && !report.errors.is_empty() {
            return Err(SilentExit(ExitCode::InputError).into());
        }
        Ok(())
    } else {
        if !HINT_PRINTED.swap(true, Ordering::Relaxed) {
            eprintln!("note: streaming mode evaluates your expression once per top-level element.");
            eprintln!(
                "      write '.id' not '.[].id'  (use --ndjson for full per-line jq semantics)"
            );
        }
        let input = jfmt_io::open_input(&input_spec).context("opening input")?;
        let output = jfmt_io::open_output(&args.common.output_spec()).context("opening output")?;
        let out_choice = if args.pretty {
            let cfg = PrettyConfig {
                indent: args.indent,
                ..PrettyConfig::default()
            };
            FilterOutput::Pretty(cfg)
        } else {
            FilterOutput::Compact
        };
        let report = run_streaming(input, output, &compiled, out_choice, opts)
            .map_err(|e| classify_runtime_err(e, args.strict))?;
        for e in &report.runtime_errors {
            eprintln!("error: {e}");
        }
        if args.strict && !report.runtime_errors.is_empty() {
            return Err(SilentExit(ExitCode::InputError).into());
        }
        Ok(())
    }
}

fn classify_compile_err(e: FilterError) -> anyhow::Error {
    eprintln!("jfmt: {e}");
    SilentExit(match &e {
        FilterError::Aggregate { .. }
        | FilterError::MultiInput { .. }
        | FilterError::Parse { .. } => ExitCode::SyntaxError,
        _ => ExitCode::InputError,
    })
    .into()
}

fn classify_runtime_err(e: FilterError, strict: bool) -> anyhow::Error {
    eprintln!("jfmt: {e}");
    SilentExit(if strict {
        ExitCode::InputError
    } else {
        ExitCode::Success
    })
    .into()
}

/// Estimate peak RAM for materializing `input`. Returns `None` when
/// the input is stdin or its size can't be determined — callers
/// interpret `None` as "skip the check" per spec D3.
fn estimate_peak_ram_bytes(spec: &jfmt_io::InputSpec) -> Option<u64> {
    let path = spec.path.as_ref()?;
    let meta = std::fs::metadata(path).ok()?;
    let on_disk = meta.len();
    // Effective compression: explicit override, then file extension.
    let compression = spec
        .compression
        .unwrap_or_else(|| jfmt_io::Compression::from_path(path));
    let multiplier: u64 = match compression {
        jfmt_io::Compression::None => 6,
        jfmt_io::Compression::Gzip | jfmt_io::Compression::Zstd => 5 * 6, // 30
    };
    Some(on_disk.saturating_mul(multiplier))
}

/// Pure predicate: is `estimate` under 80% of `total_ram`?
fn budget_ok(estimate: u64, total_ram: u64) -> bool {
    // 80% = total_ram * 4 / 5. Compute as `total_ram / 5 * 4` to
    // reduce overflow risk on very large `total_ram` values.
    estimate < total_ram / 5 * 4
}

/// Query the actual system total RAM. Wraps sysinfo per spec Annex B.
fn system_total_ram_bytes() -> u64 {
    let mut sys = sysinfo::System::new();
    sys.refresh_memory();
    sys.total_memory()
}

#[cfg(test)]
mod tests {
    use super::budget_ok;

    #[test]
    fn budget_ok_under_80_percent() {
        // 1 GiB on a 2 GiB machine = 50% < 80% → ok.
        assert!(budget_ok(1 << 30, 2u64 << 30));
    }

    #[test]
    fn budget_not_ok_over_80_percent() {
        // 1.7 GiB on a 2 GiB machine ≈ 85% → not ok.
        let total = 2u64 << 30;
        let estimate = total * 85 / 100;
        assert!(!budget_ok(estimate, total));
    }

    #[test]
    fn budget_ok_at_zero_estimate() {
        assert!(budget_ok(0, 1 << 30));
    }
}
