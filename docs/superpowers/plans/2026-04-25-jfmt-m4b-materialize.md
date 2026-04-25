# jfmt M4b — `--materialize` Filter Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `jfmt filter -m | --materialize EXPR` with full-jq semantics, RAM budget pre-flight, `--force` override, multi-value stream output. Tagged as `v0.0.5`.

**Architecture:** A new `filter::run_materialize` reads the entire document into a `serde_json::Value`, runs the existing `runtime::run_one` against it once, and writes the resulting 0/1/N values as a JSON-value stream. The static check splits its blacklist into aggregate vs multi-input groups; aggregates are allowed in materialize mode, multi-input names (`input`/`inputs`) are always rejected. The CLI estimates peak RAM from file size before materialize and aborts unless `--force` is set; stdin input skips the check.

**Tech Stack:** Rust 2021 / MSRV 1.75 · existing jaq-{core,std,json}=2.x stack from M4a · `sysinfo` (version frozen by Task 1's spike) · `serde_json::Value` · existing `MinifyWriter` / `PrettyWriter`.

**Spec:** `docs/superpowers/specs/2026-04-25-jfmt-m4b-materialize-design.md`.

---

## File Structure

### New files

| Path | Responsibility |
|---|---|
| `crates/jfmt-core/src/filter/materialize.rs` | `run_materialize(reader, writer, &Compiled, FilterOutput, FilterOptions) -> Result<MaterializeReport, FilterError>`. Loads → jaq → multi-value stream output. |
| `crates/jfmt-core/tests/filter_materialize.rs` | Integration tests for run_materialize (length/sort_by/group_by/.[]/strict). |

### Modified files

| Path | Change |
|---|---|
| `crates/jfmt-core/src/filter/static_check.rs` | Add public `Mode { Streaming, Materialize }` enum; split `BLACKLIST` into `AGGREGATE_NAMES` and `MULTI_INPUT_NAMES`; `check()` takes `mode`; new error path raises `FilterError::MultiInput`. |
| `crates/jfmt-core/src/filter/compile.rs` | `compile()` takes `mode: Mode`; passes it to `static_check::check`. |
| `crates/jfmt-core/src/filter/mod.rs` | Add `FilterError::MultiInput` and `FilterError::BudgetExceeded`; re-export `Mode`, `run_materialize`, `MaterializeReport`. Update `run_streaming` and `run_ndjson` callers of `compile()` and `static_check::check()`. |
| `crates/jfmt-cli/src/cli.rs` | `FilterArgs` gets `materialize: bool` (`-m`/`--materialize`) and `force: bool` (clap `requires = "materialize"`). `materialize.conflicts_with("ndjson")`. |
| `crates/jfmt-cli/src/commands/filter.rs` | New `if args.materialize` branch: estimate peak RAM (file path only), abort with `BudgetExceeded` unless `--force`, otherwise call `run_materialize`. |
| `crates/jfmt-cli/src/main.rs` | `classify` maps `MultiInput` to `SyntaxError`. `BudgetExceeded` already maps to `InputError` via the default arm. |
| `Cargo.toml` (workspace) | Add `sysinfo = "=<X.Y.Z>"` (version frozen by Task 1). |
| `crates/jfmt-cli/Cargo.toml` | Pull `sysinfo`. |
| `README.md` | Append a `### Filter — full jq mode (--materialize)` subsection. Update Status to v0.0.5. |
| `docs/superpowers/specs/2026-04-23-jfmt-phase1-design.md` | Mark M4b shipped. |
| `Cargo.toml` (workspace) | `version = "0.0.5"`. |

---

## Task 1: Spike & freeze `sysinfo` version

**Why:** like jaq in M4a, `sysinfo` has had MSRV churn. Pin a version that compiles on rustc 1.75 and exposes `total_memory()` as `u64` bytes.

**Files:**
- Create / delete: `crates/jfmt-cli/examples/sysinfo_spike.rs`
- Modify: `Cargo.toml` (workspace), `crates/jfmt-cli/Cargo.toml`

- [ ] **Step 1: Search current sysinfo versions**

Run:
```bash
cargo search sysinfo --limit 8
```

Pick the highest version that documentation says supports rustc 1.75. If unsure, start with the highest, attempt `cargo build`, step down on edition2024 errors. Record the chosen version.

- [ ] **Step 2: Add provisional dep**

Edit `Cargo.toml` (workspace). Add to `[workspace.dependencies]`:

```toml
# RAM budget check for `jfmt filter --materialize` (M4b).
sysinfo = "=<X.Y.Z>"
```

Edit `crates/jfmt-cli/Cargo.toml`. Add under `[dependencies]`:

```toml
sysinfo = { workspace = true }
```

- [ ] **Step 3: Write the spike**

Create `crates/jfmt-cli/examples/sysinfo_spike.rs`:

```rust
fn main() {
    // The actual API depends on the chosen version. The spike must
    // resolve to: `total_ram_bytes() -> u64` returning total system RAM.
    // Newer sysinfo (0.30+) uses `System::new_all()` then `.total_memory()`
    // (in bytes). Older versions returned KiB and required `.refresh_memory()`.
    //
    // Adjust to whichever is true for the chosen version.
    let mut sys = sysinfo::System::new();
    sys.refresh_memory();
    let total_bytes: u64 = sys.total_memory();
    assert!(total_bytes > 64 * 1024 * 1024, "expected at least 64 MiB total RAM, got {total_bytes}");
    println!("spike OK: total RAM = {total_bytes} bytes");
}
```

If `total_memory()` returns a value that's clearly KiB rather than bytes (e.g., < 1_000_000 on a multi-GiB machine), the chosen sysinfo version is on the older line; multiply by 1024 in production code AND update the spike's assertion to match. Whatever you discover, **document the unit in Annex B of the spec** in Step 5.

- [ ] **Step 4: Run the spike**

Run: `cargo run -p jfmt-cli --example sysinfo_spike`
Expected: prints `spike OK: total RAM = <N> bytes` where N is plausibly your machine's RAM in bytes.

If MSRV 1.75 cannot be satisfied by any sysinfo version (all latest minor releases require Rust 1.85+), STOP and report BLOCKED with what you found. Fallback to hand-rolled syscalls is documented in spec §8.1 but is its own task — escalate first.

- [ ] **Step 5: Append Annex B to spec**

Append to `docs/superpowers/specs/2026-04-25-jfmt-m4b-materialize-design.md`:

```markdown
## Annex B — sysinfo API mapping (frozen by Task 1 spike)

- Version: sysinfo=<X.Y.Z>.
- Constructor: `<actual constructor>` (e.g., `sysinfo::System::new()`).
- Refresh: `<refresh call or note that none is required>`.
- Total RAM: `<actual function>(...) -> u64` returning bytes (or KiB; see unit).
- Unit: <bytes | KiB>.

The `cli/commands/filter.rs::system_total_ram_bytes()` helper calls
this exact sequence.
```

Replace each `<…>` with the real symbol from the chosen version. Subsequent tasks reference Annex B.

- [ ] **Step 6: Delete the example**

Run: `git rm crates/jfmt-cli/examples/sysinfo_spike.rs`

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml Cargo.lock crates/jfmt-cli/Cargo.toml docs/superpowers/specs/2026-04-25-jfmt-m4b-materialize-design.md
git commit -m "$(cat <<'EOF'
chore(deps): add sysinfo pinned for M4b RAM budget check

Version frozen via spike (see spec Annex B). MSRV-1.75 verified by
running an example that prints total memory, then deleted.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 2: Split static_check blacklist + add `Mode`

Splits the M4a blacklist into two groups, threads a `Mode` parameter through. `MultiInput` becomes a new error variant.

**Files:**
- Modify: `crates/jfmt-core/src/filter/mod.rs` (add `MultiInput` variant)
- Modify: `crates/jfmt-core/src/filter/static_check.rs`

- [ ] **Step 1: Add `MultiInput` to FilterError**

In `crates/jfmt-core/src/filter/mod.rs`, inside the `FilterError` enum, immediately after the `Aggregate` variant, insert:

```rust
    /// Static check rejected `input` / `inputs` (jaq multi-document
    /// stream). jfmt does not support multi-document streams in any
    /// mode (Phase 1 limitation); use `--ndjson` for per-line full
    /// semantics.
    #[error(
        "filter expression uses '{name}'; jfmt does not support jq \
         multi-document streams. Use --ndjson for per-line full semantics."
    )]
    MultiInput { name: String },
```

- [ ] **Step 2: Write the failing tests inline in `static_check.rs`**

In `crates/jfmt-core/src/filter/static_check.rs`, find the existing `#[cfg(test)] mod tests` block. Replace the whole `mod tests` block with:

```rust
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
```

- [ ] **Step 3: Run the failing tests**

Run: `cargo test -p jfmt-core --lib filter::static_check`
Expected: tests fail because `Mode` doesn't exist and the production `check` signature still takes only `&Term<S>`.

- [ ] **Step 4: Implement Mode + split blacklist**

Replace the `BLACKLIST` constant and the `check` / `walk` functions with:

```rust
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
```

Update the module-level doc-comment at the top of the file to mention the two-group split (replace the §3 D3 reference with §3 D2 of the M4b spec where appropriate, but keep the existing prose otherwise).

- [ ] **Step 5: Update `compile()` to take `mode`**

In `crates/jfmt-core/src/filter/compile.rs`, change the signature of `compile`:

```rust
pub fn compile(expr: &str, mode: super::static_check::Mode) -> Result<Compiled, FilterError> {
    let term = parse_term(expr)?;
    static_check::check(&term, mode)?;

    // … rest unchanged …
}
```

Update each existing test in `compile.rs` to pass `Mode::Streaming`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::filter::static_check::Mode;
    use crate::filter::FilterError;

    #[test]
    fn parse_error_reports_message() {
        let err = compile("not a valid )(", Mode::Streaming).unwrap_err();
        assert!(matches!(err, FilterError::Parse { .. }));
    }

    #[test]
    fn aggregate_is_rejected_at_compile_in_streaming() {
        match compile("length", Mode::Streaming) {
            Err(FilterError::Aggregate { name }) => assert_eq!(name, "length"),
            other => panic!("got {other:?}"),
        }
    }

    #[test]
    fn aggregate_is_accepted_in_materialize() {
        compile("length", Mode::Materialize).expect("materialize accepts length");
    }

    #[test]
    fn multi_input_is_rejected_in_both_modes() {
        match compile("input", Mode::Streaming) {
            Err(FilterError::MultiInput { name }) => assert_eq!(name, "input"),
            other => panic!("streaming: got {other:?}"),
        }
        match compile("input", Mode::Materialize) {
            Err(FilterError::MultiInput { name }) => assert_eq!(name, "input"),
            other => panic!("materialize: got {other:?}"),
        }
    }

    #[test]
    fn legal_expression_compiles_in_streaming() {
        compile("select(.x > 0)", Mode::Streaming).expect("compile");
    }

    #[test]
    fn select_with_path_compiles_in_streaming() {
        compile(".[] | select(.id > 100)", Mode::Streaming).expect("compile");
    }
}
```

- [ ] **Step 6: Update streaming callers in `mod.rs`**

`crates/jfmt-core/src/filter/mod.rs` re-exports `compile` and (transitively) is what the CLI uses. The streaming entry points pass through to `compile` indirectly via the CLI; the CLI invokes `compile(expr)` (M4a). To preserve M4a's API:

a) Re-export `Mode` so CLI and tests can name it:

Add to `mod.rs` near the existing `pub use compile::{compile, Compiled};`:

```rust
pub use static_check::Mode;
```

b) The CLI currently calls `compile(&args.expr)`. Update CLI in Task 4 to pass `Mode::Streaming` for non-materialize paths and `Mode::Materialize` for `-m`. **Do NOT** add a thin `compile_streaming(expr)` wrapper — the explicit Mode at every call site is clearer.

c) Re-run grep: `grep -rn "filter::compile\|filter::Compile\b" crates/`. Every hit either is updated by Task 4 (CLI) or is in a test that already calls compile directly — those tests update in Task 4 as well.

- [ ] **Step 7: Verify build + tests**

Run: `cargo build --workspace`
Expected: **build will fail** because `crates/jfmt-cli/src/commands/filter.rs` and any tests that call `compile(expr)` haven't been updated to pass `mode`. That is expected — Task 4 fixes those. **Don't fix them in this commit**; this task lands the static-check restructure only.

To make this task's commit buildable on its own, do this minimal CLI patch in this same task: in `crates/jfmt-cli/src/commands/filter.rs`, find the line `let compiled = compile(&args.expr).map_err(classify_compile_err)?;` and change to `let compiled = compile(&args.expr, jfmt_core::filter::Mode::Streaming).map_err(classify_compile_err)?;`. This keeps M4a behaviour identical until Task 4 wires in the `-m` branch.

Likewise, update the four tests in `compile.rs` per Step 5 above, and fix `static_check.rs`'s in-file tests per Step 2. Those are all the call sites of `compile` and `static_check::check` in the tree.

After these minimal patches:

Run: `cargo build --workspace`
Expected: clean build.

Run: `cargo test --workspace`
Expected: all existing tests still pass; new `static_check::tests::materialize_*` and `compile::tests::*` pass.

- [ ] **Step 8: Clippy**

Run: `cargo clippy --workspace --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 9: Commit**

```bash
git add crates/jfmt-core/src/filter/static_check.rs crates/jfmt-core/src/filter/compile.rs crates/jfmt-core/src/filter/mod.rs crates/jfmt-cli/src/commands/filter.rs
git commit -m "$(cat <<'EOF'
feat(core): split filter static-check into Mode-aware groups

Adds Mode { Streaming, Materialize } to static_check; aggregate names
(length, sort_by, group_by, add, min, max, unique and friends) are
rejected only in Streaming. Multi-input names (input, inputs) are
rejected in both modes via new FilterError::MultiInput. compile()
takes a Mode parameter; existing CLI streaming paths pass
Mode::Streaming to preserve behaviour.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 3: `run_materialize` core function + integration tests

Adds the M4b execution path: read everything → jaq → multi-value stream out.

**Files:**
- Create: `crates/jfmt-core/src/filter/materialize.rs`
- Modify: `crates/jfmt-core/src/filter/mod.rs` (add `pub mod materialize` + re-exports + `BudgetExceeded` variant — see step 1)
- Create: `crates/jfmt-core/tests/filter_materialize.rs`

- [ ] **Step 1: Add `BudgetExceeded` variant**

In `crates/jfmt-core/src/filter/mod.rs`, inside `FilterError`, after `OutputShape`, insert:

```rust
    /// Pre-flight RAM estimate exceeded the safety threshold and
    /// `--force` was not passed.
    #[error(
        "estimated peak memory {estimate_bytes} bytes exceeds 80% of \
         total RAM ({total_ram_bytes} bytes); rerun with --force to override"
    )]
    BudgetExceeded {
        estimate_bytes: u64,
        total_ram_bytes: u64,
    },
```

Add `pub mod materialize;` after the existing `pub mod static_check;`. Add re-export `pub use materialize::{run_materialize, MaterializeReport};` near the other re-exports (after the `pub use compile::{...};` line).

- [ ] **Step 2: Create `materialize.rs` skeleton**

Create `crates/jfmt-core/src/filter/materialize.rs`:

```rust
//! `jfmt filter --materialize`: load whole document, run jaq once,
//! emit a JSON-value stream.

use std::io::{Read, Write};

use serde_json::Value;

use super::runtime::run_one;
use super::{Compiled, FilterError, FilterOptions, FilterOutput};
use crate::event::{Event, Scalar};
use crate::writer::{EventWriter, MinifyWriter, PrettyWriter};

/// Outcome of a materialize run.
#[derive(Debug, Default)]
pub struct MaterializeReport {
    /// Number of jq output values emitted.
    pub outputs_emitted: u64,
    /// If `opts.strict` is false, runtime errors are collected here.
    /// If `strict` is true, the function returns Err on the first one
    /// and this stays empty.
    pub runtime_errors: Vec<FilterError>,
}

/// Drive a materialize run: read everything from `reader` into a
/// `serde_json::Value`, run `compiled` against it, and write the
/// 0/1/N output values as a JSON-value stream to `writer`.
///
/// Output framing:
/// - `Compact`:  values separated by `\n`, no trailing newline.
/// - `Pretty(c)`: values separated by `\n\n`, no trailing newline.
pub fn run_materialize<R: Read, W: Write>(
    reader: R,
    writer: W,
    compiled: &Compiled,
    output: FilterOutput,
    opts: FilterOptions,
) -> Result<MaterializeReport, FilterError> {
    // (1) Load the whole document into a Value.
    let input: Value = serde_json::from_reader(reader).map_err(|e| FilterError::Runtime {
        where_: String::from("(load)"),
        msg: format!("parse: {e}"),
    })?;

    // (2) Run jaq once.
    let outputs = match run_one(compiled, input) {
        Ok(o) => o,
        Err(mut e) => {
            if let FilterError::Runtime { where_: w, .. } = &mut e {
                *w = String::from("(materialize)");
            }
            return Err(e);
        }
    };

    let mut report = MaterializeReport::default();
    report.outputs_emitted = outputs.len() as u64;

    // (3) Write the value stream.
    write_value_stream(writer, &outputs, output, &opts, &mut report)?;
    Ok(report)
}

/// Write `values` as a JSON-value stream. See module docs for framing.
fn write_value_stream<W: Write>(
    mut writer: W,
    values: &[Value],
    output: FilterOutput,
    opts: &FilterOptions,
    report: &mut MaterializeReport,
) -> Result<(), FilterError> {
    let separator: &[u8] = match &output {
        FilterOutput::Compact => b"\n",
        FilterOutput::Pretty(_) => b"\n\n",
    };

    for (i, v) in values.iter().enumerate() {
        if i > 0 {
            writer.write_all(separator)?;
        }
        match write_one_value(&mut writer, v, &output) {
            Ok(()) => {}
            Err(e) => {
                if opts.strict {
                    return Err(e);
                } else {
                    report.runtime_errors.push(e);
                }
            }
        }
    }
    Ok(())
}

fn write_one_value<W: Write>(
    writer: &mut W,
    v: &Value,
    output: &FilterOutput,
) -> Result<(), FilterError> {
    match output {
        FilterOutput::Compact => {
            let mut w = MinifyWriter::new(writer);
            emit_value_events(&mut w, v)?;
            w.finish()?;
            Ok(())
        }
        FilterOutput::Pretty(cfg) => {
            let mut w = PrettyWriter::with_config(writer, *cfg);
            emit_value_events(&mut w, v)?;
            w.finish()?;
            Ok(())
        }
    }
}

/// Emit a Value as a sequence of Events into an EventWriter.
/// Identical contract to filter::output's helper but local so we
/// don't depend on a private item from another module.
fn emit_value_events<W: EventWriter>(writer: &mut W, v: &Value) -> Result<(), FilterError> {
    match v {
        Value::Null => writer.write_event(&Event::Value(Scalar::Null))?,
        Value::Bool(b) => writer.write_event(&Event::Value(Scalar::Bool(*b)))?,
        Value::Number(n) => writer.write_event(&Event::Value(Scalar::Number(n.to_string())))?,
        Value::String(s) => writer.write_event(&Event::Value(Scalar::String(s.clone())))?,
        Value::Array(items) => {
            writer.write_event(&Event::StartArray)?;
            for it in items {
                emit_value_events(writer, it)?;
            }
            writer.write_event(&Event::EndArray)?;
        }
        Value::Object(map) => {
            writer.write_event(&Event::StartObject)?;
            for (k, v) in map {
                writer.write_event(&Event::Name(k.clone()))?;
                emit_value_events(writer, v)?;
            }
            writer.write_event(&Event::EndObject)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filter::compile;
    use crate::filter::static_check::Mode;

    fn run(expr: &str, input: &str) -> (String, MaterializeReport) {
        let compiled = compile(expr, Mode::Materialize).unwrap();
        let mut out = Vec::<u8>::new();
        let report = run_materialize(
            input.as_bytes(),
            &mut out,
            &compiled,
            FilterOutput::Compact,
            FilterOptions::default(),
        )
        .expect("run_materialize");
        (String::from_utf8(out).unwrap(), report)
    }

    #[test]
    fn length_returns_array_size() {
        let (out, _) = run("length", "[1,2,3]");
        assert_eq!(out, "3");
    }

    #[test]
    fn identity_passes_value() {
        let (out, _) = run(".", "{\"x\":1}");
        // serde_json may sort the keys; we only assert the value
        // round-trips via Value -> Value.
        assert!(out.contains(r#""x":1"#));
    }

    #[test]
    fn iterate_emits_value_stream() {
        let (out, report) = run(".[]", "[1,2,3]");
        assert_eq!(out, "1\n2\n3");
        assert_eq!(report.outputs_emitted, 3);
    }

    #[test]
    fn empty_output_writes_nothing() {
        let (out, report) = run(".[] | select(. > 100)", "[1,2,3]");
        assert_eq!(out, "");
        assert_eq!(report.outputs_emitted, 0);
    }

    #[test]
    fn single_value_no_separator() {
        let (out, report) = run(".x", r#"{"x":42}"#);
        assert_eq!(out, "42");
        assert_eq!(report.outputs_emitted, 1);
    }

    #[test]
    fn sort_by_works() {
        let (out, _) = run(
            "sort_by(.x) | .[].x",
            r#"[{"x":3},{"x":1},{"x":2}]"#,
        );
        assert_eq!(out, "1\n2\n3");
    }
}
```

- [ ] **Step 3: Run unit tests**

Run: `cargo test -p jfmt-core --lib filter::materialize`
Expected: 6 tests pass.

If any test fails because `MinifyWriter::finish` requires the writer to have been used (e.g. complains about empty event stream), check the `EventWriter` trait — `write_event` then `finish` should work for any non-empty value, and our values always go through `write_one_value` which does at least one `write_event`. If `finish` panics on the empty-output case (where no value is emitted), that's fine because the for-loop never enters.

- [ ] **Step 4: Create the integration test**

Create `crates/jfmt-core/tests/filter_materialize.rs`:

```rust
use jfmt_core::filter::static_check::Mode;
use jfmt_core::filter::{
    compile, run_materialize, FilterError, FilterOptions, FilterOutput, PrettyConfig,
};

fn run(expr: &str, input: &str) -> String {
    let compiled = compile(expr, Mode::Materialize).expect("compile");
    let mut out = Vec::<u8>::new();
    run_materialize(
        input.as_bytes(),
        &mut out,
        &compiled,
        FilterOutput::Compact,
        FilterOptions::default(),
    )
    .expect("run_materialize");
    String::from_utf8(out).unwrap()
}

#[test]
fn group_by_returns_grouped_array() {
    let s = run(
        "group_by(.k)",
        r#"[{"k":"a","v":1},{"k":"b","v":2},{"k":"a","v":3}]"#,
    );
    // Two groups: [{k:a,v:1},{k:a,v:3}] and [{k:b,v:2}].
    assert!(s.starts_with('[') && s.ends_with(']'));
    assert!(s.contains(r#""k":"a","v":1"#) || s.contains(r#""v":1,"k":"a""#));
    assert!(s.contains(r#""k":"b","v":2"#) || s.contains(r#""v":2,"k":"b""#));
}

#[test]
fn add_sums_array() {
    let s = run("add", "[1,2,3,4]");
    assert_eq!(s, "10");
}

#[test]
fn unique_dedupes() {
    let s = run("unique", "[3,1,2,1,3]");
    assert_eq!(s, "[1,2,3]");
}

#[test]
fn pretty_uses_double_newline_separator() {
    let compiled = compile(".[]", Mode::Materialize).unwrap();
    let mut out = Vec::<u8>::new();
    let cfg = PrettyConfig {
        indent: 2,
        ..PrettyConfig::default()
    };
    run_materialize(
        "[1,2,3]".as_bytes(),
        &mut out,
        &compiled,
        FilterOutput::Pretty(cfg),
        FilterOptions::default(),
    )
    .unwrap();
    let s = String::from_utf8(out).unwrap();
    // Expect `1\n\n2\n\n3` for scalars (PrettyWriter formats scalars
    // unchanged). Trailing newline absent.
    assert_eq!(s, "1\n\n2\n\n3");
}

#[test]
fn type_error_default_collected() {
    let compiled = compile(".x + 1", Mode::Materialize).unwrap();
    let mut out = Vec::<u8>::new();
    let err = run_materialize(
        r#"{"x":"hi"}"#.as_bytes(),
        &mut out,
        &compiled,
        FilterOutput::Compact,
        FilterOptions::default(),
    )
    .unwrap_err();
    assert!(matches!(err, FilterError::Runtime { .. }));
}

#[test]
fn type_error_strict_returns_err() {
    let compiled = compile(".x + 1", Mode::Materialize).unwrap();
    let mut out = Vec::<u8>::new();
    let err = run_materialize(
        r#"{"x":"hi"}"#.as_bytes(),
        &mut out,
        &compiled,
        FilterOutput::Compact,
        FilterOptions { strict: true },
    )
    .unwrap_err();
    assert!(matches!(err, FilterError::Runtime { .. }));
}
```

Note: the difference between "default" and "strict" runtime error in materialize mode is subtle: jaq fails when evaluating the filter against the entire document — there is only ever one input value, so there's only one chance to error. Both default and strict therefore propagate the error up to the caller (we can't "skip and continue" a one-shot run). The two tests above intentionally produce identical outcomes; this documents the design rather than verifying a behavioural difference. The strict flag's real effect in materialize mode is on the Pretty/Compact write path (Pretty hitting an event-writer error) — covered by separate tests if needed.

If the test "fails" because both branches return the same Err shape, that's intentional. Both tests should pass.

Re-export `PrettyConfig` from `jfmt_core::filter` so the integration test can name it directly. Either:

- Add `pub use crate::writer::PrettyConfig;` to `crates/jfmt-core/src/filter/mod.rs`, OR
- In the integration test, import via `jfmt_core::PrettyConfig` (which is re-exported at crate root in `lib.rs`).

Choose whichever already compiles; if the latter works without changes, prefer it.

- [ ] **Step 5: Run integration tests**

Run: `cargo test -p jfmt-core --test filter_materialize`
Expected: 6 tests pass.

- [ ] **Step 6: Run full workspace**

Run: `cargo test --workspace`
Expected: every prior test still passes; new tests added.

- [ ] **Step 7: Clippy**

Run: `cargo clippy --workspace --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 8: Commit**

```bash
git add crates/jfmt-core/src/filter/materialize.rs crates/jfmt-core/src/filter/mod.rs crates/jfmt-core/tests/filter_materialize.rs
git commit -m "$(cat <<'EOF'
feat(core): add run_materialize (full-jq mode, multi-value stream)

Loads the whole input via serde_json::from_reader, runs the compiled
jaq filter once against the resulting Value, and writes 0/1/N output
values as a JSON-value stream. Compact mode separates with '\n';
pretty mode separates with '\n\n'. No trailing newline. Adds
FilterError::BudgetExceeded variant (CLI consumes in Task 4).

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 4: CLI flags + RAM budget pre-flight

**Files:**
- Modify: `crates/jfmt-cli/src/cli.rs`
- Modify: `crates/jfmt-cli/src/commands/filter.rs`
- Modify: `crates/jfmt-cli/src/main.rs`

- [ ] **Step 1: Extend `FilterArgs`**

In `crates/jfmt-cli/src/cli.rs`, find `pub struct FilterArgs` and add two fields after the existing `pub strict` field:

```rust
    /// Materialize the whole input and run with full jq semantics
    /// (allows length, sort_by, group_by, etc.). Conflicts with
    /// --ndjson.
    #[arg(short = 'm', long = "materialize", conflicts_with = "ndjson")]
    pub materialize: bool,

    /// Skip the RAM budget pre-flight check. Only meaningful with
    /// --materialize.
    #[arg(long = "force", requires = "materialize")]
    pub force: bool,
```

Place these BEFORE the existing `pretty` / `compact` / `indent` fields so the help output groups mode flags together. (Cosmetic, but matches the existing CLI's grouping style.)

- [ ] **Step 2: Add a pure budget helper**

In `crates/jfmt-cli/src/commands/filter.rs`, add at the bottom of the file:

```rust
/// Estimate peak RAM for materializing `input`. Returns `None` when
/// the input is stdin or its size can't be determined — callers
/// interpret `None` as "skip the check" per spec D3.
fn estimate_peak_ram_bytes(spec: &jfmt_io::InputSpec) -> Option<u64> {
    let path = spec.path.as_ref()?;
    let meta = std::fs::metadata(path).ok()?;
    let on_disk = meta.len();
    // Effective compression: explicit override, then file extension.
    let compression = spec
        .compression
        .unwrap_or_else(|| jfmt_io::Compression::from_path(path));
    let multiplier: u64 = match compression {
        jfmt_io::Compression::None => 6,
        jfmt_io::Compression::Gzip | jfmt_io::Compression::Zstd => 5 * 6, // 30
    };
    Some(on_disk.saturating_mul(multiplier))
}

/// Pure predicate: is `estimate` under 80% of `total_ram`?
fn budget_ok(estimate: u64, total_ram: u64) -> bool {
    // 80% = total_ram * 4 / 5. Compute as `total_ram / 5 * 4` to
    // reduce overflow risk on very large `total_ram` values.
    estimate < total_ram / 5 * 4
}

/// Query the actual system total RAM. Wraps sysinfo per spec Annex B.
fn system_total_ram_bytes() -> u64 {
    let mut sys = sysinfo::System::new();
    sys.refresh_memory();
    sys.total_memory()
}

#[cfg(test)]
mod tests {
    use super::budget_ok;

    #[test]
    fn budget_ok_under_80_percent() {
        // 1 GiB on a 2 GiB machine = 50% < 80% → ok.
        assert!(budget_ok(1 << 30, 2u64 << 30));
    }

    #[test]
    fn budget_not_ok_over_80_percent() {
        // 1.7 GiB on a 2 GiB machine ≈ 85% → not ok.
        let total = 2u64 << 30;
        let estimate = total * 85 / 100;
        assert!(!budget_ok(estimate, total));
    }

    #[test]
    fn budget_ok_at_zero_estimate() {
        assert!(budget_ok(0, 1 << 30));
    }
}
```

If Annex B documented a different sysinfo entry sequence (e.g., `System::new_all()` instead of `new() + refresh_memory()`), update `system_total_ram_bytes` to match. The code shape is otherwise stable.

- [ ] **Step 3: Add the materialize branch to `run`**

In `crates/jfmt-cli/src/commands/filter.rs`, find the existing `pub fn run(args: FilterArgs, threads: usize) -> Result<()>` function. Replace its body with:

```rust
pub fn run(args: FilterArgs, threads: usize) -> Result<()> {
    use jfmt_core::filter::Mode;

    if args.pretty && args.common.ndjson {
        return Err(anyhow::anyhow!("--pretty conflicts with --ndjson"));
    }

    // Mode pick: --materialize chooses Materialize, otherwise Streaming.
    // Note: --ndjson is its own runtime flavour (run_ndjson) but it
    // still compiles in Streaming mode (per-line full semantics happen
    // because each line IS a full document).
    let mode = if args.materialize {
        Mode::Materialize
    } else {
        Mode::Streaming
    };
    let compiled = compile(&args.expr, mode).map_err(classify_compile_err)?;

    let opts = FilterOptions { strict: args.strict };
    let input_spec = args.common.input_spec();

    if args.materialize {
        // RAM budget pre-flight (file inputs only).
        if !args.force {
            if let Some(estimate) = estimate_peak_ram_bytes(&input_spec) {
                let total = system_total_ram_bytes();
                if !budget_ok(estimate, total) {
                    return Err(SilentExit(ExitCode::InputError).into());
                }
            }
        }

        let input = jfmt_io::open_input(&input_spec).context("opening input")?;
        let output = jfmt_io::open_output(&args.common.output_spec()).context("opening output")?;
        let out_choice = if args.pretty {
            let cfg = PrettyConfig { indent: args.indent, ..PrettyConfig::default() };
            FilterOutput::Pretty(cfg)
        } else {
            FilterOutput::Compact
        };
        match run_materialize(input, output, &compiled, out_choice, opts) {
            Ok(_report) => Ok(()),
            Err(e) => Err(classify_runtime_err(e, args.strict)),
        }
    } else if args.common.ndjson {
        let input = jfmt_io::open_input(&input_spec).context("opening input")?;
        let output = jfmt_io::open_output(&args.common.output_spec()).context("opening output")?;
        let report = run_ndjson(input, output, compiled, threads, opts)
            .context("filter NDJSON pipeline")?;
        for (line, e) in &report.errors {
            eprintln!("error: line {line}: {}", e.message);
        }
        if args.strict && !report.errors.is_empty() {
            return Err(SilentExit(ExitCode::InputError).into());
        }
        Ok(())
    } else {
        if !HINT_PRINTED.swap(true, std::sync::atomic::Ordering::Relaxed) {
            eprintln!(
                "note: streaming mode evaluates your expression once per top-level element."
            );
            eprintln!(
                "      write '.id' not '.[].id'  (use --ndjson for full per-line jq semantics)"
            );
        }
        let input = jfmt_io::open_input(&input_spec).context("opening input")?;
        let output = jfmt_io::open_output(&args.common.output_spec()).context("opening output")?;
        let out_choice = if args.pretty {
            let cfg = PrettyConfig { indent: args.indent, ..PrettyConfig::default() };
            FilterOutput::Pretty(cfg)
        } else {
            FilterOutput::Compact
        };
        let report = run_streaming(input, output, &compiled, out_choice, opts)
            .map_err(|e| classify_runtime_err(e, args.strict))?;
        for e in &report.runtime_errors {
            eprintln!("error: {e}");
        }
        if args.strict && !report.runtime_errors.is_empty() {
            return Err(SilentExit(ExitCode::InputError).into());
        }
        Ok(())
    }
}
```

The pre-flight emits a stderr message immediately before returning the SilentExit. Insert this just before `return Err(SilentExit(...).into());`:

```rust
                    eprintln!(
                        "jfmt: estimated peak memory {} bytes exceeds 80% of total RAM ({} bytes); rerun with --force to override",
                        estimate, total
                    );
```

- [ ] **Step 4: Update imports in filter.rs**

At the top of `crates/jfmt-cli/src/commands/filter.rs`, ensure these imports exist:

```rust
use crate::cli::FilterArgs;
use crate::exit::ExitCode;
use crate::SilentExit;
use anyhow::{Context, Result};
use jfmt_core::filter::{
    compile, run_materialize, run_ndjson, run_streaming, FilterError, FilterOptions, FilterOutput,
};
use jfmt_core::PrettyConfig;
use std::sync::atomic::{AtomicBool, Ordering};
```

The `Mode` import is inside the `run` function via `use jfmt_core::filter::Mode;` per Step 3.

- [ ] **Step 5: Update classify_compile_err to handle MultiInput**

Find `fn classify_compile_err` and update:

```rust
fn classify_compile_err(e: FilterError) -> anyhow::Error {
    eprintln!("jfmt: {e}");
    SilentExit(match &e {
        FilterError::Aggregate { .. }
        | FilterError::Parse { .. }
        | FilterError::MultiInput { .. } => ExitCode::SyntaxError,
        _ => ExitCode::InputError,
    })
    .into()
}
```

- [ ] **Step 6: Update main.rs classify**

In `crates/jfmt-cli/src/main.rs`, update the `classify` function:

```rust
fn classify(e: &anyhow::Error) -> ExitCode {
    if let Some(core_err) = e.downcast_ref::<jfmt_core::Error>() {
        if matches!(core_err, jfmt_core::Error::Syntax { .. }) {
            return ExitCode::SyntaxError;
        }
    }
    if let Some(filt) = e.downcast_ref::<jfmt_core::FilterError>() {
        if matches!(
            filt,
            jfmt_core::FilterError::Parse { .. }
                | jfmt_core::FilterError::Aggregate { .. }
                | jfmt_core::FilterError::MultiInput { .. }
        ) {
            return ExitCode::SyntaxError;
        }
    }
    ExitCode::InputError
}
```

- [ ] **Step 7: Build + smoke-test**

Run: `cargo build -p jfmt-cli`
Expected: clean.

Smoke (paste each output verbatim):

```bash
# Aggregate now passes in -m
echo '[1,2,3]' | cargo run -q -p jfmt-cli -- filter -m 'length'
# Expected stdout: 3

# Multi-output stream
echo '[1,2,3]' | cargo run -q -p jfmt-cli -- filter -m '.[]'
# Expected stdout: 1
#                  2
#                  3
# (no trailing newline)

# Sort_by works in -m
echo '[{"x":3},{"x":1},{"x":2}]' | cargo run -q -p jfmt-cli -- filter -m 'sort_by(.x)'
# Expected stdout: [{"x":1},{"x":2},{"x":3}]

# Conflicts: -m and --ndjson
echo '[]' | cargo run -q -p jfmt-cli -- filter -m --ndjson '.'; echo "exit=$?"
# Expected: exit=2 (clap usage error)

# --force without -m
echo '[]' | cargo run -q -p jfmt-cli -- filter --force '.'; echo "exit=$?"
# Expected: exit=2 (clap requires)

# Multi-input rejected in both modes
echo '[]' | cargo run -q -p jfmt-cli -- filter -m 'input'; echo "exit=$?"
# Expected stderr: "uses 'input'; jfmt does not support jq multi-document streams. Use --ndjson..."
# Expected: exit=2
```

If any smoke test fails, debug before committing. Don't proceed past Step 7 with a failing smoke.

- [ ] **Step 8: Run all tests**

Run: `cargo test --workspace`
Expected: all green (M4a tests + new core tests).

- [ ] **Step 9: Clippy**

Run: `cargo clippy --workspace --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 10: Commit**

```bash
git add crates/jfmt-cli/src/cli.rs crates/jfmt-cli/src/commands/filter.rs crates/jfmt-cli/src/main.rs
git commit -m "$(cat <<'EOF'
feat(cli): add 'jfmt filter -m' (--materialize) with RAM budget check

Adds --materialize / -m and --force flags. Materialize mode loads the
whole input, runs jaq with full semantics, and emits a JSON-value
stream. File inputs trigger a pre-flight RAM check (peak = file_size *
6, * 5 again for compressed) against 80% of system RAM via sysinfo;
abort with exit 1 unless --force. stdin inputs skip the check.
--ndjson and -m are clap-mutually-exclusive; --force requires -m.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 5: CLI e2e tests

Append filter materialize coverage to `crates/jfmt-cli/tests/cli_filter.rs`. Avoid OOM tests that depend on machine size; cover happy paths, conflicts, and the `--force` override.

**Files:**
- Modify: `crates/jfmt-cli/tests/cli_filter.rs`
- Create: `crates/jfmt-cli/tests/fixtures/filter_materialize_array.json`

- [ ] **Step 1: Fixture**

Create `crates/jfmt-cli/tests/fixtures/filter_materialize_array.json`:

```json
[{"x":3,"name":"c"},{"x":1,"name":"a"},{"x":2,"name":"b"}]
```

- [ ] **Step 2: Append tests to `cli_filter.rs`**

Append at the end of `crates/jfmt-cli/tests/cli_filter.rs`:

```rust
// ===== M4b — --materialize =====

#[test]
fn materialize_length_returns_count() {
    jfmt()
        .args([
            "filter",
            "-m",
            "length",
            "tests/fixtures/filter_materialize_array.json",
        ])
        .assert()
        .success()
        .stdout("3");
}

#[test]
fn materialize_sort_by_returns_sorted() {
    jfmt()
        .args([
            "filter",
            "-m",
            "sort_by(.x) | .[].x",
            "tests/fixtures/filter_materialize_array.json",
        ])
        .assert()
        .success()
        .stdout("1\n2\n3");
}

#[test]
fn materialize_iterate_emits_value_stream() {
    jfmt()
        .args(["filter", "-m", ".[]"])
        .write_stdin("[1,2,3]")
        .assert()
        .success()
        .stdout("1\n2\n3");
}

#[test]
fn materialize_aggregate_no_longer_rejected() {
    // With --materialize, length/group_by/etc. should compile cleanly.
    jfmt()
        .args(["filter", "-m", "group_by(.k)"])
        .write_stdin(r#"[{"k":"a"},{"k":"b"},{"k":"a"}]"#)
        .assert()
        .success();
}

#[test]
fn materialize_input_still_rejected() {
    jfmt()
        .args(["filter", "-m", "input"])
        .write_stdin("[]")
        .assert()
        .code(2)
        .stderr(predicate::str::contains("multi-document streams"));
}

#[test]
fn materialize_conflicts_with_ndjson() {
    jfmt()
        .args(["filter", "-m", "--ndjson", "."])
        .write_stdin("{}")
        .assert()
        .code(2);
}

#[test]
fn force_requires_materialize() {
    jfmt()
        .args(["filter", "--force", "."])
        .write_stdin("{}")
        .assert()
        .code(2);
}

#[test]
fn materialize_stdin_skips_budget_check() {
    // stdin has no known size; the pre-flight should be a no-op
    // regardless of machine RAM.
    jfmt()
        .args(["filter", "-m", "."])
        .write_stdin(r#"{"x":1}"#)
        .assert()
        .success()
        .stderr(predicate::str::contains("exceeds").not());
}

#[test]
fn materialize_pretty_uses_blank_line_separator() {
    jfmt()
        .args(["filter", "-m", "--pretty", ".[]"])
        .write_stdin("[1,2,3]")
        .assert()
        .success()
        .stdout("1\n\n2\n\n3");
}
```

Note for `materialize_pretty_uses_blank_line_separator`: PrettyWriter emits scalars without indentation, so the output for each value is just the scalar literal. The separator is `\n\n`, no trailing newline. If PrettyWriter's actual scalar output differs (e.g., adds a leading newline), inspect and adjust the assertion.

- [ ] **Step 3: Run**

Run: `cargo test -p jfmt-cli --test cli_filter`
Expected: previous 9 + new 9 = 18 tests pass. If any test inspects exit codes that should be 2 but are 1, the cause is usually that classify in main.rs is missing the variant — re-check Task 4 Step 6.

- [ ] **Step 4: Run all tests**

Run: `cargo test --workspace`
Expected: all green.

- [ ] **Step 5: Clippy**

Run: `cargo clippy --workspace --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add crates/jfmt-cli/tests/cli_filter.rs crates/jfmt-cli/tests/fixtures/filter_materialize_array.json
git commit -m "$(cat <<'EOF'
test(cli): cover filter --materialize happy path, conflicts, stdin

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 6: README + spec milestone update

**Files:**
- Modify: `README.md`
- Modify: `docs/superpowers/specs/2026-04-23-jfmt-phase1-design.md`

- [ ] **Step 1: Update README Status**

Replace the existing `## Status` block with:

```markdown
## Status

**M4b preview (v0.0.5)** — `pretty`, `minify`, `validate`, and
`filter` (streaming + NDJSON parallel + `--materialize` for full-jq
semantics with RAM budget pre-flight). See
[`docs/superpowers/specs/2026-04-23-jfmt-phase1-design.md`](docs/superpowers/specs/2026-04-23-jfmt-phase1-design.md)
for the Phase 1 roadmap.
```

- [ ] **Step 2: Append a `--materialize` block to the README's `### Filter` section**

Find the existing `### Filter` section. After the streaming-mode rules and before the "Object output keys are emitted in alphabetical order…" paragraph (or at a natural ordering point), insert:

```markdown
**Full-jq mode (`-m` / `--materialize`):**

```bash
# Allows aggregates: length, sort_by, group_by, add, min, max, unique
jfmt filter -m 'sort_by(.x)' file.json
jfmt filter -m 'length' file.json
jfmt filter -m '.[]' file.json   # multi-value stream output

# Override the RAM budget check (file_size * 6, file_size * 30 if compressed)
jfmt filter -m --force 'sort_by(.x)' big.json
```

`-m` loads the entire input into memory and runs the jq expression
with full semantics. For file inputs, jfmt estimates peak memory and
aborts unless `--force` is set or the estimate is under 80 % of total
RAM. stdin input skips the check (the `-m` flag itself is the user's
"I have enough memory" promise). `-m` and `--ndjson` are mutually
exclusive.

Multiple jq output values are emitted as a JSON-value stream
(separated by `\n`, or `\n\n` with `--pretty`). No trailing newline,
matching jfmt's `pretty` / `minify` output. This intentionally
differs from `jq -c`'s trailing newline.
```

(Mind the inner code-fence — preserve as written.)

- [ ] **Step 3: Update Phase 1 spec milestone status**

In `docs/superpowers/specs/2026-04-23-jfmt-phase1-design.md`, find the milestone table and the shipped-status section that Task 13 of M4a updated. Mark M4b shipped:

- In the milestone table row for M4b: change "not started" → `shipped (v0.0.5, 2026-04-25)`.
- Append a "M4b ✓ Shipped v0.0.5 on 2026-04-25" line in the shipped-status block, mirroring the M4a entry.

Don't change M4a or earlier rows.

- [ ] **Step 4: Final test sweep**

Run: `cargo test --workspace`
Expected: all green.

Run: `cargo clippy --workspace --all-targets -- -D warnings`
Expected: clean.

Run: `cargo fmt --all -- --check`
Expected: clean. If drift, run `cargo fmt --all`, **mention it in your report**, and either fold into Task 7's commit or commit separately as a `chore: rustfmt` follow-up.

- [ ] **Step 5: Commit**

```bash
git add README.md docs/superpowers/specs/2026-04-23-jfmt-phase1-design.md
git commit -m "$(cat <<'EOF'
docs: document jfmt filter --materialize and mark M4b shipped

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 7: Ship `v0.0.5`

**Files:**
- Modify: `Cargo.toml` (workspace)

- [ ] **Step 1: Bump version**

In `Cargo.toml` (workspace), change:

```toml
version = "0.0.4"
```

to:

```toml
version = "0.0.5"
```

- [ ] **Step 2: Verify**

Run: `cargo build --workspace`
Expected: clean.

Run: `cargo test --workspace`
Expected: all green.

- [ ] **Step 3: Commit + tag**

```bash
git add Cargo.toml Cargo.lock
git commit -m "$(cat <<'EOF'
chore: bump version to 0.0.5 (M4b — filter --materialize)

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
git tag -a v0.0.5 -m "M4b: filter --materialize with RAM budget"
```

---

## Self-Review Checklist (Performed)

**Spec coverage:**
- §1 Scope (`-m`, multi-value stream, RAM check, `--force`, stdin skip, `--ndjson` conflict) → Tasks 3, 4, 5 collectively.
- §2 D1 multi-value stream → Task 3 `write_value_stream`.
- §2 D2 split blacklist → Task 2.
- §2 D3 stdin skips check → Task 4 `estimate_peak_ram_bytes` returns None for stdin.
- §2 D4 no trailing newline → Task 3 `write_value_stream` (separator BETWEEN values; loop emits no terminator after the last).
- §3 Module layout → Task 3 (materialize.rs) + Task 2 (static_check) + Task 4 (CLI changes).
- §4 Data flow → Task 3.
- §4.3 RAM budget → Task 4 (`estimate_peak_ram_bytes`, `budget_ok`, `system_total_ram_bytes`).
- §5 Errors and exit codes → Task 2 (MultiInput), Task 3 (BudgetExceeded), Task 4 (CLI mapping).
- §6 CLI → Task 4.
- §7.1 unit tests (static_check Mode + materialize 0/1/N) → Tasks 2, 3.
- §7.2 integration tests → Task 3.
- §7.3 e2e → Task 5.
- §8 Risks: sysinfo MSRV → Task 1; `6×` heuristic → coded as constant in Task 4 (single source of truth, easy to bump); no-trailing-newline → README in Task 6.
- §10 Acceptance: `cargo test`/`clippy` → Task 6 sweep; tag → Task 7.

**Placeholder scan:** Task 1 leaves `<X.Y.Z>` for sysinfo deliberately — frozen by spike, with Annex B as the load-bearing contract for Task 4. No "TBD" / "TODO" elsewhere.

**Type consistency:** `Mode { Streaming, Materialize }` defined Task 2, used Tasks 2/3/4. `compile(expr, mode)` signature matches at all call sites (Tasks 2/3/4 + tests). `MaterializeReport.outputs_emitted` defined Task 3, not externally consumed in Tasks 4-7 (CLI ignores it). `FilterError::MultiInput` defined Task 2; consumed Task 4 (CLI classify) and tested Tasks 2/5. `FilterError::BudgetExceeded` defined Task 3; produced Task 4 (CLI). `estimate_peak_ram_bytes` returns `Option<u64>` consistently.
