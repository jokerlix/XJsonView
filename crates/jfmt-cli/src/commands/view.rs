use std::path::Path;

use anyhow::{anyhow, Result};

/// M8.1 placeholder: errors with a clear message. M8.2 replaces this with
/// real GUI binary discovery + spawn.
pub fn run<P: AsRef<Path>>(_file: P) -> Result<()> {
    Err(anyhow!(
        "GUI viewer not yet bundled — run `apps/jfmt-viewer` directly during M8.1 development. \
         Production `jfmt view` integration ships in M8.2."
    ))
}
