//! Clap argument definitions.

use clap::{Args, Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(name = "jfmt", version, about = "Streaming JSON/NDJSON formatter")]
pub struct Cli {
    /// Worker threads for --ndjson pipelines. 0 = physical cores;
    /// 1 = serial; >=2 = parallel. Ignored in single-document mode.
    #[arg(long = "threads", global = true, default_value_t = 0)]
    pub threads: usize,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Pretty-print a JSON document with indentation.
    Pretty(PrettyArgs),
    /// Minify a JSON document, removing all whitespace.
    Minify(MinifyArgs),
    /// Validate JSON / NDJSON syntax and optionally emit stats.
    Validate(ValidateArgs),
    /// Filter JSON / NDJSON with a jq expression.
    Filter(FilterArgs),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum CompressArg {
    None,
    Gz,
    Zst,
}

impl From<CompressArg> for jfmt_io::Compression {
    fn from(c: CompressArg) -> Self {
        match c {
            CompressArg::None => jfmt_io::Compression::None,
            CompressArg::Gz => jfmt_io::Compression::Gzip,
            CompressArg::Zst => jfmt_io::Compression::Zstd,
        }
    }
}

#[derive(Debug, Args)]
pub struct CommonArgs {
    /// Input path. Omit or use `-` for stdin.
    #[arg(value_name = "INPUT")]
    pub input: Option<String>,

    /// Output path. Omit to write to stdout.
    #[arg(short = 'o', long = "output", value_name = "PATH")]
    pub output: Option<PathBuf>,

    /// Override input compression detection.
    #[arg(long = "compress", value_enum)]
    pub compress: Option<CompressArg>,

    /// Treat input as NDJSON (one JSON value per line). Accepted for
    /// forward-compat; NDJSON fast path lands in M3.
    #[arg(long = "ndjson")]
    pub ndjson: bool,
}

#[derive(Debug, Args)]
pub struct PrettyArgs {
    #[command(flatten)]
    pub common: CommonArgs,

    /// Number of spaces per indent level.
    #[arg(long = "indent", value_name = "N", default_value_t = 2)]
    pub indent: u8,

    /// Indent with tabs instead of spaces.
    #[arg(long = "tabs", conflicts_with = "indent")]
    pub tabs: bool,
}

#[derive(Debug, Args)]
pub struct MinifyArgs {
    #[command(flatten)]
    pub common: CommonArgs,
}

#[derive(Debug, Args)]
pub struct ValidateArgs {
    #[command(flatten)]
    pub common: CommonArgs,

    /// Emit a human-readable stats summary to stderr.
    #[arg(long = "stats")]
    pub stats: bool,

    /// Emit structured stats as JSON to PATH.
    #[arg(long = "stats-json", value_name = "PATH")]
    pub stats_json: Option<PathBuf>,

    /// In NDJSON mode, stop at the first bad line.
    #[arg(long = "fail-fast")]
    pub fail_fast: bool,
}

#[derive(Debug, Args)]
pub struct FilterArgs {
    /// jq expression (per-shard semantics; see `jfmt filter --help`).
    #[arg(value_name = "EXPR")]
    pub expr: String,

    #[command(flatten)]
    pub common: CommonArgs,

    /// Promote runtime jq errors to fatal exit (code 1).
    #[arg(long = "strict")]
    pub strict: bool,

    /// Pretty-print output. Conflicts with --compact and --ndjson.
    #[arg(long = "pretty", conflicts_with = "compact")]
    pub pretty: bool,

    /// Compact output (default).
    #[arg(long = "compact")]
    pub compact: bool,

    /// Indent width when --pretty is set.
    #[arg(long = "indent", value_name = "N", default_value_t = 2, requires = "pretty")]
    pub indent: u8,
}

impl CommonArgs {
    #[allow(dead_code)] // consumed once subcommands wire up in Tasks 14–15
    pub fn input_spec(&self) -> jfmt_io::InputSpec {
        let path = match self.input.as_deref() {
            None | Some("-") => None,
            Some(p) => Some(PathBuf::from(p)),
        };
        jfmt_io::InputSpec {
            path,
            compression: self.compress.map(Into::into),
        }
    }

    #[allow(dead_code)]
    pub fn output_spec(&self) -> jfmt_io::OutputSpec {
        let mut spec = match &self.output {
            Some(p) => jfmt_io::OutputSpec::file(p.clone()),
            None => jfmt_io::OutputSpec::stdout(),
        };
        spec.gzip_level = 6;
        spec.zstd_level = 3;
        spec
    }
}
