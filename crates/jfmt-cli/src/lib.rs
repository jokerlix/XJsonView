//! Library surface of the jfmt CLI. The `jfmt` binary is a thin wrapper
//! around `run_cli` (see `main.rs`); integration tests reach into the
//! convert translators via this module.

pub mod cli;
pub mod commands;
pub mod exit;

use clap::Parser;
use cli::{Cli, Command};
use exit::ExitCode;

/// Marker error: the subcommand has already written its own diagnostics to
/// stderr; main should exit with the given code without printing anything.
#[derive(Debug)]
pub struct SilentExit(pub ExitCode);

impl std::fmt::Display for SilentExit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "silent exit ({})", self.0.as_i32())
    }
}

impl std::error::Error for SilentExit {}

/// Parse argv and run the matched subcommand. Returns the process exit
/// code; the binary calls `process::exit` with it.
pub fn run_cli() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
        Ok(()) => ExitCode::Success,
        Err(e) => {
            if let Some(s) = e.downcast_ref::<SilentExit>() {
                s.0
            } else {
                eprintln!("jfmt: {e:#}");
                classify(&e)
            }
        }
    }
}

fn run(cli: Cli) -> anyhow::Result<()> {
    let threads = cli.threads;
    match cli.command {
        Command::Pretty(args) => commands::pretty::run(args, threads),
        Command::Minify(args) => commands::minify::run(args, threads),
        Command::Validate(args) => commands::validate::run(args, threads),
        Command::Filter(args) => commands::filter::run(args, threads),
        Command::Convert(args) => commands::convert::run(args),
        Command::View { file } => commands::view::run(file),
    }
}

fn classify(e: &anyhow::Error) -> ExitCode {
    if let Some(core_err) = e.downcast_ref::<jfmt_core::Error>() {
        if matches!(core_err, jfmt_core::Error::Syntax { .. }) {
            return ExitCode::SyntaxError;
        }
    }
    if let Some(filt) = e.downcast_ref::<jfmt_core::FilterError>() {
        if matches!(
            filt,
            jfmt_core::FilterError::Parse { .. } | jfmt_core::FilterError::Aggregate { .. }
        ) {
            return ExitCode::SyntaxError;
        }
    }
    if let Some(xml_err) = e.downcast_ref::<jfmt_xml::XmlError>() {
        if matches!(xml_err, jfmt_xml::XmlError::Parse { .. }) {
            return ExitCode::XmlSyntax;
        }
    }
    if e.downcast_ref::<commands::convert::xml_to_json::NonContiguousSiblings>()
        .is_some()
    {
        return ExitCode::StrictNonContiguous;
    }
    if e.downcast_ref::<commands::convert::xml_to_json::ArrayRuleMultiple>()
        .is_some()
    {
        return ExitCode::Translation;
    }
    ExitCode::InputError
}
