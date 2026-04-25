//! Walk the jaq AST and reject expressions that need whole-document
//! evaluation. Spec: design §3 D3 + §4.3.
//!
//! # Strategy
//!
//! AST scan. We lex the expression with [`jaq_core::load::lex::Lexer`]
//! and parse it into a [`jaq_core::load::parse::Term`] via the public
//! [`jaq_core::load::parse::Parser::term`] entry point, then recursively
//! walk every sub-term. Any [`Term::Call(name, _)`] whose name appears
//! in [`BLACKLIST`] is rejected with [`FilterError::Aggregate`]. We also
//! reject the bare-word forms (jaq parses `length`, `add`, `input`,
//! `inputs` as zero-arg calls, so they hit the same branch).
//!
//! AST scanning is preferred over source-text scanning because it
//! ignores blacklisted names that appear inside string literals,
//! comments, or as user-defined function names. Module body access in
//! `jaq-core 2.2` is `pub(crate)`, so we cannot consume the result of
//! [`Loader::load`] directly — but [`Parser::term`] is public and gives
//! us the term AST for a single expression, which is exactly what
//! `jfmt filter` accepts.
//!
//! # Public API
//!
//! [`check`] takes a parsed [`Term`] reference. Task 5 (`compile.rs`)
//! lexes + parses with the same machinery and hands the resulting term
//! to us.

use jaq_core::load::lex::StrPart;
use jaq_core::load::parse::{Pattern, Term};

use super::FilterError;

/// jq builtins that require whole-document semantics. Calling any of
/// these on a single shard would yield wrong results, so we reject the
/// expression at compile time. The static check is a fail-fast
/// convenience; the runtime guard in Task 6 is the real safety net.
const BLACKLIST: &[&str] = &[
    "add",
    "all",
    "any",
    "group_by",
    "input",
    "inputs",
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

/// Walk `term` and return `Err(FilterError::Aggregate { name })` on the
/// first blacklisted call we hit. The traversal is a depth-first
/// pre-order walk; on the first hit we return immediately.
pub fn check<S: AsRef<str>>(term: &Term<S>) -> Result<(), FilterError> {
    walk(term)
}

fn walk<S: AsRef<str>>(term: &Term<S>) -> Result<(), FilterError> {
    match term {
        Term::Id | Term::Recurse | Term::Num(_) | Term::Break(_) => Ok(()),

        Term::Var(name) => {
            // jaq parses bare identifiers like `length` as `Call`, but
            // `$x`-style variables come through `Var`. Variables can
            // never name a builtin (they all start with `$`), so this
            // is just for completeness.
            let n = name.as_ref();
            if BLACKLIST.contains(&n) {
                return Err(FilterError::Aggregate { name: n.to_string() });
            }
            Ok(())
        }

        Term::Call(name, args) => {
            let n = name.as_ref();
            if BLACKLIST.contains(&n) {
                return Err(FilterError::Aggregate { name: n.to_string() });
            }
            for a in args {
                walk(a)?;
            }
            Ok(())
        }

        Term::Str(_, parts) => {
            for p in parts {
                if let StrPart::Term(t) = p {
                    walk(t)?;
                }
            }
            Ok(())
        }

        Term::Arr(inner) => {
            if let Some(t) = inner {
                walk(t)?;
            }
            Ok(())
        }

        Term::Obj(entries) => {
            for (k, v) in entries {
                walk(k)?;
                if let Some(v) = v {
                    walk(v)?;
                }
            }
            Ok(())
        }

        Term::Neg(t) => walk(t),

        Term::Pipe(l, pat, r) => {
            walk(l)?;
            if let Some(p) = pat {
                walk_pattern(p)?;
            }
            walk(r)
        }

        Term::BinOp(l, _, r) => {
            walk(l)?;
            walk(r)
        }

        Term::Label(_, body) => walk(body),

        Term::Fold(_, init, pat, body) => {
            walk(init)?;
            walk_pattern(pat)?;
            for t in body {
                walk(t)?;
            }
            Ok(())
        }

        Term::TryCatch(t, c) => {
            walk(t)?;
            if let Some(c) = c {
                walk(c)?;
            }
            Ok(())
        }

        Term::IfThenElse(branches, otherwise) => {
            for (cond, then) in branches {
                walk(cond)?;
                walk(then)?;
            }
            if let Some(o) = otherwise {
                walk(o)?;
            }
            Ok(())
        }

        Term::Def(defs, body) => {
            // We deliberately do NOT skip user-defined definitions
            // even if they shadow a builtin: a user could write
            // `def length: 0; length` — but our blacklist is on names,
            // not on resolved bindings. False positives here are
            // acceptable; a user shadowing a builtin name we reject
            // is exotic and the workaround (rename) is trivial.
            for d in defs {
                walk(&d.body)?;
            }
            walk(body)
        }

        Term::Path(head, path) => {
            walk(head)?;
            for (part, _opt) in &path.0 {
                use jaq_core::path::Part;
                match part {
                    Part::Index(t) => walk(t)?,
                    Part::Range(a, b) => {
                        if let Some(a) = a {
                            walk(a)?;
                        }
                        if let Some(b) = b {
                            walk(b)?;
                        }
                    }
                }
            }
            Ok(())
        }
    }
}

fn walk_pattern<S: AsRef<str>>(pat: &Pattern<S>) -> Result<(), FilterError> {
    match pat {
        Pattern::Var(_) => Ok(()),
        Pattern::Arr(items) => {
            for p in items {
                walk_pattern(p)?;
            }
            Ok(())
        }
        Pattern::Obj(entries) => {
            for (k, p) in entries {
                walk(k)?;
                walk_pattern(p)?;
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

    /// Lex + parse `expr` into a [`Term`] and run [`check`] on it.
    fn scan_expr(expr: &str) -> Result<(), FilterError> {
        let tokens = Lexer::new(expr)
            .lex()
            .map_err(|errs| FilterError::Parse { msg: format!("{errs:?}") })?;
        let term: Term<&str> = Parser::new(&tokens)
            .parse(|p| p.term())
            .map_err(|errs| FilterError::Parse { msg: format!("{errs:?}") })?;
        check(&term)
    }

    fn assert_aggregate(expr: &str, expected_name: &str) {
        match scan_expr(expr) {
            Err(FilterError::Aggregate { name }) => assert_eq!(name, expected_name),
            other => panic!("expected Aggregate({expected_name:?}), got {other:?}"),
        }
    }

    fn assert_ok(expr: &str) {
        scan_expr(expr).expect("expression must pass static check");
    }

    #[test]
    fn rejects_length() {
        assert_aggregate("length", "length");
    }
    #[test]
    fn rejects_sort_by() {
        assert_aggregate("sort_by(.x)", "sort_by");
    }
    #[test]
    fn rejects_group_by() {
        assert_aggregate("group_by(.k)", "group_by");
    }
    #[test]
    fn rejects_add() {
        assert_aggregate("add", "add");
    }
    #[test]
    fn rejects_min() {
        assert_aggregate("min", "min");
    }
    #[test]
    fn rejects_max() {
        assert_aggregate("max", "max");
    }
    #[test]
    fn rejects_unique() {
        assert_aggregate("unique", "unique");
    }
    #[test]
    fn rejects_inputs() {
        assert_aggregate("[inputs]", "inputs");
    }
    #[test]
    fn rejects_input() {
        assert_aggregate("input", "input");
    }
    #[test]
    fn rejects_inside_pipe() {
        assert_aggregate(".[] | length", "length");
    }

    #[test]
    fn accepts_select() {
        assert_ok("select(.x > 0)");
    }
    #[test]
    fn accepts_path_and_arithmetic() {
        assert_ok(".a.b + 1");
    }
    #[test]
    fn accepts_test_regex() {
        assert_ok(r#"select(.url | test("^https://"))"#);
    }
    #[test]
    fn accepts_object_construction() {
        assert_ok("{x: .x, y: .y}");
    }
    #[test]
    fn accepts_alternation() {
        assert_ok(".a // \"default\"");
    }
}
