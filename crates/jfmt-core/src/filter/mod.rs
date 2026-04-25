//! `jfmt filter` engine: per-shard jq evaluation in two flavours
//! (single-document streaming, NDJSON parallel). Out of scope for
//! M4a: `--materialize` mode (lands in M4b).

pub mod compile;
pub mod output;
pub mod runtime;
pub mod shard;
pub mod static_check;

use thiserror::Error;

/// Top-level filter error. Library variants; the CLI maps them to
/// exit codes via `crates/jfmt-cli/src/exit.rs`.
#[derive(Debug, Error)]
pub enum FilterError {
    /// jaq parser rejected the expression.
    #[error("invalid filter expression: {msg}")]
    Parse { msg: String },

    /// Static check blacklisted the expression because it cannot be
    /// evaluated per-shard. Carry the offending name so the CLI can
    /// suggest `--ndjson` / `--materialize`.
    #[error(
        "filter expression uses '{name}' which requires whole-document evaluation; \
             consider `--ndjson` (per-line full semantics) or `--materialize` (M4b)"
    )]
    Aggregate { name: String },

    /// jaq runtime error on one shard / line. `where_` carries the
    /// shard's line number (NDJSON) or array index / object key
    /// (single-document) for stderr reporting.
    #[error("filter runtime error at {where_}: {msg}")]
    Runtime { where_: String, msg: String },

    /// Object or scalar shard produced more than one output. We can't
    /// re-encode that in shape-preserving mode.
    #[error(
        "filter at {where_} produced multiple outputs for {kind}; \
             use --ndjson or --materialize to allow this"
    )]
    OutputShape { where_: String, kind: &'static str },

    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    /// Underlying parser/writer error from `jfmt_core`.
    #[error(transparent)]
    Core(#[from] crate::Error),
}

/// Options shared by both flavours.
#[derive(Debug, Clone, Default)]
pub struct FilterOptions {
    /// If `true`, runtime errors abort the run (mapped to non-zero
    /// exit code by the CLI). Otherwise they are reported to stderr
    /// and skipped.
    pub strict: bool,
}

pub use compile::{compile, Compiled};

use crate::event::Event;
use crate::parser::EventReader;
use crate::writer::{EventWriter, MinifyWriter, PrettyConfig, PrettyWriter};
use std::io::{Read, Write};

pub use shard::{Shard, ShardAccumulator, ShardLocator, TopLevel};

/// Output formatting choice for filter results.
#[derive(Debug, Clone, Default)]
pub enum FilterOutput {
    #[default]
    Compact,
    Pretty(PrettyConfig),
}

/// Outcome of a streaming filter run.
#[derive(Debug, Default)]
pub struct StreamingReport {
    pub shards_seen: u64,
    pub runtime_errors: Vec<FilterError>,
}

/// Drive a single-document streaming filter from `reader` to `writer`.
/// Runtime errors are collected; if `opts.strict` is set, the first
/// error is returned immediately.
pub fn run_streaming<R: Read, W: Write>(
    reader: R,
    writer: W,
    compiled: &Compiled,
    output: FilterOutput,
    opts: FilterOptions,
) -> Result<StreamingReport, FilterError> {
    match output {
        FilterOutput::Compact => {
            let w = MinifyWriter::new(writer);
            run_streaming_inner(reader, w, compiled, opts)
        }
        FilterOutput::Pretty(cfg) => {
            let w = PrettyWriter::with_config(writer, cfg);
            run_streaming_inner(reader, w, compiled, opts)
        }
    }
}

fn run_streaming_inner<R: Read, W: EventWriter>(
    reader: R,
    writer: W,
    compiled: &Compiled,
    opts: FilterOptions,
) -> Result<StreamingReport, FilterError> {
    use crate::filter::output::OutputShaper;
    use crate::filter::runtime::run_one;

    let mut reader = EventReader::new(reader);
    let mut acc = ShardAccumulator::new();
    let mut shaper = OutputShaper::new(writer);
    let mut report = StreamingReport::default();
    let mut began = false;

    while let Some(ev) = reader.next_event()? {
        if !began {
            let top = match &ev {
                Event::StartArray => TopLevel::Array,
                Event::StartObject => TopLevel::Object,
                _ => TopLevel::Scalar,
            };
            shaper.begin(top)?;
            began = true;
        }

        if let Some(shard) = acc.push(ev).map_err(|e| FilterError::Runtime {
            where_: String::new(),
            msg: format!("shard accumulator: {e}"),
        })? {
            report.shards_seen += 1;
            let where_ = match &shard.locator {
                ShardLocator::Index(i) => format!("[{i}]"),
                ShardLocator::Key(k) => format!(".{k}"),
                ShardLocator::Root => String::from("(root)"),
            };
            let key_for_shaper = match &shard.locator {
                ShardLocator::Key(k) => Some(k.clone()),
                _ => None,
            };
            match run_one(compiled, shard.value) {
                Ok(outputs) => {
                    if let Err(e) = shaper.emit(outputs, key_for_shaper.as_deref(), &where_) {
                        if opts.strict {
                            return Err(e);
                        } else {
                            report.runtime_errors.push(e);
                        }
                    }
                }
                Err(mut e) => {
                    if let FilterError::Runtime { where_: w, .. } = &mut e {
                        *w = where_.clone();
                    }
                    if opts.strict {
                        return Err(e);
                    } else {
                        report.runtime_errors.push(e);
                    }
                }
            }
        }
    }

    if began {
        shaper.finish()?;
    }
    Ok(report)
}
