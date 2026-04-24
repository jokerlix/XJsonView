use crate::cli::MinifyArgs;
use anyhow::Context;
use jfmt_core::{transcode, MinifyWriter};

pub fn run(args: MinifyArgs) -> anyhow::Result<()> {
    let input = jfmt_io::open_input(&args.common.input_spec()).context("opening input")?;
    let output = jfmt_io::open_output(&args.common.output_spec()).context("opening output")?;
    let writer = MinifyWriter::new(output);
    transcode(input, writer).context("minifying failed")?;
    Ok(())
}
