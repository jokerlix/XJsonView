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
    #[error("filter expression uses '{name}' which requires whole-document evaluation; \
             consider `--ndjson` (per-line full semantics) or `--materialize` (M4b)")]
    Aggregate { name: String },

    /// jaq runtime error on one shard / line. `where_` carries the
    /// shard's line number (NDJSON) or array index / object key
    /// (single-document) for stderr reporting.
    #[error("filter runtime error at {where_}: {msg}")]
    Runtime { where_: String, msg: String },

    /// Object or scalar shard produced more than one output. We can't
    /// re-encode that in shape-preserving mode.
    #[error("filter at {where_} produced multiple outputs for {kind}; \
             use --ndjson or --materialize to allow this")]
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
