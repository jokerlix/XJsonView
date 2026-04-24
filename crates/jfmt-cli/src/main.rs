mod cli;
mod commands;
mod exit;

use clap::Parser;
use cli::{Cli, Command};
use exit::ExitCode;
use std::process;

fn main() {
    let cli = Cli::parse();
    let code = match run(cli) {
        Ok(()) => ExitCode::Success,
        Err(e) => {
            eprintln!("jfmt: {e:#}");
            classify(&e)
        }
    };
    process::exit(code.as_i32());
}

fn run(cli: Cli) -> anyhow::Result<()> {
    match cli.command {
        Command::Pretty(args) => commands::pretty::run(args),
        Command::Minify(args) => commands::minify::run(args),
    }
}

fn classify(e: &anyhow::Error) -> ExitCode {
    if let Some(core_err) = e.downcast_ref::<jfmt_core::Error>() {
        if matches!(core_err, jfmt_core::Error::Syntax { .. }) {
            return ExitCode::SyntaxError;
        }
    }
    ExitCode::InputError
}
