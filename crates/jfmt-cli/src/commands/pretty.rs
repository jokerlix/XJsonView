use crate::cli::PrettyArgs;
use anyhow::Context;
use jfmt_core::{transcode, PrettyConfig, PrettyWriter};

pub fn run(args: PrettyArgs) -> anyhow::Result<()> {
    let input = jfmt_io::open_input(&args.common.input_spec()).context("opening input")?;
    let output = jfmt_io::open_output(&args.common.output_spec()).context("opening output")?;

    let cfg = PrettyConfig {
        indent: args.indent,
        use_tabs: args.tabs,
        newline: "\n",
    };
    let writer = PrettyWriter::with_config(output, cfg);
    transcode(input, writer).context("pretty-printing failed")?;
    Ok(())
}
