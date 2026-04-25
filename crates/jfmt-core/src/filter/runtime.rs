//! Run a `Compiled` filter against one `serde_json::Value`. The
//! `inputs` iterator is always empty so any `input` / `inputs`
//! reference that slipped past the static check raises a clean
//! runtime error.

use super::{Compiled, FilterError};
use serde_json::Value;

/// Run `compiled` against `input`. Returns the stream of jaq output
/// values (0, 1, or N).
pub fn run_one(compiled: &Compiled, input: Value) -> Result<Vec<Value>, FilterError> {
    use jaq_core::{Ctx, RcIter};
    use jaq_json::Val;

    // Empty inputs iterator — the static check rejects `inputs`/`input`
    // expressions, but if any slip through they hit this empty source
    // and produce a clean jaq runtime error.
    let inputs: RcIter<core::iter::Empty<Result<Val, _>>> = RcIter::new(core::iter::empty());

    let ctx = Ctx::new([], &inputs);
    let val: Val = input.into();

    let mut out = Vec::new();
    for r in compiled.inner.filter.run((ctx, val)) {
        let v: Val = r.map_err(|e| FilterError::Runtime {
            where_: String::new(),
            msg: format!("{e}"),
        })?;
        out.push(serde_json::Value::from(v));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filter::compile;
    use serde_json::json;

    #[test]
    fn select_passing_returns_value() {
        let c = compile("select(.x > 0)").unwrap();
        let out = run_one(&c, json!({"x": 1})).unwrap();
        assert_eq!(out, vec![json!({"x": 1})]);
    }

    #[test]
    fn select_failing_returns_empty() {
        let c = compile("select(.x > 0)").unwrap();
        let out = run_one(&c, json!({"x": -1})).unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn comma_returns_two() {
        let c = compile(".a, .b").unwrap();
        let out = run_one(&c, json!({"a": 1, "b": 2})).unwrap();
        assert_eq!(out, vec![json!(1), json!(2)]);
    }

    #[test]
    fn type_error_reports_runtime() {
        let c = compile(".x + 1").unwrap();
        let err = run_one(&c, json!({"x": "string"})).unwrap_err();
        assert!(matches!(err, FilterError::Runtime { .. }));
    }
}
