//! `jfmt convert` — JSON ↔ XML conversion.

pub mod array_rule;
pub mod format;
pub mod xml_to_json;

use crate::cli::ConvertArgs;
use anyhow::{anyhow, bail, Context, Result};
use format::Format;
use std::io::{BufRead, Write};

pub fn run(args: ConvertArgs) -> Result<()> {
    let from = resolve_from(&args)?;
    let to = resolve_to(&args, from)?;
    if from == to {
        bail!(
            "--from and --to are both {:?}; convert requires different formats",
            from
        );
    }

    // Open input + output via jfmt-io. (Future tasks fill the bodies.)
    let input = open_input(&args)?;
    let output = open_output(&args)?;

    match (from, to) {
        (Format::Xml, Format::Json) => xml_to_json::translate(input, output, &args),
        (Format::Json, Format::Xml) => bail!("JSON → XML not yet implemented (Task 10)"),
        _ => unreachable!("from != to enforced above"),
    }
}

fn resolve_from(args: &ConvertArgs) -> Result<Format> {
    if let Some(f) = args.from {
        return Ok(f);
    }
    let path = args
        .input
        .as_deref()
        .ok_or_else(|| anyhow!("--from is required when reading from stdin"))?;
    format::infer_from_path(path)
        .ok_or_else(|| anyhow!("cannot infer --from from path {path:?}; pass --from xml|json"))
}

fn resolve_to(args: &ConvertArgs, from: Format) -> Result<Format> {
    if let Some(t) = args.to {
        return Ok(t);
    }
    if let Some(path) = &args.output {
        if let Some(t) = format::infer_from_path(path) {
            return Ok(t);
        }
    }
    // Default: opposite of from.
    Ok(match from {
        Format::Json => Format::Xml,
        Format::Xml => Format::Json,
    })
}

fn open_input(args: &ConvertArgs) -> Result<Box<dyn BufRead + Send>> {
    let spec = jfmt_io::InputSpec {
        path: args.input.clone(),
        compression: None,
    };
    jfmt_io::open_input(&spec).with_context(|| format!("opening {:?}", args.input))
}

fn open_output(args: &ConvertArgs) -> Result<Box<dyn Write + Send>> {
    let spec = match &args.output {
        Some(p) => jfmt_io::OutputSpec::file(p.clone()),
        None => jfmt_io::OutputSpec::stdout(),
    };
    jfmt_io::open_output(&spec).with_context(|| format!("creating {:?}", args.output))
}
