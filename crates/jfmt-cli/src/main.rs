mod cli;
mod commands;
mod exit;

use clap::Parser;
use cli::{Cli, Command};
use exit::ExitCode;
use std::process;

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

fn main() {
    let cli = Cli::parse();
    let code = match run(cli) {
        Ok(()) => ExitCode::Success,
        Err(e) => {
            if let Some(s) = e.downcast_ref::<SilentExit>() {
                s.0
            } else {
                eprintln!("jfmt: {e:#}");
                classify(&e)
            }
        }
    };
    process::exit(code.as_i32());
}

fn run(cli: Cli) -> anyhow::Result<()> {
    let threads = cli.threads;
    match cli.command {
        Command::Pretty(args) => commands::pretty::run(args, threads),
        Command::Minify(args) => commands::minify::run(args, threads),
        Command::Validate(args) => commands::validate::run(args, threads),
        Command::Filter(args) => commands::filter::run(args, threads),
        Command::Convert(args) => commands::convert::run(args),
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
    ExitCode::InputError
}
