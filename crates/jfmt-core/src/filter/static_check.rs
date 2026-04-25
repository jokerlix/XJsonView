//! Walk the jaq AST and reject expressions that need whole-document
//! evaluation. Spec: design §3 D3 + §4.3.
//!
//! # Strategy
//!
//! AST scan. We lex the expression with [`jaq_core::load::lex::Lexer`]
//! and parse it into a [`jaq_core::load::parse::Term`] via the public
//! [`jaq_core::load::parse::Parser::term`] entry point, then recursively
//! walk every sub-term.
//!
//! # Two-group blacklist + Mode
//!
//! M4b splits the original single blacklist into two groups:
//!
//! - [`AGGREGATE_NAMES`] — whole-document aggregates (`length`,
//!   `sort_by`, `group_by`, `add`, `min`, `max`, `unique`, …).
//!   Rejected only when the compiler is in [`Mode::Streaming`].
//!   In [`Mode::Materialize`] these are legal because the entire
//!   document is loaded into memory before evaluation.
//! - [`MULTI_INPUT_NAMES`] — `input` / `inputs`, the jq multi-document
//!   stream consumers. jfmt rejects these in **both** modes (Phase 1
//!   limitation); the user-facing escape hatch is `--ndjson`.
//!
//! [`check`] takes a parsed [`Term`] reference and a [`Mode`].
//! `compile()` lexes + parses with the same machinery and hands the
//! resulting term to us.

use jaq_core::load::lex::StrPart;
use jaq_core::load::parse::{Pattern, Term};

use super::FilterError;

/// Which mode the filter compiler is operating in. Selects which
/// blacklist groups apply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// M4a streaming or NDJSON: aggregates AND multi-input both
    /// rejected.
    Streaming,
    /// M4b `--materialize`: aggregates allowed, multi-input still
    /// rejected.
    Materialize,
}

/// jq builtins that need whole-document evaluation. Rejected only in
/// [`Mode::Streaming`].
const AGGREGATE_NAMES: &[&str] = &[
    "add",
    "all",
    "any",
    "group_by",
    "length",
    "max",
    "max_by",
    "min",
    "min_by",
    "sort",
    "sort_by",
    "unique",
    "unique_by",
];

/// jq names that consume from a multi-document input stream. jfmt
/// supports neither in any mode.
const MULTI_INPUT_NAMES: &[&str] = &["input", "inputs"];

/// Walk `term` under the given `mode` and reject the first
/// blacklisted call we hit. Pre-order traversal; first hit returns.
pub fn check<S: AsRef<str>>(term: &Term<S>, mode: Mode) -> Result<(), FilterError> {
    walk(term, mode)
}

fn classify(name: &str, mode: Mode) -> Option<FilterError> {
    if MULTI_INPUT_NAMES.contains(&name) {
        return Some(FilterError::MultiInput {
            name: name.to_string(),
        });
    }
    if mode == Mode::Streaming && AGGREGATE_NAMES.contains(&name) {
        return Some(FilterError::Aggregate {
            name: name.to_string(),
        });
    }
    None
}

fn walk<S: AsRef<str>>(term: &Term<S>, mode: Mode) -> Result<(), FilterError> {
    match term {
        Term::Id | Term::Recurse | Term::Num(_) | Term::Break(_) => Ok(()),

        Term::Var(name) => {
            if let Some(err) = classify(name.as_ref(), mode) {
                return Err(err);
            }
            Ok(())
        }

        Term::Call(name, args) => {
            if let Some(err) = classify(name.as_ref(), mode) {
                return Err(err);
            }
            for a in args {
                walk(a, mode)?;
            }
            Ok(())
        }

        Term::Str(_, parts) => {
            for p in parts {
                if let StrPart::Term(t) = p {
                    walk(t, mode)?;
                }
            }
            Ok(())
        }

        Term::Arr(inner) => {
            if let Some(t) = inner {
                walk(t, mode)?;
            }
            Ok(())
        }

        Term::Obj(entries) => {
            for (k, v) in entries {
                walk(k, mode)?;
                if let Some(v) = v {
                    walk(v, mode)?;
                }
            }
            Ok(())
        }

        Term::Neg(t) => walk(t, mode),

        Term::Pipe(l, pat, r) => {
            walk(l, mode)?;
            if let Some(p) = pat {
                walk_pattern(p, mode)?;
            }
            walk(r, mode)
        }

        Term::BinOp(l, _, r) => {
            walk(l, mode)?;
            walk(r, mode)
        }

        Term::Label(_, body) => walk(body, mode),

        Term::Fold(_, init, pat, body) => {
            walk(init, mode)?;
            walk_pattern(pat, mode)?;
            for t in body {
                walk(t, mode)?;
            }
            Ok(())
        }

        Term::TryCatch(t, c) => {
            walk(t, mode)?;
            if let Some(c) = c {
                walk(c, mode)?;
            }
            Ok(())
        }

        Term::IfThenElse(branches, otherwise) => {
            for (cond, then) in branches {
                walk(cond, mode)?;
                walk(then, mode)?;
            }
            if let Some(o) = otherwise {
                walk(o, mode)?;
            }
            Ok(())
        }

        Term::Def(defs, body) => {
            for d in defs {
                walk(&d.body, mode)?;
            }
            walk(body, mode)
        }

        Term::Path(head, path) => {
            walk(head, mode)?;
            for (part, _opt) in &path.0 {
                use jaq_core::path::Part;
                match part {
                    Part::Index(t) => walk(t, mode)?,
                    Part::Range(a, b) => {
                        if let Some(a) = a {
                            walk(a, mode)?;
                        }
                        if let Some(b) = b {
                            walk(b, mode)?;
                        }
                    }
                }
            }
            Ok(())
        }
    }
}

fn walk_pattern<S: AsRef<str>>(pat: &Pattern<S>, mode: Mode) -> Result<(), FilterError> {
    match pat {
        Pattern::Var(_) => Ok(()),
        Pattern::Arr(items) => {
            for p in items {
                walk_pattern(p, mode)?;
            }
            Ok(())
        }
        Pattern::Obj(entries) => {
            for (k, p) in entries {
                walk(k, mode)?;
                walk_pattern(p, mode)?;
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filter::FilterError;
    use jaq_core::load::lex::Lexer;
    use jaq_core::load::parse::Parser;

    /// Lex + parse `expr` into a `Term` and run `check` on it under
    /// the given mode.
    fn scan_expr(expr: &str, mode: Mode) -> Result<(), FilterError> {
        let tokens = Lexer::new(expr).lex().map_err(|errs| FilterError::Parse {
            msg: format!("{errs:?}"),
        })?;
        let term: Term<&str> =
            Parser::new(&tokens)
                .parse(|p| p.term())
                .map_err(|errs| FilterError::Parse {
                    msg: format!("{errs:?}"),
                })?;
        check(&term, mode)
    }

    fn assert_aggregate(expr: &str, expected_name: &str) {
        match scan_expr(expr, Mode::Streaming) {
            Err(FilterError::Aggregate { name }) => assert_eq!(name, expected_name),
            other => panic!("expected Aggregate({expected_name:?}), got {other:?}"),
        }
    }

    fn assert_multi_input(expr: &str, expected_name: &str, mode: Mode) {
        match scan_expr(expr, mode) {
            Err(FilterError::MultiInput { name }) => assert_eq!(name, expected_name),
            other => panic!("expected MultiInput({expected_name:?}), got {other:?}"),
        }
    }

    fn assert_ok(expr: &str, mode: Mode) {
        scan_expr(expr, mode).expect("expression must pass static check");
    }

    // ---- Streaming mode: M4a behaviour preserved ----

    #[test]
    fn streaming_rejects_length() {
        assert_aggregate("length", "length");
    }
    #[test]
    fn streaming_rejects_sort_by() {
        assert_aggregate("sort_by(.x)", "sort_by");
    }
    #[test]
    fn streaming_rejects_group_by() {
        assert_aggregate("group_by(.k)", "group_by");
    }
    #[test]
    fn streaming_rejects_add() {
        assert_aggregate("add", "add");
    }
    #[test]
    fn streaming_rejects_min() {
        assert_aggregate("min", "min");
    }
    #[test]
    fn streaming_rejects_max() {
        assert_aggregate("max", "max");
    }
    #[test]
    fn streaming_rejects_unique() {
        assert_aggregate("unique", "unique");
    }
    #[test]
    fn streaming_rejects_inputs() {
        assert_multi_input("[inputs]", "inputs", Mode::Streaming);
    }
    #[test]
    fn streaming_rejects_input() {
        assert_multi_input("input", "input", Mode::Streaming);
    }
    #[test]
    fn streaming_rejects_inside_pipe() {
        assert_aggregate(".[] | length", "length");
    }

    #[test]
    fn streaming_accepts_select() {
        assert_ok("select(.x > 0)", Mode::Streaming);
    }
    #[test]
    fn streaming_accepts_path_and_arithmetic() {
        assert_ok(".a.b + 1", Mode::Streaming);
    }
    #[test]
    fn streaming_accepts_test_regex() {
        assert_ok(r#"select(.url | test("^https://"))"#, Mode::Streaming);
    }
    #[test]
    fn streaming_accepts_object_construction() {
        assert_ok("{x: .x, y: .y}", Mode::Streaming);
    }
    #[test]
    fn streaming_accepts_alternation() {
        assert_ok(".a // \"default\"", Mode::Streaming);
    }

    // ---- Materialize mode: aggregates allowed, multi-input still rejected ----

    #[test]
    fn materialize_accepts_length() {
        assert_ok("length", Mode::Materialize);
    }
    #[test]
    fn materialize_accepts_sort_by() {
        assert_ok("sort_by(.x)", Mode::Materialize);
    }
    #[test]
    fn materialize_accepts_group_by() {
        assert_ok("group_by(.k)", Mode::Materialize);
    }
    #[test]
    fn materialize_accepts_add() {
        assert_ok("add", Mode::Materialize);
    }
    #[test]
    fn materialize_accepts_min_max_unique() {
        assert_ok("min", Mode::Materialize);
        assert_ok("max", Mode::Materialize);
        assert_ok("unique", Mode::Materialize);
    }
    #[test]
    fn materialize_rejects_input() {
        assert_multi_input("input", "input", Mode::Materialize);
    }
    #[test]
    fn materialize_rejects_inputs() {
        assert_multi_input("[inputs]", "inputs", Mode::Materialize);
    }
}
