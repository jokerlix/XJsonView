//! Parse + static-check + jaq compile.
//!
//! # Two parses on the same source
//!
//! `compile()` lexes + parses the expression *twice*:
//!
//! 1. Once via [`Lexer`] + [`Parser::term`] to obtain a [`Term<&str>`]
//!    that we hand to [`static_check::check`] (Task 4). Module-body
//!    access on a parsed [`load::Modules`] is `pub(crate)` in jaq-core
//!    2.2.1, so we cannot share the AST between static-check and the
//!    compiler.
//! 2. Once via [`Loader::load`] which produces [`load::Modules`] —
//!    the input shape that [`Compiler::compile`] requires.
//!
//! Re-parsing a single jq expression is cheap. If a future jaq-core
//! release exposes the term tree from `Modules`, this can become a
//! single-pass.

use std::sync::Arc;

use jaq_core::load::lex::Lexer;
use jaq_core::load::parse::{Parser, Term};
use jaq_core::load::{Arena, File, Loader};
use jaq_core::Compiler;

use super::{static_check, FilterError};

/// Compiled, ready-to-run filter. Cheap to clone (`Arc` inside) so it
/// can be shared across NDJSON workers.
#[derive(Clone)]
pub struct Compiled {
    // Read by `filter::runtime` (Task 6) once it lands.
    #[allow(dead_code)]
    pub(crate) inner: Arc<CompiledInner>,
}

impl std::fmt::Debug for Compiled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Compiled").finish_non_exhaustive()
    }
}

pub(crate) struct CompiledInner {
    /// The jaq filter — ready to run against [`jaq_json::Val`].
    // Read by `filter::runtime` (Task 6) once it lands.
    #[allow(dead_code)]
    pub(crate) filter: jaq_core::Filter<jaq_core::Native<jaq_json::Val>>,
}

/// Compile a jq expression into a [`Compiled`] filter.
///
/// Pipeline:
/// 1. Lex + parse `expr` into a [`Term`] and run [`static_check::check`]
///    to reject expressions that need whole-document semantics.
/// 2. Re-parse via [`Loader::load`] to obtain [`load::Modules`].
/// 3. Hand the modules to [`Compiler`] together with `jaq_std::funs()`
///    and `jaq_json::funs()` to obtain the runnable filter.
pub fn compile(expr: &str) -> Result<Compiled, FilterError> {
    // (1) Lex + parse for the static-check pass. Mirrors the helper
    //     in `static_check.rs`'s tests.
    let term = parse_term(expr)?;
    static_check::check(&term)?;

    // (2) Re-parse via the higher-level Loader for the Compiler input.
    let arena = Arena::default();
    let loader = Loader::new(jaq_std::defs().chain(jaq_json::defs()));
    let modules = loader
        .load(
            &arena,
            File {
                path: (),
                code: expr,
            },
        )
        .map_err(|errs| FilterError::Parse {
            msg: format_load_errors(&errs),
        })?;

    // (3) Compile into a runnable filter.
    let compiler = Compiler::default().with_funs(jaq_std::funs().chain(jaq_json::funs()));
    let filter = compiler
        .compile(modules)
        .map_err(|errs| FilterError::Parse {
            msg: format_compile_errors(&errs),
        })?;

    Ok(Compiled {
        inner: Arc::new(CompiledInner { filter }),
    })
}

/// Lex `expr` and parse it as a single term. Returns `Term<&str>` that
/// borrows from `expr`. Mirrors the test helper in `static_check.rs`.
fn parse_term(expr: &str) -> Result<Term<&str>, FilterError> {
    let tokens = Lexer::new(expr).lex().map_err(|errs| FilterError::Parse {
        msg: format!("{errs:?}"),
    })?;
    Parser::new(&tokens)
        .parse(|p| p.term())
        .map_err(|errs| FilterError::Parse {
            msg: format!("{errs:?}"),
        })
}

/// Format the `Errors` value returned by [`Loader::load`]. Debug
/// formatting is good enough for M4a; Task 11 (CLI) can dress it up.
fn format_load_errors<E: std::fmt::Debug>(errs: &E) -> String {
    format!("{errs:?}")
}

/// Format the `Errors` value returned by [`Compiler::compile`].
fn format_compile_errors<E: std::fmt::Debug>(errs: &E) -> String {
    format!("{errs:?}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filter::FilterError;

    #[test]
    fn parse_error_reports_message() {
        let err = compile("not a valid )(").unwrap_err();
        assert!(matches!(err, FilterError::Parse { .. }));
    }

    #[test]
    fn aggregate_is_rejected_at_compile() {
        match compile("length") {
            Err(FilterError::Aggregate { name }) => assert_eq!(name, "length"),
            other => panic!("got {other:?}"),
        }
    }

    #[test]
    fn legal_expression_compiles() {
        compile("select(.x > 0)").expect("compile");
    }

    #[test]
    fn select_with_path_compiles() {
        compile(".[] | select(.id > 100)").expect("compile");
    }
}
