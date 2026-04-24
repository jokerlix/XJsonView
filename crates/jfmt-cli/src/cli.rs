//! Clap argument definitions.

use clap::{Args, Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(name = "jfmt", version, about = "Streaming JSON/NDJSON formatter")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Pretty-print a JSON document with indentation.
    Pretty(PrettyArgs),
    /// Minify a JSON document, removing all whitespace.
    Minify(MinifyArgs),
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
