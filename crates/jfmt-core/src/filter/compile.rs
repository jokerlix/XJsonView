//! Parse + static-check + jaq compile.

use super::FilterError;

/// Compiled filter. Cheap to clone (`Arc` inside) so it can be shared
/// across NDJSON workers.
#[derive(Clone)]
pub struct Compiled {
    // Filled in Task 5.
    _placeholder: (),
}

pub fn compile(_expr: &str) -> Result<Compiled, FilterError> {
    unimplemented!("Task 5 fills this in")
}
