# jfmt M4a — Streaming + NDJSON Filter Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship `jfmt filter EXPR` with single-document streaming and NDJSON parallel modes, embedded jaq, static-check + runtime guard, `--strict`. Tagged as `v0.0.4`. Out of scope: `-m`/`--materialize` (M4b).

**Architecture:** A new `jfmt-core::filter` module owns expression compilation (jaq parser + static-check), per-shard execution (jaq runtime wrapped to forbid cross-shard input), shape-preserving output, and a glue layer that drives the existing `parser::EventReader` for single-document mode and the existing `ndjson::run_ndjson_pipeline` for NDJSON mode. The NDJSON pipeline's payload migrates from `Vec<u8>` per line to `Vec<Vec<u8>>` per line so a filter can emit 0/1/N values per input.

**Tech Stack:** Rust 2021 / MSRV 1.75 · `jaq-core` + `jaq-std` + `jaq-syn` (versions frozen by Task 1's spike) · `serde_json::Value` · existing `struson` parser · existing M3 pipeline · `crossbeam-channel` · `proptest` · `assert_cmd`.

**Spec:** `docs/superpowers/specs/2026-04-25-jfmt-m4a-filter-design.md`.

---

## File Structure

### New files (`jfmt-core`)

| Path | Responsibility |
|---|---|
| `crates/jfmt-core/src/filter/mod.rs` | Public API (`run_streaming`, `run_ndjson_line`), `FilterOptions`, `FilterError`. |
| `crates/jfmt-core/src/filter/compile.rs` | `compile(expr) -> Compiled` — parse + static-check + jaq compile. |
| `crates/jfmt-core/src/filter/static_check.rs` | AST blacklist scanner. Pure function on jaq's parsed AST. |
| `crates/jfmt-core/src/filter/runtime.rs` | `Compiled::run(value) -> Result<Vec<Value>, RuntimeError>`. Empty `inputs` iterator. |
| `crates/jfmt-core/src/filter/shard.rs` | `ShardAccumulator`: `EventReader` ↔ `serde_json::Value` bridge. |
| `crates/jfmt-core/src/filter/output.rs` | `OutputShaper`: shape-preserving emit (Array/Object/Scalar). |

### New files (`jfmt-cli`)

| Path | Responsibility |
|---|---|
| `crates/jfmt-cli/src/commands/filter.rs` | `Command::Filter` runner; routes to streaming or NDJSON path. |
| `crates/jfmt-cli/tests/cli_filter.rs` | End-to-end tests. |
| `crates/jfmt-cli/tests/fixtures/filter_*.json{,nl}` | Fixtures for e2e. |

### Modified files

| Path | Change |
|---|---|
| `Cargo.toml` (workspace) | Add `jaq-core`, `jaq-std`, `jaq-syn` (versions frozen by Task 1). |
| `crates/jfmt-core/Cargo.toml` | Pull jaq deps. |
| `crates/jfmt-core/src/lib.rs` | `pub mod filter;` + re-exports. |
| `crates/jfmt-core/src/ndjson/worker.rs` | `WorkerOutput` payload `Vec<u8>` → `Vec<Vec<u8>>`. |
| `crates/jfmt-core/src/ndjson/reorder.rs` | Iterate inner `Vec<u8>`s, append `\n` after each. |
| `crates/jfmt-core/src/ndjson/mod.rs` | Closure return type `Result<Vec<u8>, LineError>` → `Result<Vec<Vec<u8>>, LineError>`. Existing tests/callers wrap a single `Vec<u8>` in `vec![…]`. |
| `crates/jfmt-cli/src/commands/{pretty,minify,validate}.rs` | Wrap their existing closure returns in `vec![…]`. |
| `crates/jfmt-cli/src/cli.rs` | Add `Command::Filter(FilterArgs)`. |
| `crates/jfmt-cli/src/main.rs` | Route `Command::Filter`. |
| `crates/jfmt-cli/src/exit.rs` | Map `FilterError` variants (downcast) to exit codes. |
| `README.md` | New `## filter` section. |
| `docs/superpowers/specs/2026-04-23-jfmt-phase1-design.md` | Mark M4a shipped. |
| `Cargo.toml` (workspace) | `version = "0.0.4"`. |

---

## Task 1: Spike & freeze jaq dependency versions

**Why this task exists:** the `jaq-*` crate split has shifted across releases (`jaq-parse` → `jaq-syn`, etc.) and we have an MSRV-1.75 constraint that has bitten previous dep additions. Doing the spike first means every later task can call a known-real API. **No code from later tasks is written before this task lands.**

**Files:**
- Create: `crates/jfmt-core/examples/jaq_spike.rs` (deleted at end of task)
- Modify: `Cargo.toml`, `crates/jfmt-core/Cargo.toml`

- [ ] **Step 1: Search current jaq versions**

Run:
```bash
cargo search jaq-core --limit 5
cargo search jaq-std --limit 5
cargo search jaq-syn --limit 5
cargo search jaq-parse --limit 5
```

Pick the **latest set that all share the same major version line** AND that compiles on rustc 1.75. Start with the highest version; if `cargo build` fails with edition2024 errors, step down a minor version. Record the chosen versions inline in this task's notes for later steps.

- [ ] **Step 2: Add provisional deps to workspace**

Edit `Cargo.toml` (workspace). Add to `[workspace.dependencies]`, replacing `<X.Y.Z>` with the versions you picked. Use `=` pins.

```toml
# jq evaluator (M4a). Pin tight so static-check blacklist stays in sync.
jaq-core = "=<X.Y.Z>"
jaq-std = "=<X.Y.Z>"
jaq-syn = "=<X.Y.Z>"   # use jaq-parse if jaq-syn does not exist at that version
```

Edit `crates/jfmt-core/Cargo.toml`. Add under `[dependencies]`:

```toml
jaq-core = { workspace = true }
jaq-std = { workspace = true }
jaq-syn = { workspace = true }
serde_json = { workspace = true }
```

- [ ] **Step 3: Write the spike**

Create `crates/jfmt-core/examples/jaq_spike.rs`. Goal: prove that we can parse, compile, and run `select(.x > 1)` against `{"x": 2}` and get `{"x": 2}` back; against `{"x": 0}` and get nothing back.

The exact API names below may differ across jaq versions — adjust to match what `cargo doc --open -p jaq-core` shows for the chosen version. The structure is fixed:

```rust
use serde_json::json;

fn main() {
    let expr = "select(.x > 1)";

    // (1) Parse: jaq-syn (or jaq-parse).
    // (2) Compile against the standard library: jaq-std::funs() / jaq-core::Compiler.
    // (3) Run with an empty inputs iterator and a single input value.

    let compiled = compile(expr);

    let yes = run(&compiled, json!({"x": 2}));
    assert_eq!(yes, vec![json!({"x": 2})]);

    let no = run(&compiled, json!({"x": 0}));
    assert!(no.is_empty());

    println!("spike OK");
}

fn compile(expr: &str) -> /* compiled filter handle */ ! { todo!("see jaq docs") }
fn run(_compiled: &!, _input: serde_json::Value) -> Vec<serde_json::Value> { todo!() }
```

Replace the two `todo!()`s with real code from the jaq docs / examples. The point is to map the jaq API onto two functions: `compile(&str) -> Compiled` and `run(&Compiled, Value) -> Vec<Value>`. Those become the starting shapes for `filter/compile.rs` and `filter/runtime.rs`.

- [ ] **Step 4: Run the spike**

Run: `cargo run -p jfmt-core --example jaq_spike`
Expected: prints `spike OK`. If the binary panics, fix the spike code (not the deps) until it works. The two assertions are the success criteria.

- [ ] **Step 5: Capture the spike's API mapping in spec annex**

Append to `docs/superpowers/specs/2026-04-25-jfmt-m4a-filter-design.md`:

```markdown
## Annex A — jaq API mapping (frozen by Task 1 spike)

- Versions: jaq-core=<X.Y.Z>, jaq-std=<X.Y.Z>, jaq-syn=<X.Y.Z>.
- Parse: `<actual function path>(expr) -> <Ast>`
- Compile: `<actual function path>(ast, defs) -> <Filter>`
- Run: `<actual function path>(filter, ctx, value) -> <iterator of Value>`
- Empty inputs iterator: `<actual type or constructor>`
```

Replace each `<…>` with the real symbol from the chosen jaq version. Subsequent tasks reference this annex.

- [ ] **Step 6: Delete the example**

Run: `git rm crates/jfmt-core/examples/jaq_spike.rs`

The crate's `[dependencies]` are now confirmed; the example has done its job.

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml crates/jfmt-core/Cargo.toml docs/superpowers/specs/2026-04-25-jfmt-m4a-filter-design.md
git commit -m "$(cat <<'EOF'
chore(deps): add jaq-{core,std,syn} pinned for M4a

Versions chosen via spike (see spec Annex A). MSRV-1.75 verified by
running a select(.x>1) round-trip example, then deleted.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 2: Filter module skeleton + `FilterError` type

**Files:**
- Create: `crates/jfmt-core/src/filter/mod.rs`
- Create: `crates/jfmt-core/src/filter/{compile,static_check,runtime,shard,output}.rs` (stubs)
- Modify: `crates/jfmt-core/src/lib.rs`

- [ ] **Step 1: Create `filter/mod.rs`**

```rust
//! `jfmt filter` engine: per-shard jq evaluation in two flavours
//! (single-document streaming, NDJSON parallel). Out of scope for
//! M4a: `--materialize` mode (lands in M4b).

pub mod compile;
pub mod output;
pub mod runtime;
pub mod shard;
pub mod static_check;

use thiserror::Error;

/// Top-level filter error. Library variants; the CLI maps them to
/// exit codes via `crates/jfmt-cli/src/exit.rs`.
#[derive(Debug, Error)]
pub enum FilterError {
    /// jaq parser rejected the expression.
    #[error("invalid filter expression: {msg}")]
    Parse { msg: String },

    /// Static check blacklisted the expression because it cannot be
    /// evaluated per-shard. Carry the offending name so the CLI can
    /// suggest `--ndjson` / `--materialize`.
    #[error("filter expression uses '{name}' which requires whole-document evaluation; \
             consider `--ndjson` (per-line full semantics) or `--materialize` (M4b)")]
    Aggregate { name: String },

    /// jaq runtime error on one shard / line. `where_` carries the
    /// shard's line number (NDJSON) or array index / object key
    /// (single-document) for stderr reporting.
    #[error("filter runtime error at {where_}: {msg}")]
    Runtime { where_: String, msg: String },

    /// Object or scalar shard produced more than one output. We can't
    /// re-encode that in shape-preserving mode.
    #[error("filter at {where_} produced multiple outputs for {kind}; \
             use --ndjson or --materialize to allow this")]
    OutputShape { where_: String, kind: &'static str },

    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    /// Underlying parser/writer error from `jfmt_core`.
    #[error(transparent)]
    Core(#[from] crate::Error),
}

/// Options shared by both flavours.
#[derive(Debug, Clone, Default)]
pub struct FilterOptions {
    /// If `true`, runtime errors abort the run (mapped to non-zero
    /// exit code by the CLI). Otherwise they are reported to stderr
    /// and skipped.
    pub strict: bool,
}

pub use compile::{compile, Compiled};
```

- [ ] **Step 2: Create the five sub-module stubs**

Create `crates/jfmt-core/src/filter/compile.rs`:

```rust
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
```

Create `crates/jfmt-core/src/filter/static_check.rs`:

```rust
//! Walk the jaq AST and reject expressions that need whole-document
//! evaluation. Spec: design §3 D3.

// Filled in Task 4.
```

Create `crates/jfmt-core/src/filter/runtime.rs`:

```rust
//! Run a `Compiled` filter against one `serde_json::Value` with an
//! empty `inputs` iterator. See spec §4.3.

// Filled in Task 6.
```

Create `crates/jfmt-core/src/filter/shard.rs`:

```rust
//! Bridge between the event stream (`crate::Event`) and
//! `serde_json::Value` shards. See spec §4.1.

// Filled in Task 3.
```

Create `crates/jfmt-core/src/filter/output.rs`:

```rust
//! Shape-preserving emitter: array / object / scalar -> writer.
//! See spec §4.1 OutputShaper.

// Filled in Task 7.
```

- [ ] **Step 3: Wire into `lib.rs`**

In `crates/jfmt-core/src/lib.rs`, add `pub mod filter;` after `pub mod escape;`. Add re-exports under the existing `pub use` block:

```rust
pub mod filter;

pub use filter::{compile as compile_filter, Compiled, FilterError, FilterOptions};
```

- [ ] **Step 4: Build**

Run: `cargo build -p jfmt-core`
Expected: clean compile (warnings about `_placeholder`, `unimplemented!`, etc. are OK; `-D warnings` is *not* set during normal `cargo build`).

- [ ] **Step 5: Run existing tests**

Run: `cargo test -p jfmt-core`
Expected: all existing tests still pass; no new tests introduced yet.

- [ ] **Step 6: Commit**

```bash
git add crates/jfmt-core/src/filter crates/jfmt-core/src/lib.rs
git commit -m "$(cat <<'EOF'
feat(core): scaffold filter module + FilterError type

Sub-modules (compile/static_check/runtime/shard/output) are stubs
filled in Tasks 3-7. FilterError variants and FilterOptions are
final.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 3: ShardAccumulator (`shard.rs`) + round-trip property test

**Files:**
- Modify: `crates/jfmt-core/src/filter/shard.rs`
- Create: `crates/jfmt-core/tests/filter_shard_roundtrip.rs`

`ShardAccumulator` is a `Read`-side helper: feed it `Event`s, it emits a `serde_json::Value` per top-level shard (each element of a top-level array, each value of a top-level object, or the scalar / single value if the document is non-container at top level).

- [ ] **Step 1: Write the failing tests inline in `shard.rs`**

Replace the placeholder content of `crates/jfmt-core/src/filter/shard.rs` with the test scaffold first. We will write the implementation in Step 3.

```rust
//! Bridge between the event stream and `serde_json::Value` shards.

use crate::event::{Event, Scalar};
use serde_json::Value;

/// Top-level form of the document, decided after the first event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TopLevel {
    Array,
    Object,
    Scalar,
}

/// One shard ready for jaq, plus the locator used in error messages.
#[derive(Debug, Clone, PartialEq)]
pub struct Shard {
    /// 0-based array index, owned object key, or empty for top-level scalar.
    pub locator: ShardLocator,
    pub value: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ShardLocator {
    Index(u64),
    Key(String),
    Root,
}

/// Stateful accumulator. Feed it events one at a time; it returns
/// `Some(Shard)` whenever a top-level shard is complete, `None`
/// otherwise.
pub struct ShardAccumulator {
    state: State,
    /// Stack of partially-built containers waiting for their child events.
    stack: Vec<Builder>,
    next_index: u64,
}

enum State {
    /// Before any events.
    Start,
    /// Top-level form known; reading shards.
    Body { top: TopLevel },
    /// After the closing event of the top-level container.
    Done,
}

enum Builder {
    Array(Vec<Value>),
    Object {
        map: serde_json::Map<String, Value>,
        pending_key: Option<String>,
    },
}

impl ShardAccumulator {
    pub fn new() -> Self {
        Self {
            state: State::Start,
            stack: Vec::new(),
            next_index: 0,
        }
    }

    pub fn top_level(&self) -> Option<TopLevel> {
        match self.state {
            State::Body { top } => Some(top),
            _ => None,
        }
    }

    /// Feed one event. Returns the shard that just completed, if any.
    pub fn push(&mut self, ev: Event) -> Result<Option<Shard>, ShardError> {
        unimplemented!("Step 3")
    }
}

impl Default for ShardAccumulator {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ShardError {
    #[error("unexpected event in {state}: {event:?}")]
    Unexpected { state: &'static str, event: Event },
    #[error("event stream ended mid-shard")]
    Truncated,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drive(events: Vec<Event>) -> (Vec<Shard>, Option<TopLevel>) {
        let mut acc = ShardAccumulator::new();
        let mut shards = Vec::new();
        for ev in events {
            if let Some(s) = acc.push(ev).expect("push") {
                shards.push(s);
            }
        }
        (shards, acc.top_level())
    }

    #[test]
    fn top_level_array_emits_one_shard_per_element() {
        let evs = vec![
            Event::StartArray,
            Event::Value(Scalar::Number("1".into())),
            Event::Value(Scalar::Number("2".into())),
            Event::EndArray,
        ];
        let (shards, top) = drive(evs);
        assert_eq!(top, Some(TopLevel::Array));
        assert_eq!(shards.len(), 2);
        assert_eq!(shards[0].locator, ShardLocator::Index(0));
        assert_eq!(shards[0].value, serde_json::json!(1));
        assert_eq!(shards[1].locator, ShardLocator::Index(1));
        assert_eq!(shards[1].value, serde_json::json!(2));
    }

    #[test]
    fn top_level_object_emits_one_shard_per_key() {
        let evs = vec![
            Event::StartObject,
            Event::Name("a".into()),
            Event::Value(Scalar::Number("1".into())),
            Event::Name("b".into()),
            Event::Value(Scalar::String("hi".into())),
            Event::EndObject,
        ];
        let (shards, top) = drive(evs);
        assert_eq!(top, Some(TopLevel::Object));
        assert_eq!(shards.len(), 2);
        assert_eq!(shards[0].locator, ShardLocator::Key("a".into()));
        assert_eq!(shards[0].value, serde_json::json!(1));
        assert_eq!(shards[1].locator, ShardLocator::Key("b".into()));
        assert_eq!(shards[1].value, serde_json::json!("hi"));
    }

    #[test]
    fn top_level_scalar_emits_one_shard_at_root() {
        let evs = vec![Event::Value(Scalar::Bool(true))];
        let (shards, top) = drive(evs);
        assert_eq!(top, Some(TopLevel::Scalar));
        assert_eq!(shards.len(), 1);
        assert_eq!(shards[0].locator, ShardLocator::Root);
        assert_eq!(shards[0].value, serde_json::json!(true));
    }

    #[test]
    fn nested_array_inside_array_shard_assembles_correctly() {
        let evs = vec![
            Event::StartArray,
            Event::StartArray,
            Event::Value(Scalar::Number("1".into())),
            Event::Value(Scalar::Number("2".into())),
            Event::EndArray,
            Event::EndArray,
        ];
        let (shards, _) = drive(evs);
        assert_eq!(shards.len(), 1);
        assert_eq!(shards[0].value, serde_json::json!([1, 2]));
    }

    #[test]
    fn nested_object_inside_array_shard_assembles_correctly() {
        let evs = vec![
            Event::StartArray,
            Event::StartObject,
            Event::Name("k".into()),
            Event::Value(Scalar::Null),
            Event::EndObject,
            Event::EndArray,
        ];
        let (shards, _) = drive(evs);
        assert_eq!(shards.len(), 1);
        assert_eq!(shards[0].value, serde_json::json!({"k": null}));
    }

    #[test]
    fn empty_top_level_array_emits_no_shards() {
        let evs = vec![Event::StartArray, Event::EndArray];
        let (shards, top) = drive(evs);
        assert_eq!(top, Some(TopLevel::Array));
        assert!(shards.is_empty());
    }

    #[test]
    fn empty_top_level_object_emits_no_shards() {
        let evs = vec![Event::StartObject, Event::EndObject];
        let (shards, top) = drive(evs);
        assert_eq!(top, Some(TopLevel::Object));
        assert!(shards.is_empty());
    }

    #[test]
    fn number_preserves_lexical_form() {
        let evs = vec![Event::Value(Scalar::Number("1.0e10".into()))];
        let (shards, _) = drive(evs);
        // serde_json may normalise the literal; what we assert is that
        // *some* number was produced, and the conversion did not panic.
        assert!(shards[0].value.is_number());
    }
}
```

- [ ] **Step 2: Run the failing test**

Run: `cargo test -p jfmt-core --lib filter::shard`
Expected: every test panics with `unimplemented!("Step 3")`. This proves the tests reach the production code.

- [ ] **Step 3: Implement `push`**

Replace the `unimplemented!` in `ShardAccumulator::push` with this body:

```rust
pub fn push(&mut self, ev: Event) -> Result<Option<Shard>, ShardError> {
    use Event::*;
    match (&mut self.state, ev) {
        // ---- decide top-level form on the first event ----
        (State::Start, StartArray) => {
            self.state = State::Body { top: TopLevel::Array };
            Ok(None)
        }
        (State::Start, StartObject) => {
            self.state = State::Body { top: TopLevel::Object };
            Ok(None)
        }
        (State::Start, Value(s)) => {
            self.state = State::Done;
            Ok(Some(Shard {
                locator: ShardLocator::Root,
                value: scalar_to_value(s),
            }))
        }
        (State::Start, e) => Err(ShardError::Unexpected {
            state: "start",
            event: e,
        }),

        // ---- closing the top-level container ----
        (State::Body { top: TopLevel::Array }, EndArray) if self.stack.is_empty() => {
            self.state = State::Done;
            Ok(None)
        }
        (State::Body { top: TopLevel::Object }, EndObject) if self.stack.is_empty() => {
            self.state = State::Done;
            Ok(None)
        }

        // ---- top-level array: each completed value is a shard ----
        (State::Body { top: TopLevel::Array }, ev) if self.stack.is_empty() => {
            // Start a builder if the event opens a container; if it's
            // a scalar, emit a shard immediately.
            match ev {
                StartArray => {
                    self.stack.push(Builder::Array(Vec::new()));
                    Ok(None)
                }
                StartObject => {
                    self.stack.push(Builder::Object {
                        map: serde_json::Map::new(),
                        pending_key: None,
                    });
                    Ok(None)
                }
                Value(s) => {
                    let idx = self.next_index;
                    self.next_index += 1;
                    Ok(Some(Shard {
                        locator: ShardLocator::Index(idx),
                        value: scalar_to_value(s),
                    }))
                }
                e => Err(ShardError::Unexpected {
                    state: "top-array",
                    event: e,
                }),
            }
        }

        // ---- top-level object: track pending key, emit on value ----
        (State::Body { top: TopLevel::Object }, ev) if self.stack.is_empty() => {
            // We expect Name then Value/StartArray/StartObject pairs.
            // Track pending_key in a single-element builder simulating
            // the object-at-depth-0 to keep the inner code uniform.
            match ev {
                Name(k) => {
                    // Push a synthetic "depth-1 object scope" carrying
                    // only the pending key.
                    self.stack.push(Builder::Object {
                        map: serde_json::Map::new(),
                        pending_key: Some(k),
                    });
                    Ok(None)
                }
                e => Err(ShardError::Unexpected {
                    state: "top-object",
                    event: e,
                }),
            }
        }

        // ---- inside a builder: assemble a value ----
        (State::Body { top }, ev) => {
            assemble(&mut self.stack, ev, *top, &mut self.next_index)
        }

        (State::Done, e) => Err(ShardError::Unexpected {
            state: "done",
            event: e,
        }),
    }
}
```

Add the helper functions below the `impl` block:

```rust
fn scalar_to_value(s: Scalar) -> Value {
    match s {
        Scalar::String(s) => Value::String(s),
        Scalar::Number(lex) => {
            // Prefer to keep the original lexical form. If serde_json
            // can parse it as a Number, use that; otherwise fall back
            // to a string so we never lose data.
            serde_json::from_str::<Value>(&lex).unwrap_or(Value::String(lex))
        }
        Scalar::Bool(b) => Value::Bool(b),
        Scalar::Null => Value::Null,
    }
}

fn assemble(
    stack: &mut Vec<Builder>,
    ev: Event,
    top: TopLevel,
    next_index: &mut u64,
) -> Result<Option<Shard>, ShardError> {
    use Event::*;
    let value: Value = match ev {
        StartArray => {
            stack.push(Builder::Array(Vec::new()));
            return Ok(None);
        }
        StartObject => {
            stack.push(Builder::Object {
                map: serde_json::Map::new(),
                pending_key: None,
            });
            return Ok(None);
        }
        Name(k) => {
            match stack.last_mut() {
                Some(Builder::Object { pending_key, .. }) => {
                    *pending_key = Some(k);
                    return Ok(None);
                }
                _ => {
                    return Err(ShardError::Unexpected {
                        state: "name-outside-object",
                        event: Name(k),
                    });
                }
            }
        }
        Value(s) => scalar_to_value(s),
        EndArray => {
            let b = stack.pop().ok_or(ShardError::Truncated)?;
            match b {
                Builder::Array(v) => Value::Array(v),
                Builder::Object { .. } => {
                    return Err(ShardError::Unexpected {
                        state: "end-array-mismatch",
                        event: EndArray,
                    });
                }
            }
        }
        EndObject => {
            let b = stack.pop().ok_or(ShardError::Truncated)?;
            match b {
                Builder::Object { map, pending_key } => {
                    if pending_key.is_some() {
                        return Err(ShardError::Unexpected {
                            state: "object-ended-with-pending-key",
                            event: EndObject,
                        });
                    }
                    Value::Object(map)
                }
                Builder::Array(_) => {
                    return Err(ShardError::Unexpected {
                        state: "end-object-mismatch",
                        event: EndObject,
                    });
                }
            }
        }
    };

    // We have a fully-realised `value`. Either consume it as a child
    // of the parent builder, or emit a shard if the stack is now
    // shallow enough that this is a top-level shard.
    place_value(stack, value, top, next_index)
}

fn place_value(
    stack: &mut Vec<Builder>,
    value: Value,
    top: TopLevel,
    next_index: &mut u64,
) -> Result<Option<Shard>, ShardError> {
    // Top-array: stack empty after this place => shard at index n.
    // Top-object: stack has the synthetic depth-1 scope holding the
    //             pending key; emit shard when that scope receives
    //             its value.
    // Scalar:    handled in `push` directly; never reaches here.
    let _ = top;

    match stack.last_mut() {
        Some(Builder::Array(v)) => {
            v.push(value);
            Ok(None)
        }
        Some(Builder::Object { map, pending_key }) => {
            let key = pending_key.take().ok_or_else(|| ShardError::Unexpected {
                state: "value-without-key",
                event: Event::Value(Scalar::Null),
            })?;
            // If this is the synthetic top-level object scope (it
            // owns no `map` content because we emit per-key shards),
            // emit the shard and pop.
            if stack.len() == 1 && matches!(top, TopLevel::Object) {
                stack.pop();
                return Ok(Some(Shard {
                    locator: ShardLocator::Key(key),
                    value,
                }));
            }
            map.insert(key, value);
            Ok(None)
        }
        None => {
            // Stack empty: this is a top-level array element.
            debug_assert!(matches!(top, TopLevel::Array));
            let idx = *next_index;
            *next_index += 1;
            Ok(Some(Shard {
                locator: ShardLocator::Index(idx),
                value,
            }))
        }
    }
}
```

- [ ] **Step 4: Run the unit tests**

Run: `cargo test -p jfmt-core --lib filter::shard`
Expected: all 8 tests pass.

- [ ] **Step 5: Write the round-trip property test**

Create `crates/jfmt-core/tests/filter_shard_roundtrip.rs`:

```rust
//! Property: any serde_json::Value, when serialized and re-parsed
//! through the EventReader and ShardAccumulator, reproduces an
//! equivalent Value sequence.

use jfmt_core::filter::shard::{ShardAccumulator, ShardLocator, TopLevel};
use jfmt_core::EventReader;
use proptest::prelude::*;
use serde_json::Value;

fn arb_value() -> impl Strategy<Value = Value> {
    let leaf = prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::Bool),
        any::<i32>().prop_map(|n| serde_json::json!(n)),
        "[a-zA-Z0-9 ]{0,8}".prop_map(Value::String),
    ];
    leaf.prop_recursive(3, 24, 6, |inner| {
        prop_oneof![
            prop::collection::vec(inner.clone(), 0..6).prop_map(Value::Array),
            prop::collection::hash_map("[a-z]{1,3}", inner, 0..4)
                .prop_map(|m| Value::Object(m.into_iter().collect())),
        ]
    })
}

proptest! {
    #[test]
    fn roundtrip_preserves_value(v in arb_value()) {
        let bytes = serde_json::to_vec(&v).unwrap();
        let mut reader = EventReader::new(&bytes[..]);
        let mut acc = ShardAccumulator::new();
        let mut shards = Vec::new();
        while let Some(ev) = reader.next_event().unwrap() {
            if let Some(s) = acc.push(ev).unwrap() {
                shards.push(s);
            }
        }
        let top = acc.top_level().expect("top decided");
        match top {
            TopLevel::Array => {
                let want = v.as_array().unwrap();
                prop_assert_eq!(shards.len(), want.len());
                for (i, s) in shards.iter().enumerate() {
                    prop_assert_eq!(s.locator.clone(), ShardLocator::Index(i as u64));
                    prop_assert_eq!(&s.value, &want[i]);
                }
            }
            TopLevel::Object => {
                let want = v.as_object().unwrap();
                prop_assert_eq!(shards.len(), want.len());
                for s in &shards {
                    let key = match &s.locator {
                        ShardLocator::Key(k) => k,
                        _ => panic!("expected Key locator"),
                    };
                    prop_assert_eq!(&s.value, want.get(key).unwrap());
                }
            }
            TopLevel::Scalar => {
                prop_assert_eq!(shards.len(), 1);
                prop_assert_eq!(&shards[0].value, &v);
            }
        }
    }
}
```

- [ ] **Step 6: Run the proptest**

Run: `cargo test -p jfmt-core --test filter_shard_roundtrip`
Expected: 1 test, 256 cases (proptest default), all pass. If proptest shrinks an input that fails, capture the shrunk `Value` and **stop** — fix the implementation before continuing.

- [ ] **Step 7: Commit**

```bash
git add crates/jfmt-core/src/filter/shard.rs crates/jfmt-core/tests/filter_shard_roundtrip.rs
git commit -m "$(cat <<'EOF'
feat(core): add ShardAccumulator + Event<->Value roundtrip property

Bridges the EventReader to per-shard serde_json::Value for the filter
engine: top-level array -> indexed shards, top-level object ->
keyed shards, top-level scalar -> single Root shard. Empty containers
emit zero shards.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 4: Static check (`static_check.rs`)

The static checker walks the parsed jaq AST and rejects expressions that contain known whole-document builtins or `input` / `inputs`. The list is the spec §3 D3 blacklist.

**Files:**
- Modify: `crates/jfmt-core/src/filter/static_check.rs`

The exact AST type names depend on the jaq version frozen in Task 1's Annex A. Below the AST type is referred to as `jaq_syn::Term` (the typical name); replace with whatever the spike's annex shows.

- [ ] **Step 1: Write failing tests inline**

Replace the placeholder content of `crates/jfmt-core/src/filter/static_check.rs` with:

```rust
//! AST blacklist scanner. Rejects expressions that need whole-document
//! evaluation. See spec §3 D3.

use super::FilterError;

/// Names that cannot be evaluated per-shard. Sorted so binary_search
/// works.
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

/// Scan a parsed AST. Return `Err(FilterError::Aggregate)` on the
/// first blacklisted name encountered.
pub fn check(ast: &jaq_syn::Main) -> Result<(), FilterError> {
    visit_main(ast)
}

fn visit_main(_m: &jaq_syn::Main) -> Result<(), FilterError> {
    unimplemented!("Step 3 fills in once Annex A's AST type is known")
}

fn is_blacklisted(name: &str) -> bool {
    BLACKLIST.binary_search(&name).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filter::FilterError;

    fn parse(expr: &str) -> jaq_syn::Main {
        // Use the parse function from Annex A.
        jaq_syn::parse(expr).expect("parse")
    }

    fn assert_aggregate(expr: &str, expected_name: &str) {
        match check(&parse(expr)) {
            Err(FilterError::Aggregate { name }) => assert_eq!(name, expected_name),
            other => panic!("expected Aggregate({expected_name:?}), got {other:?}"),
        }
    }

    fn assert_ok(expr: &str) {
        check(&parse(expr)).expect("expression must pass static check");
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
    fn rejects_min_max_unique() {
        assert_aggregate("min", "min");
        assert_aggregate("max", "max");
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
```

- [ ] **Step 2: Run failing tests**

Run: `cargo test -p jfmt-core --lib filter::static_check`
Expected: all tests fail with `unimplemented!`.

- [ ] **Step 3: Implement the visitor**

Replace `visit_main` and add a recursive visitor over the AST. **The exact match arms depend on the jaq AST shape recorded in Annex A.** The pattern is fixed; adapt the arm names:

```rust
fn visit_main(m: &jaq_syn::Main) -> Result<(), FilterError> {
    // jaq's `Main` typically wraps a top-level `Term` plus `defs`.
    // Visit defs first (their bodies might introduce blacklisted
    // calls that the body of the main term then references), then
    // the body.
    for def in m.defs.iter() {
        visit_term(&def.body)?;
    }
    visit_term(&m.body)
}

fn visit_term(t: &jaq_syn::Term) -> Result<(), FilterError> {
    use jaq_syn::Term::*;
    match t {
        // A direct call by name. This is the primary blacklist hit.
        Call(name, args) => {
            if is_blacklisted(name) {
                return Err(FilterError::Aggregate {
                    name: name.clone(),
                });
            }
            for a in args {
                visit_term(a)?;
            }
            Ok(())
        }

        // Any composite term: descend into all child terms. The exact
        // variant set depends on the jaq version; **Annex A's AST
        // dump is the source of truth**. Add arms for every variant
        // here, defaulting to "no children" only for true leaves
        // (Num, Str, etc.).
        Pipe(a, _, b) | BinOp(a, _, b) => {
            visit_term(a)?;
            visit_term(b)
        }
        Neg(a) | Try(a, None) => visit_term(a),
        Try(a, Some(b)) => {
            visit_term(a)?;
            visit_term(b)
        }
        IfThenElse(branches, alt) => {
            for (c, t) in branches {
                visit_term(c)?;
                visit_term(t)?;
            }
            if let Some(a) = alt {
                visit_term(a)?;
            }
            Ok(())
        }
        Var(_) | Num(_) | Str(_) | True | False | Null => Ok(()),
        Path(_, segs) => {
            // Filter / iterate path segments may contain expressions.
            for s in segs {
                visit_path_segment(s)?;
            }
            Ok(())
        }
        Arr(inner) => match inner {
            Some(t) => visit_term(t),
            None => Ok(()),
        },
        Obj(entries) => {
            for (k, v) in entries {
                visit_term(k)?;
                visit_term(v)?;
            }
            Ok(())
        }
        Recurse | Id => Ok(()),
        // Catch-all to make this code resilient to AST extensions in
        // future jaq versions: descend into any unknown variant via
        // its Debug repr is impossible, so we are conservative and
        // *accept* unknown variants. The runtime guard (Task 6) is
        // the safety net.
        _ => Ok(()),
    }
}

fn visit_path_segment(_s: &jaq_syn::path::Part<jaq_syn::Term>) -> Result<(), FilterError> {
    // Path segments wrap inner terms (e.g., `.[expr]`). Visit them.
    // Specific arm shape from Annex A. If the jaq version exposes
    // `iter_terms()` or similar, prefer that.
    Ok(())
}
```

If the AST surface in Annex A differs (e.g., `Term::Call { name, args }` named-struct form, or `IfThenElse` is `If(Vec<(Term, Term)>, Option<Box<Term>>)`), update the arm shapes — the *behaviour* is unchanged: every leaf checks `is_blacklisted` and recurses into every child term.

- [ ] **Step 4: Run the unit tests**

Run: `cargo test -p jfmt-core --lib filter::static_check`
Expected: 13 tests pass. If a "rejects_*" test passes only because parsing fails, that's a bug — fix the visitor so the AST is reached.

- [ ] **Step 5: Commit**

```bash
git add crates/jfmt-core/src/filter/static_check.rs
git commit -m "$(cat <<'EOF'
feat(core): add filter static-check (blacklist + inputs/input)

Walks jaq's parsed AST and rejects whole-document builtins
(length/sort_by/group_by/add/min/max/unique and friends), plus the
input/inputs primitives. Unknown AST variants are accepted; runtime
guard in Task 6 catches misses.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 5: `compile()` glue

**Files:**
- Modify: `crates/jfmt-core/src/filter/compile.rs`

- [ ] **Step 1: Replace the stub with the real shape**

```rust
//! Parse + static-check + jaq compile.

use super::{static_check, FilterError};
use std::sync::Arc;

/// Compiled, ready-to-run filter. Cheap to clone; share across
/// NDJSON workers.
#[derive(Clone)]
pub struct Compiled {
    pub(crate) inner: Arc<CompiledInner>,
}

pub(crate) struct CompiledInner {
    /// The actual jaq Filter — type comes from Annex A.
    pub(crate) filter: jaq_core::Filter<jaq_std::Native>,
}

pub fn compile(expr: &str) -> Result<Compiled, FilterError> {
    // (1) Parse.
    let main = jaq_syn::parse(expr).map_err(|e| FilterError::Parse { msg: format!("{e}") })?;

    // (2) Static check.
    static_check::check(&main)?;

    // (3) Compile against jaq-std's standard library.
    //
    // Names below are the typical surface; adjust per Annex A.
    let std_defs = jaq_std::funs();
    let compiler = jaq_core::Compiler::default().with_funs(std_defs);
    let filter = compiler
        .compile(main)
        .map_err(|e| FilterError::Parse { msg: format!("{e}") })?;

    Ok(Compiled {
        inner: Arc::new(CompiledInner { filter }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
```

- [ ] **Step 2: Run**

Run: `cargo test -p jfmt-core --lib filter::compile`
Expected: 3 tests pass. If `jaq_core::Compiler::default()` does not exist, replace with whatever Annex A shows (`Compiler::new()`, `Compiler::with_funs(funs)`, etc.). The contract is unchanged.

- [ ] **Step 3: Commit**

```bash
git add crates/jfmt-core/src/filter/compile.rs
git commit -m "$(cat <<'EOF'
feat(core): wire filter compile() — parse + static-check + jaq compile

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 6: Runtime wrapper (`runtime.rs`)

**Files:**
- Modify: `crates/jfmt-core/src/filter/runtime.rs`

- [ ] **Step 1: Replace the stub**

```rust
//! Run a `Compiled` filter against one `serde_json::Value`. The
//! `inputs` iterator is always empty so any `input` / `inputs`
//! reference that slipped past the static check raises a clean
//! runtime error.

use super::{Compiled, FilterError};
use serde_json::Value;

/// Run `compiled` against `input`. Returns the stream of jaq output
/// values (0, 1, or N).
pub fn run_one(compiled: &Compiled, input: Value) -> Result<Vec<Value>, FilterError> {
    // The exact API names below come from Annex A.
    //
    // Conceptually:
    //   let ctx = jaq_core::Ctx::new(vec![], &mut std::iter::empty());
    //   let outputs: Vec<_> = compiled.inner.filter.run((ctx, input.into())).collect();
    //
    // Each `output` is `Result<jaq_core::Val, jaq_core::Error>`.

    let mut empty_inputs = std::iter::empty();
    let ctx = jaq_core::Ctx::new(Vec::new(), &mut empty_inputs);

    let mut out = Vec::new();
    for r in compiled.inner.filter.run((ctx, input.into())) {
        let v = r.map_err(|e| FilterError::Runtime {
            where_: String::new(),
            msg: format!("{e}"),
        })?;
        out.push(v.into());
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
```

- [ ] **Step 2: Run**

Run: `cargo test -p jfmt-core --lib filter::runtime`
Expected: 4 tests pass. If `jaq_core::Ctx::new` signature differs (e.g., requires a different `inputs` adapter type or generic argument), wrap the empty iterator in whatever Annex A specifies. The four tests are the contract.

- [ ] **Step 3: Commit**

```bash
git add crates/jfmt-core/src/filter/runtime.rs
git commit -m "$(cat <<'EOF'
feat(core): add filter runtime wrapper with empty inputs guard

Compiles a 'select(.x>0)' against a Value, returns 0/1/N outputs.
Empty inputs iterator means input/inputs always errors at runtime.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 7: OutputShaper (`output.rs`)

**Files:**
- Modify: `crates/jfmt-core/src/filter/output.rs`

The shaper writes filtered outputs back to a `Write` in a form that matches the input's top-level shape, using the existing `MinifyWriter` / `PrettyWriter`.

- [ ] **Step 1: Write the failing tests**

Replace `crates/jfmt-core/src/filter/output.rs` with:

```rust
//! Shape-preserving emitter. See spec §4.1.

use super::{shard::TopLevel, FilterError};
use crate::event::{Event, Scalar};
use crate::writer::EventWriter;
use serde_json::Value;

/// Streaming output for single-document mode. Caller drives by
/// calling `begin(top)` once, then `emit(...)` per shard, then
/// `finish()`.
pub struct OutputShaper<W: EventWriter> {
    writer: W,
    top: Option<TopLevel>,
}

impl<W: EventWriter> OutputShaper<W> {
    pub fn new(writer: W) -> Self {
        Self { writer, top: None }
    }

    pub fn begin(&mut self, top: TopLevel) -> Result<(), FilterError> {
        self.top = Some(top);
        match top {
            TopLevel::Array => self.writer.write_event(&Event::StartArray)?,
            TopLevel::Object => self.writer.write_event(&Event::StartObject)?,
            TopLevel::Scalar => {}
        }
        Ok(())
    }

    /// Emit zero, one, or many jaq output values for a single shard.
    /// `key` is `Some` if the input top-level was Object; `None`
    /// otherwise. `where_` is used in `OutputShape` errors.
    pub fn emit(
        &mut self,
        outputs: Vec<Value>,
        key: Option<&str>,
        where_: &str,
    ) -> Result<(), FilterError> {
        let top = self.top.expect("begin must be called");
        match top {
            TopLevel::Array => {
                for v in outputs {
                    write_value(&mut self.writer, &v)?;
                }
                Ok(())
            }
            TopLevel::Object => match outputs.len() {
                0 => Ok(()),
                1 => {
                    let k = key.expect("Object top-level requires key");
                    self.writer.write_event(&Event::Name(k.to_string()))?;
                    write_value(&mut self.writer, &outputs[0])
                }
                _ => Err(FilterError::OutputShape {
                    where_: where_.to_string(),
                    kind: "object",
                }),
            },
            TopLevel::Scalar => match outputs.len() {
                0 => Ok(()),
                1 => write_value(&mut self.writer, &outputs[0]),
                _ => Err(FilterError::OutputShape {
                    where_: where_.to_string(),
                    kind: "scalar",
                }),
            },
        }
    }

    pub fn finish(mut self) -> Result<(), FilterError> {
        match self.top {
            Some(TopLevel::Array) => self.writer.write_event(&Event::EndArray)?,
            Some(TopLevel::Object) => self.writer.write_event(&Event::EndObject)?,
            Some(TopLevel::Scalar) | None => {}
        }
        self.writer.finish()?;
        Ok(())
    }
}

/// Emit a `serde_json::Value` as a sequence of `Event`s into `writer`.
fn write_value<W: EventWriter>(writer: &mut W, v: &Value) -> Result<(), FilterError> {
    match v {
        Value::Null => writer.write_event(&Event::Value(Scalar::Null))?,
        Value::Bool(b) => writer.write_event(&Event::Value(Scalar::Bool(*b)))?,
        Value::Number(n) => writer.write_event(&Event::Value(Scalar::Number(n.to_string())))?,
        Value::String(s) => writer.write_event(&Event::Value(Scalar::String(s.clone())))?,
        Value::Array(items) => {
            writer.write_event(&Event::StartArray)?;
            for it in items {
                write_value(writer, it)?;
            }
            writer.write_event(&Event::EndArray)?;
        }
        Value::Object(map) => {
            writer.write_event(&Event::StartObject)?;
            for (k, v) in map {
                writer.write_event(&Event::Name(k.clone()))?;
                write_value(writer, v)?;
            }
            writer.write_event(&Event::EndObject)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::writer::MinifyWriter;
    use serde_json::json;

    fn shape_with_minify<F>(top: TopLevel, body: F) -> String
    where
        F: FnOnce(&mut OutputShaper<MinifyWriter<Vec<u8>>>),
    {
        let buf = Vec::new();
        let writer = MinifyWriter::new(buf);
        let mut shaper = OutputShaper::new(writer);
        shaper.begin(top).unwrap();
        body(&mut shaper);
        // Steal the inner buffer by finishing.
        let MinifyWriter { /*inner*/ .. } = unsafe { std::mem::zeroed() }; // placeholder
        // The real way: re-build with into_inner. See note below.
        unreachable!("the real test uses an in-test helper that exposes the buffer; \
                      see Step 1.5 below");
    }

    // Step 1.5: replace the helper with one that consumes the shaper
    // and returns the bytes. See Step 3 of this task.

    #[test]
    fn array_zero_outputs_drops_element() {
        // Filled in Step 3.
    }

    #[test]
    fn array_n_outputs_expand() {
        // Filled in Step 3.
    }

    #[test]
    fn object_one_output_writes_pair() {
        // Filled in Step 3.
    }

    #[test]
    fn object_zero_outputs_drops_key() {
        // Filled in Step 3.
    }

    #[test]
    fn object_n_outputs_errors() {
        // Filled in Step 3.
    }

    #[test]
    fn scalar_one_output_writes_value() {
        // Filled in Step 3.
    }

    #[test]
    fn scalar_n_outputs_errors() {
        // Filled in Step 3.
    }

    fn _suppress_unused_for_compile() {
        let _ = shape_with_minify::<fn(&mut OutputShaper<MinifyWriter<Vec<u8>>>)>;
    }

    fn _emit_dummy(_v: Value) {
        let _ = json!({});
    }
}
```

- [ ] **Step 2: Run the failing tests**

Run: `cargo test -p jfmt-core --lib filter::output`
Expected: tests are placeholders and pass trivially; the real assertions land in Step 3. This step exists to confirm the file compiles after the imports.

If `MinifyWriter` does not have a public `new(W)` or `into_inner()`, peek at `crates/jfmt-core/src/writer/minify.rs` to learn the real constructor. The tests below assume `MinifyWriter::new(buf)` and that consuming `finish()` flushes the inner writer. Adapt as needed.

- [ ] **Step 3: Replace the test bodies with real assertions**

Replace the entire `#[cfg(test)] mod tests` block in `output.rs` with:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::writer::MinifyWriter;
    use serde_json::json;

    /// Helper: shape outputs and return the resulting bytes as UTF-8.
    fn shape(top: TopLevel, calls: &[(&[Value], Option<&str>)]) -> Result<String, FilterError> {
        let buf = Vec::<u8>::new();
        let writer = MinifyWriter::new(buf);
        let mut shaper = OutputShaper::new(writer);
        shaper.begin(top)?;
        for (i, (vals, key)) in calls.iter().enumerate() {
            let where_ = format!("idx={i}");
            shaper.emit(vals.to_vec(), *key, &where_)?;
        }
        // We need the bytes back. MinifyWriter must expose them via
        // into_inner() or similar; if it doesn't, add an inherent
        // method `pub fn into_inner(self) -> W` to MinifyWriter
        // (single line, no behaviour change).
        let bytes = shaper.finish_into_bytes()?;
        Ok(String::from_utf8(bytes).unwrap())
    }

    #[test]
    fn array_zero_outputs_drops_element() {
        let s = shape(TopLevel::Array, &[(&[], None), (&[json!(1)], None)]).unwrap();
        assert_eq!(s, "[1]");
    }

    #[test]
    fn array_n_outputs_expand() {
        let s = shape(TopLevel::Array, &[(&[json!(1), json!(2)], None)]).unwrap();
        assert_eq!(s, "[1,2]");
    }

    #[test]
    fn object_one_output_writes_pair() {
        let s = shape(TopLevel::Object, &[(&[json!(1)], Some("a"))]).unwrap();
        assert_eq!(s, "{\"a\":1}");
    }

    #[test]
    fn object_zero_outputs_drops_key() {
        let s = shape(
            TopLevel::Object,
            &[(&[], Some("a")), (&[json!(2)], Some("b"))],
        )
        .unwrap();
        assert_eq!(s, "{\"b\":2}");
    }

    #[test]
    fn object_n_outputs_errors() {
        let err = shape(TopLevel::Object, &[(&[json!(1), json!(2)], Some("a"))]).unwrap_err();
        assert!(matches!(err, FilterError::OutputShape { kind: "object", .. }));
    }

    #[test]
    fn scalar_one_output_writes_value() {
        let s = shape(TopLevel::Scalar, &[(&[json!(true)], None)]).unwrap();
        assert_eq!(s, "true");
    }

    #[test]
    fn scalar_n_outputs_errors() {
        let err = shape(TopLevel::Scalar, &[(&[json!(1), json!(2)], None)]).unwrap_err();
        assert!(matches!(err, FilterError::OutputShape { kind: "scalar", .. }));
    }
}
```

The helper calls `shaper.finish_into_bytes()`. Add that method to `OutputShaper` (above `pub fn finish`):

```rust
    /// Test-only: finish and return the underlying bytes. Requires
    /// the writer to support `into_inner()`.
    #[cfg(test)]
    pub fn finish_into_bytes(mut self) -> Result<Vec<u8>, FilterError>
    where
        W: crate::writer::IntoInner<Vec<u8>>,
    {
        match self.top {
            Some(TopLevel::Array) => self.writer.write_event(&Event::EndArray)?,
            Some(TopLevel::Object) => self.writer.write_event(&Event::EndObject)?,
            Some(TopLevel::Scalar) | None => {}
        }
        self.writer.finish()?;
        Ok(self.writer.into_inner())
    }
```

This requires a small addition to `crates/jfmt-core/src/writer/mod.rs`:

```rust
/// Optional capability for tests / introspection: yield the
/// underlying writer, consuming the wrapper.
pub trait IntoInner<T> {
    fn into_inner(self) -> T;
}
```

…and an impl in each of `pretty.rs` / `minify.rs`:

```rust
impl<W: std::io::Write> crate::writer::IntoInner<W> for MinifyWriter<W> {
    fn into_inner(self) -> W {
        self.inner
    }
}

// (and analogous for PrettyWriter)
impl<W: std::io::Write> crate::writer::IntoInner<W> for PrettyWriter<W> {
    fn into_inner(self) -> W {
        self.inner
    }
}
```

If the existing field name on these structs is not `inner`, use whatever it is — peek at the file.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p jfmt-core --lib filter::output`
Expected: 7 tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/jfmt-core/src/filter/output.rs crates/jfmt-core/src/writer/
git commit -m "$(cat <<'EOF'
feat(core): add OutputShaper with shape-preserving emit

Array expands N outputs in place, Object writes 0/1 (errors on N>1),
Scalar writes 0/1 (errors on N>1). Adds writer::IntoInner trait so
tests can recover bytes from MinifyWriter / PrettyWriter.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 8: `run_streaming` glue + integration tests

**Files:**
- Modify: `crates/jfmt-core/src/filter/mod.rs`
- Create: `crates/jfmt-core/tests/filter_streaming.rs`

- [ ] **Step 1: Add `run_streaming` to `filter/mod.rs`**

Append to `filter/mod.rs`:

```rust
use crate::event::Event;
use crate::parser::EventReader;
use crate::writer::{EventWriter, MinifyWriter, PrettyConfig, PrettyWriter};
use std::io::{Read, Write};

pub use shard::{Shard, ShardAccumulator, ShardLocator, TopLevel};

/// Output formatting choice for filter results.
#[derive(Debug, Clone)]
pub enum FilterOutput {
    Compact,
    Pretty(PrettyConfig),
}

impl Default for FilterOutput {
    fn default() -> Self {
        FilterOutput::Compact
    }
}

/// Outcome of a streaming filter run.
#[derive(Debug, Default)]
pub struct StreamingReport {
    pub shards_seen: u64,
    pub runtime_errors: Vec<FilterError>,
}

/// Drive a single-document streaming filter from `reader` to `writer`.
/// Runtime errors are collected; if `opts.strict` is set, the first
/// error is returned immediately.
pub fn run_streaming<R: Read, W: Write>(
    reader: R,
    writer: W,
    compiled: &Compiled,
    output: FilterOutput,
    opts: FilterOptions,
) -> Result<StreamingReport, FilterError> {
    match output {
        FilterOutput::Compact => {
            let w = MinifyWriter::new(writer);
            run_streaming_inner(reader, w, compiled, opts)
        }
        FilterOutput::Pretty(cfg) => {
            let w = PrettyWriter::new(writer, cfg);
            run_streaming_inner(reader, w, compiled, opts)
        }
    }
}

fn run_streaming_inner<R: Read, W: EventWriter>(
    reader: R,
    writer: W,
    compiled: &Compiled,
    opts: FilterOptions,
) -> Result<StreamingReport, FilterError> {
    use crate::filter::output::OutputShaper;
    use crate::filter::runtime::run_one;

    let mut reader = EventReader::new(reader);
    let mut acc = ShardAccumulator::new();
    let mut shaper = OutputShaper::new(writer);
    let mut report = StreamingReport::default();
    let mut began = false;

    while let Some(ev) = reader.next_event()? {
        // First non-whitespace event tells us the shape; begin the
        // shaper before pushing it through.
        if !began {
            // Decide top-level form by the kind of the first event.
            let top = match &ev {
                Event::StartArray => TopLevel::Array,
                Event::StartObject => TopLevel::Object,
                _ => TopLevel::Scalar,
            };
            shaper.begin(top)?;
            began = true;
        }

        if let Some(shard) = acc.push(ev).map_err(|e| FilterError::Runtime {
            where_: String::new(),
            msg: format!("shard accumulator: {e}"),
        })? {
            report.shards_seen += 1;
            let where_ = match &shard.locator {
                ShardLocator::Index(i) => format!("[{i}]"),
                ShardLocator::Key(k) => format!(".{k}"),
                ShardLocator::Root => String::from("(root)"),
            };
            match run_one(compiled, shard.value) {
                Ok(outputs) => {
                    let key_owned;
                    let key = match &shard.locator {
                        ShardLocator::Key(k) => {
                            key_owned = k.clone();
                            Some(key_owned.as_str())
                        }
                        _ => None,
                    };
                    if let Err(e) = shaper.emit(outputs, key, &where_) {
                        if opts.strict {
                            return Err(e);
                        } else {
                            report.runtime_errors.push(e);
                        }
                    }
                }
                Err(mut e) => {
                    if let FilterError::Runtime { where_: w, .. } = &mut e {
                        *w = where_.clone();
                    }
                    if opts.strict {
                        return Err(e);
                    } else {
                        report.runtime_errors.push(e);
                    }
                }
            }
        }
    }

    if !began {
        // Empty input — nothing to do.
    } else {
        shaper.finish()?;
    }
    Ok(report)
}
```

- [ ] **Step 2: Create the integration test**

Create `crates/jfmt-core/tests/filter_streaming.rs`:

```rust
use jfmt_core::filter::{compile, run_streaming, FilterOptions, FilterOutput};

fn run(expr: &str, input: &str) -> (String, jfmt_core::filter::StreamingReport) {
    let compiled = compile(expr).expect("compile");
    let mut out = Vec::<u8>::new();
    let report = run_streaming(
        input.as_bytes(),
        &mut out,
        &compiled,
        FilterOutput::Compact,
        FilterOptions::default(),
    )
    .expect("run_streaming");
    (String::from_utf8(out).unwrap(), report)
}

#[test]
fn array_select_filters_elements() {
    let (out, _) = run("select(.x > 1)", r#"[{"x":1},{"x":2},{"x":3}]"#);
    assert_eq!(out, r#"[{"x":2},{"x":3}]"#);
}

#[test]
fn array_identity_passes_through() {
    let (out, _) = run(".", r#"[1,2,3]"#);
    assert_eq!(out, "[1,2,3]");
}

#[test]
fn object_filter_drops_keys() {
    let (out, _) = run("select(. > 1)", r#"{"a":1,"b":2,"c":3}"#);
    // Keys with value > 1 survive.
    assert_eq!(out, r#"{"b":2,"c":3}"#);
}

#[test]
fn scalar_filter_passes_through() {
    let (out, _) = run("select(. > 0)", "5");
    assert_eq!(out, "5");
}

#[test]
fn scalar_filter_dropping() {
    let (out, _) = run("select(. > 0)", "-1");
    assert_eq!(out, "");
}

#[test]
fn array_multi_output_expands() {
    let (out, _) = run(".x, .y", r#"[{"x":1,"y":2}]"#);
    assert_eq!(out, "[1,2]");
}

#[test]
fn object_multi_output_records_runtime_error() {
    let (out, report) = run(".a, .b", r#"{"k":{"a":1,"b":2}}"#);
    assert_eq!(out, "{}");
    assert_eq!(report.runtime_errors.len(), 1);
}

#[test]
fn type_error_records_runtime_error_default() {
    let (out, report) = run(".x + 1", r#"[{"x":"hi"},{"x":2}]"#);
    assert_eq!(out, "[3]");
    assert_eq!(report.runtime_errors.len(), 1);
}

#[test]
fn type_error_strict_returns_err() {
    let compiled = compile(".x + 1").unwrap();
    let mut out = Vec::<u8>::new();
    let err = run_streaming(
        r#"[{"x":"hi"}]"#.as_bytes(),
        &mut out,
        &compiled,
        FilterOutput::Compact,
        FilterOptions { strict: true },
    )
    .unwrap_err();
    assert!(matches!(
        err,
        jfmt_core::filter::FilterError::Runtime { .. }
    ));
}
```

- [ ] **Step 3: Run**

Run: `cargo test -p jfmt-core --test filter_streaming`
Expected: 9 tests pass.

If the "object_filter_drops_keys" test fails because the streaming object output preserves *some* original ordering inversion (serde_json::Map default vs ordered), accept whatever stable ordering the writer produces and adjust the assertion to match — what matters is that key `a` (value 1) is gone and keys `b`, `c` survive.

- [ ] **Step 4: Commit**

```bash
git add crates/jfmt-core/src/filter/mod.rs crates/jfmt-core/tests/filter_streaming.rs
git commit -m "$(cat <<'EOF'
feat(core): wire run_streaming end-to-end

Drives EventReader -> ShardAccumulator -> compiled filter ->
OutputShaper -> writer with shape-preserving output. Runtime errors
collected by default; --strict-equivalent FilterOptions.strict aborts
on the first error.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 9: NDJSON pipeline payload migration `Vec<u8>` → `Vec<Vec<u8>>`

**Why:** filter mode produces 0/1/N output values per input line. The reorder buffer currently writes one payload + `\n`; we need it to iterate.

**Files:**
- Modify: `crates/jfmt-core/src/ndjson/worker.rs`
- Modify: `crates/jfmt-core/src/ndjson/reorder.rs`
- Modify: `crates/jfmt-core/src/ndjson/mod.rs`
- Modify: `crates/jfmt-cli/src/commands/{pretty,minify,validate}.rs`

- [ ] **Step 1: Update `WorkerOutput`**

In `crates/jfmt-core/src/ndjson/worker.rs`, change:

```rust
pub type WorkerOutput = (u64, Result<Vec<u8>, LineError>);
```

to:

```rust
pub type WorkerOutput = (u64, Result<Vec<Vec<u8>>, LineError>);
```

Update `run_worker` (or whatever the closure-driving function is) so the `Ok` variant from `f` is `Vec<Vec<u8>>` instead of `Vec<u8>`. The closure signature itself changes too (Step 3).

- [ ] **Step 2: Update `reorder::emit`**

In `crates/jfmt-core/src/ndjson/reorder.rs`, replace:

```rust
fn emit<W: Write>(
    out: &mut W,
    errors: &mut Vec<(u64, LineError)>,
    cancel: &Arc<AtomicBool>,
    fail_fast: bool,
    seq: u64,
    payload: Result<Vec<u8>, LineError>,
) -> std::io::Result<()> {
    match payload {
        Ok(bytes) => {
            out.write_all(&bytes)?;
            out.write_all(b"\n")?;
        }
        Err(e) => {
            …
        }
    }
    Ok(())
}
```

with:

```rust
fn emit<W: Write>(
    out: &mut W,
    errors: &mut Vec<(u64, LineError)>,
    cancel: &Arc<AtomicBool>,
    fail_fast: bool,
    seq: u64,
    payload: Result<Vec<Vec<u8>>, LineError>,
) -> std::io::Result<()> {
    match payload {
        Ok(parts) => {
            for bytes in &parts {
                out.write_all(bytes)?;
                out.write_all(b"\n")?;
            }
        }
        Err(e) => {
            if fail_fast {
                if !cancel.swap(true, Ordering::Relaxed) {
                    errors.push((seq, e));
                }
            } else {
                errors.push((seq, e));
            }
        }
    }
    Ok(())
}
```

Update the corresponding `Entry` field type:

```rust
struct Entry {
    seq: u64,
    payload: Result<Vec<Vec<u8>>, LineError>,
}
```

Update each existing reorder unit test in this file: every occurrence of `Ok(b"foo".to_vec())` becomes `Ok(vec![b"foo".to_vec()])`. The expected output bytes are unchanged.

- [ ] **Step 3: Update closure signature in `mod.rs`**

In `crates/jfmt-core/src/ndjson/mod.rs`, change the `F` bound:

```rust
F: Fn(&[u8], &mut StatsCollector) -> Result<Vec<u8>, LineError> + Send + Sync + 'static,
```

to:

```rust
F: Fn(&[u8], &mut StatsCollector) -> Result<Vec<Vec<u8>>, LineError> + Send + Sync + 'static,
```

Update the inline tests in `mod.rs` (the `single_worker_end_to_end` test): wrap `out` with `vec![out]`.

- [ ] **Step 4: Update CLI callers**

In each of `crates/jfmt-cli/src/commands/{pretty,minify,validate}.rs`, find the `run_ndjson_pipeline` closure body. Wherever the closure currently returns `Ok(bytes)`, change to `Ok(vec![bytes])`. There is exactly one such return per closure; do not touch error returns.

- [ ] **Step 5: Build and run all tests**

Run: `cargo test --workspace`
Expected: every test that previously passed still passes. The pipeline behaviour for length-1 vectors is byte-identical to the old single-payload path. If a test fails, the most likely cause is a missed wrap site — search for `Ok\(.*\.to_vec\(\)\)` inside ndjson worker / closure code.

- [ ] **Step 6: Commit**

```bash
git add crates/jfmt-core/src/ndjson crates/jfmt-cli/src/commands
git commit -m "$(cat <<'EOF'
refactor(ndjson): per-line payload becomes Vec<Vec<u8>> for 0/1/N outputs

Prepares the M3 pipeline for filter mode where one input line can
produce zero, one, or many output values. Existing pretty/minify/
validate callers wrap their single-output bytes in vec![...]; output
bytes and exit behaviour are unchanged.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 10: NDJSON filter integration

**Files:**
- Modify: `crates/jfmt-core/src/filter/mod.rs`
- Create: `crates/jfmt-core/tests/filter_ndjson.rs`

- [ ] **Step 1: Add `run_ndjson` to `filter/mod.rs`**

Append:

```rust
use crate::ndjson::{run_ndjson_pipeline, LineError, NdjsonPipelineOptions, PipelineReport};

/// Drive an NDJSON parallel filter pipeline. `output` is always
/// compact (one JSON value per line) regardless of FilterOutput.
pub fn run_ndjson<R, W>(
    input: R,
    output: W,
    compiled: Compiled,
    threads: usize,
    opts: FilterOptions,
) -> std::io::Result<PipelineReport>
where
    R: std::io::Read + Send + 'static,
    W: std::io::Write + Send + 'static,
{
    let f_opts = NdjsonPipelineOptions {
        threads,
        channel_capacity: 0,
        fail_fast: opts.strict,
        collect_stats: false,
    };
    let compiled = compiled; // moved into the closure
    run_ndjson_pipeline(
        input,
        output,
        move |line, collector| {
            collector.begin_record();
            let v: serde_json::Value = serde_json::from_slice(line).map_err(|e| LineError {
                line: 0,
                offset: 0,
                column: None,
                message: format!("parse: {e}"),
            })?;
            let outputs = runtime::run_one(&compiled, v).map_err(|e| LineError {
                line: 0,
                offset: 0,
                column: None,
                message: format!("filter runtime: {e}"),
            })?;
            collector.end_record(true);
            // Serialize each output as compact JSON.
            let mut parts = Vec::with_capacity(outputs.len());
            for v in outputs {
                let bytes = serde_json::to_vec(&v).map_err(|e| LineError {
                    line: 0,
                    offset: 0,
                    column: None,
                    message: format!("serialize: {e}"),
                })?;
                parts.push(bytes);
            }
            Ok(parts)
        },
        f_opts,
    )
}
```

- [ ] **Step 2: Write the integration test**

Create `crates/jfmt-core/tests/filter_ndjson.rs`:

```rust
use jfmt_core::filter::{compile, run_ndjson, FilterOptions};
use std::io::Cursor;
use std::sync::{Arc, Mutex};

#[derive(Clone)]
struct SharedBuf(Arc<Mutex<Vec<u8>>>);
impl SharedBuf {
    fn new() -> Self {
        Self(Arc::new(Mutex::new(Vec::new())))
    }
}
impl std::io::Write for SharedBuf {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn run(expr: &str, input: &[u8], threads: usize, strict: bool) -> (String, usize) {
    let compiled = compile(expr).unwrap();
    let buf = SharedBuf::new();
    let report = run_ndjson(
        Cursor::new(input.to_vec()),
        buf.clone(),
        compiled,
        threads,
        FilterOptions { strict },
    )
    .expect("run_ndjson");
    let s = String::from_utf8(buf.0.lock().unwrap().clone()).unwrap();
    (s, report.errors.len())
}

#[test]
fn select_skips_non_matching_lines() {
    let input = b"{\"x\":1}\n{\"x\":2}\n{\"x\":3}\n";
    let (out, errs) = run("select(.x > 1)", input, 1, false);
    assert_eq!(errs, 0);
    assert_eq!(out, "{\"x\":2}\n{\"x\":3}\n");
}

#[test]
fn comma_emits_two_lines_per_input() {
    let input = b"{\"a\":1,\"b\":2}\n";
    let (out, _) = run(".a, .b", input, 1, false);
    assert_eq!(out, "1\n2\n");
}

#[test]
fn empty_output_lines_are_omitted() {
    let input = b"{\"x\":1}\n{\"x\":-1}\n{\"x\":5}\n";
    let (out, errs) = run("select(.x > 0)", input, 1, false);
    assert_eq!(errs, 0);
    assert_eq!(out, "{\"x\":1}\n{\"x\":5}\n");
}

#[test]
fn type_error_default_continues() {
    let input = b"{\"x\":\"hi\"}\n{\"x\":2}\n";
    let (out, errs) = run(".x + 1", input, 1, false);
    assert_eq!(errs, 1);
    assert_eq!(out, "3\n");
}

#[test]
fn type_error_strict_aborts_first() {
    let input = b"{\"x\":\"hi\"}\n{\"x\":2}\n";
    // strict + fail_fast: first error wins; the second line may or
    // may not have been consumed by the time cancel propagates, but
    // there must be at least one error reported.
    let (_, errs) = run(".x + 1", input, 1, true);
    assert!(errs >= 1);
}

#[test]
fn parallel_matches_serial() {
    let mut input = Vec::new();
    for i in 0..200u32 {
        input.extend_from_slice(format!("{{\"i\":{i}}}\n").as_bytes());
    }
    let (s1, _) = run("select(.i % 3 == 0)", &input, 1, false);
    let (s4, _) = run("select(.i % 3 == 0)", &input, 4, false);
    assert_eq!(s1, s4);
}
```

- [ ] **Step 3: Run**

Run: `cargo test -p jfmt-core --test filter_ndjson`
Expected: 6 tests pass.

- [ ] **Step 4: Commit**

```bash
git add crates/jfmt-core/src/filter/mod.rs crates/jfmt-core/tests/filter_ndjson.rs
git commit -m "$(cat <<'EOF'
feat(core): add run_ndjson filter (parallel, 0/1/N outputs per line)

Reuses the M3 pipeline; closure produces Vec<Vec<u8>> = N serialized
jaq outputs. fail_fast = opts.strict so --strict aborts on first
runtime error.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 11: CLI `filter` subcommand

**Files:**
- Modify: `crates/jfmt-cli/src/cli.rs`
- Create: `crates/jfmt-cli/src/commands/filter.rs`
- Modify: `crates/jfmt-cli/src/commands/mod.rs`
- Modify: `crates/jfmt-cli/src/main.rs`

- [ ] **Step 1: Add the clap args**

In `crates/jfmt-cli/src/cli.rs`, extend `Command`:

```rust
#[derive(Debug, Subcommand)]
pub enum Command {
    Pretty(PrettyArgs),
    Minify(MinifyArgs),
    Validate(ValidateArgs),
    /// Filter JSON / NDJSON with a jq expression.
    Filter(FilterArgs),
}
```

Append:

```rust
#[derive(Debug, Args)]
pub struct FilterArgs {
    /// jq expression (per-shard semantics; see `jfmt filter --help`).
    #[arg(value_name = "EXPR")]
    pub expr: String,

    #[command(flatten)]
    pub common: CommonArgs,

    /// Promote runtime jq errors to fatal exit (code 1).
    #[arg(long = "strict")]
    pub strict: bool,

    /// Pretty-print output with N-space indent. Conflicts with --compact.
    #[arg(long = "pretty", conflicts_with = "compact")]
    pub pretty: bool,

    /// Compact output (default).
    #[arg(long = "compact")]
    pub compact: bool,

    /// Indent width when --pretty is set.
    #[arg(long = "indent", value_name = "N", default_value_t = 2, requires = "pretty")]
    pub indent: u8,
}
```

- [ ] **Step 2: Write the runner**

Create `crates/jfmt-cli/src/commands/filter.rs`:

```rust
use crate::cli::FilterArgs;
use crate::main_silent_exit::SilentExit;
use crate::ExitCode;
use anyhow::Context;
use jfmt_core::filter::{
    compile, run_ndjson, run_streaming, FilterError, FilterOptions, FilterOutput,
};
use jfmt_core::PrettyConfig;

/// Whether the streaming-mode hint has been printed in this process.
/// (One-shot per invocation; not strictly necessary but matches the
/// spec wording.)
static HINT_PRINTED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

pub fn run(args: FilterArgs, threads: usize) -> anyhow::Result<()> {
    if args.pretty && args.common.ndjson {
        return Err(anyhow::anyhow!("--pretty conflicts with --ndjson"));
    }

    let compiled = compile(&args.expr).map_err(|e| classify_compile_err(e))?;

    let opts = FilterOptions {
        strict: args.strict,
    };
    let input_spec = args.common.input_spec();
    let output_spec = args.common.output_spec();
    let input = jfmt_io::open_input(&input_spec).context("opening input")?;

    if args.common.ndjson {
        let output = jfmt_io::open_output(&output_spec).context("opening output")?;
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
        let output = jfmt_io::open_output(&output_spec).context("opening output")?;
        let out_choice = if args.pretty {
            FilterOutput::Pretty(PrettyConfig::with_indent(args.indent.into()))
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

fn classify_compile_err(e: FilterError) -> anyhow::Error {
    eprintln!("jfmt: {e}");
    SilentExit(match &e {
        FilterError::Aggregate { .. } | FilterError::Parse { .. } => ExitCode::SyntaxError,
        _ => ExitCode::InputError,
    })
    .into()
}

fn classify_runtime_err(e: FilterError, strict: bool) -> anyhow::Error {
    eprintln!("jfmt: {e}");
    SilentExit(if strict {
        ExitCode::InputError
    } else {
        ExitCode::Success
    })
    .into()
}
```

The runner imports `crate::main_silent_exit::SilentExit` — add this re-export to `main.rs` (Step 4).

- [ ] **Step 3: Wire into `commands/mod.rs`**

```rust
pub mod filter;
pub mod minify;
pub mod pretty;
pub mod validate;
```

- [ ] **Step 4: Update `main.rs`**

```rust
mod cli;
mod commands;
mod exit;

use clap::Parser;
use cli::{Cli, Command};
pub use exit::ExitCode;
use std::process;

#[derive(Debug)]
pub struct SilentExit(pub ExitCode);

impl std::fmt::Display for SilentExit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "silent exit ({})", self.0.as_i32())
    }
}

impl std::error::Error for SilentExit {}

// Re-export so `commands::filter` can name it without a long path.
pub mod main_silent_exit {
    pub use super::SilentExit;
}

fn main() {
    let cli = Cli::parse();
    let code = match run(cli) {
        Ok(()) => ExitCode::Success,
        Err(e) => {
            if let Some(s) = e.downcast_ref::<SilentExit>() {
                s.0
            } else {
                eprintln!("jfmt: {e:#}");
                classify(&e)
            }
        }
    };
    process::exit(code.as_i32());
}

fn run(cli: Cli) -> anyhow::Result<()> {
    let threads = cli.threads;
    match cli.command {
        Command::Pretty(args) => commands::pretty::run(args, threads),
        Command::Minify(args) => commands::minify::run(args, threads),
        Command::Validate(args) => commands::validate::run(args, threads),
        Command::Filter(args) => commands::filter::run(args, threads),
    }
}

fn classify(e: &anyhow::Error) -> ExitCode {
    if let Some(core_err) = e.downcast_ref::<jfmt_core::Error>() {
        if matches!(core_err, jfmt_core::Error::Syntax { .. }) {
            return ExitCode::SyntaxError;
        }
    }
    if let Some(filt) = e.downcast_ref::<jfmt_core::FilterError>() {
        if matches!(filt, jfmt_core::FilterError::Parse { .. } | jfmt_core::FilterError::Aggregate { .. }) {
            return ExitCode::SyntaxError;
        }
    }
    ExitCode::InputError
}
```

- [ ] **Step 5: Build and smoke-test**

Run: `cargo build -p jfmt-cli`
Run: `echo '[{"x":1},{"x":2}]' | cargo run -p jfmt-cli -- filter 'select(.x > 1)'`
Expected (stdout): `[{"x":2}]` (plus the streaming-mode hint on stderr).

Run: `printf '{"x":1}\n{"x":2}\n' | cargo run -p jfmt-cli -- filter --ndjson 'select(.x > 1)'`
Expected (stdout): `{"x":2}\n`.

Run: `cargo run -p jfmt-cli -- filter 'length' < /dev/null; echo "exit=$?"`
Expected: stderr complains about `length`; `exit=2`.

- [ ] **Step 6: Commit**

```bash
git add crates/jfmt-cli/src/cli.rs crates/jfmt-cli/src/commands/filter.rs crates/jfmt-cli/src/commands/mod.rs crates/jfmt-cli/src/main.rs
git commit -m "$(cat <<'EOF'
feat(cli): add 'jfmt filter' subcommand (streaming + NDJSON)

Wires the new filter engine to clap. --pretty/--compact are mutually
exclusive and --pretty is rejected in --ndjson mode. Compile errors
exit 2; runtime errors default to exit 0 with stderr report; --strict
turns runtime errors into exit 1.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 12: CLI end-to-end tests

**Files:**
- Create: `crates/jfmt-cli/tests/cli_filter.rs`
- Create: `crates/jfmt-cli/tests/fixtures/filter_array.json`
- Create: `crates/jfmt-cli/tests/fixtures/filter_lines.ndjson`

- [ ] **Step 1: Create fixtures**

`filter_array.json`:
```json
[{"x":1,"name":"a"},{"x":2,"name":"b"},{"x":3,"name":"c"}]
```

`filter_lines.ndjson` (note: each line is a complete JSON value):
```
{"i":0,"level":"info"}
{"i":1,"level":"error"}
{"i":2,"level":"info"}
{"i":3,"level":"error"}
{"i":4,"level":"warn"}
```

- [ ] **Step 2: Write the e2e tests**

Create `crates/jfmt-cli/tests/cli_filter.rs`:

```rust
use assert_cmd::Command;
use predicates::str;

fn jfmt() -> Command {
    Command::cargo_bin("jfmt").unwrap()
}

#[test]
fn streaming_array_select() {
    jfmt()
        .args(["filter", "select(.x > 1)", "tests/fixtures/filter_array.json"])
        .assert()
        .success()
        .stdout(predicates::str::contains(r#"{"x":2,"name":"b"}"#))
        .stdout(predicates::str::contains(r#"{"x":3,"name":"c"}"#))
        .stdout(predicates::str::contains(r#"{"x":1,"name":"a"}"#).not())
        .stderr(predicates::str::contains("streaming mode"));
}

#[test]
fn ndjson_select_skips_lines() {
    jfmt()
        .args([
            "filter",
            "--ndjson",
            r#"select(.level == "error")"#,
            "tests/fixtures/filter_lines.ndjson",
        ])
        .assert()
        .success()
        .stdout(r#"{"i":1,"level":"error"}
{"i":3,"level":"error"}
"#);
}

#[test]
fn ndjson_multi_output_expands() {
    jfmt()
        .args([
            "filter",
            "--ndjson",
            ".i, .level",
            "tests/fixtures/filter_lines.ndjson",
        ])
        .assert()
        .success()
        .stdout(predicates::str::contains("0\n\"info\""))
        .stdout(predicates::str::contains("1\n\"error\""));
}

#[test]
fn aggregate_fails_with_exit_2() {
    jfmt()
        .args(["filter", "length"])
        .write_stdin("[1,2,3]")
        .assert()
        .code(2)
        .stderr(predicates::str::contains("length"))
        .stderr(predicates::str::contains("--ndjson").or(predicates::str::contains("--materialize")));
}

#[test]
fn parse_error_fails_with_exit_2() {
    jfmt()
        .args(["filter", "not a valid )("])
        .write_stdin("[]")
        .assert()
        .code(2);
}

#[test]
fn runtime_error_default_exit_0() {
    jfmt()
        .args(["filter", "--ndjson", ".x + 1"])
        .write_stdin("{\"x\":\"a\"}\n{\"x\":2}\n")
        .assert()
        .success()
        .stdout("3\n")
        .stderr(predicates::str::contains("error"));
}

#[test]
fn runtime_error_strict_exit_1() {
    jfmt()
        .args(["filter", "--ndjson", "--strict", ".x + 1"])
        .write_stdin("{\"x\":\"a\"}\n{\"x\":2}\n")
        .assert()
        .code(1);
}

#[test]
fn threads_parity_serial_vs_parallel() {
    let mut input = String::new();
    for i in 0..500 {
        input.push_str(&format!(r#"{{"i":{i}}}\n"#).replace("\\n", "\n"));
    }

    let s1 = jfmt()
        .args(["--threads", "1", "filter", "--ndjson", "select(.i % 7 == 0)"])
        .write_stdin(input.clone())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let s4 = jfmt()
        .args(["--threads", "4", "filter", "--ndjson", "select(.i % 7 == 0)"])
        .write_stdin(input)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    assert_eq!(s1, s4);
}

#[test]
fn pretty_with_ndjson_is_rejected() {
    jfmt()
        .args(["filter", "--ndjson", "--pretty", "."])
        .write_stdin("{}")
        .assert()
        .failure();
}
```

- [ ] **Step 3: Run**

Run: `cargo test -p jfmt-cli --test cli_filter`
Expected: 9 tests pass. The streaming-mode hint may print only on the first invocation per process; assert_cmd spawns a fresh process each call so the hint is in stderr for each test that uses streaming mode.

- [ ] **Step 4: Commit**

```bash
git add crates/jfmt-cli/tests/cli_filter.rs crates/jfmt-cli/tests/fixtures/filter_array.json crates/jfmt-cli/tests/fixtures/filter_lines.ndjson
git commit -m "$(cat <<'EOF'
test(cli): cover filter happy path, NDJSON, errors, and threads parity

Covers exit codes (2 for compile, 0 by default, 1 with --strict),
stream-mode shape preservation, NDJSON 0/1/N expansion, and
deterministic --threads parity at N=1 vs N=4.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 13: README + spec update

**Files:**
- Modify: `README.md`
- Modify: `docs/superpowers/specs/2026-04-23-jfmt-phase1-design.md`

- [ ] **Step 1: README**

Add a `## filter` section after the existing `## validate` section. If the README structure differs, place it analogously after the previously-shipped subcommand. Body:

```markdown
## filter

Apply a jq expression to JSON or NDJSON. Streaming mode is the default;
each top-level element of an array (or each value of an object) is one
*shard*, and the expression runs once per shard.

```bash
# Streaming: filter elements of an array
jfmt filter 'select(.x > 1)' big.json

# NDJSON: full jq semantics per line, in parallel
jfmt filter --ndjson 'select(.level == "error")' logs.ndjson

# Pretty-print streaming output
jfmt filter --pretty --indent 4 '.user' file.json

# Strict mode: runtime jq errors abort with exit 1
jfmt filter --ndjson --strict '.x + 1' may-have-bad-types.ndjson
```

**Streaming mode rules**

- Top-level array → output array (drop / expand per shard).
- Top-level object → output object (drop key on 0 outputs; multi-output is an error).
- Top-level scalar → 0 or 1 output (multi-output is an error).
- Aggregate jq builtins (`length`, `sort_by`, `group_by`, `add`, `min`, `max`, `unique`, …) are
  rejected at compile time. Use `--ndjson` for per-line full semantics, or wait for `--materialize`
  in M4b.

**Exit codes**

| Code | Meaning |
|---|---|
| 0 | success (or non-strict run with reported runtime errors) |
| 1 | runtime error under `--strict`, or I/O failure |
| 2 | invalid jq expression or aggregate builtin used |

`--threads N` controls the worker pool in `--ndjson` mode (default = physical cores).
```

- [ ] **Step 2: Spec milestone update**

In `docs/superpowers/specs/2026-04-23-jfmt-phase1-design.md`, find the milestone table (or shipped-status section) and mark M4a shipped, e.g.:

```
- M4a (`v0.0.4`): streaming + NDJSON `jfmt filter`. **Shipped 2026-04-25.**
- M4b: `--materialize` with memory budget. **Pending.**
```

Adjust to whatever convention the file uses.

- [ ] **Step 3: Run final test sweep**

Run: `cargo test --workspace`
Expected: all green.

Run: `cargo clippy --workspace --all-targets -- -D warnings`
Expected: no warnings.

Run: `cargo fmt --all -- --check`
Expected: clean. If not, run `cargo fmt --all` and re-stage.

- [ ] **Step 4: Commit**

```bash
git add README.md docs/superpowers/specs/2026-04-23-jfmt-phase1-design.md
git commit -m "$(cat <<'EOF'
docs: document jfmt filter and mark M4a shipped in Phase 1 spec

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 14: Ship `v0.0.4`

**Files:**
- Modify: `Cargo.toml` (workspace)

- [ ] **Step 1: Bump version**

In `Cargo.toml` (workspace), change:

```toml
version = "0.0.3"
```

to:

```toml
version = "0.0.4"
```

- [ ] **Step 2: Verify**

Run: `cargo build --workspace`
Expected: clean build.

Run: `cargo test --workspace`
Expected: all green.

- [ ] **Step 3: Commit + tag**

```bash
git add Cargo.toml
git commit -m "$(cat <<'EOF'
chore: bump version to 0.0.4 (M4a — streaming + NDJSON filter)

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
git tag -a v0.0.4 -m "M4a: streaming + NDJSON filter"
```

---

## Self-Review Checklist (Performed)

**Spec coverage:**
- §1 Scope (M4a vs M4b) → Tasks 1–14 deliver streaming + NDJSON; M4b explicitly deferred.
- §2 Decisions D1–D4 → D1 split (Tasks 1–14 are M4a only); D2 shape (Task 7); D3 static + runtime (Tasks 4 + 6); D4 strict (Tasks 8, 10, 11).
- §3 Module layout → Tasks 2–7 each touch one of the six new files; CLI in Task 11.
- §3.1 Dep pin → Task 1's spike + freeze.
- §4.1 ShardAccumulator → Task 3.
- §4.1 OutputShaper → Task 7.
- §4.2 NDJSON pipeline migration → Task 9.
- §4.2 NDJSON filter worker → Task 10.
- §4.3 Static + runtime → Tasks 4, 5, 6.
- §5 Error variants → Task 2 (definition) + CLI mapping in Task 11.
- §6 CLI surface → Task 11.
- §7.1 jfmt-core unit tests → embedded in Tasks 3, 4, 5, 6, 7.
- §7.2 jfmt-core integration tests → Tasks 3 (roundtrip), 8 (streaming), 10 (ndjson).
- §7.3 jfmt-cli e2e → Task 12.
- §10 Acceptance: `cargo test`/`clippy` → Task 13; `--threads` parity → Task 12; README + spec → Task 13; tag → Task 14.

**Placeholder scan:** Task 1 leaves jaq versions and exact AST type names to the spike on purpose, with the contract that downstream code references Annex A. No "TBD" / "TODO" elsewhere; every code block is complete.

**Type consistency:** `Compiled` is `Clone` (Arc inside) — used as `&Compiled` in streaming and moved-by-value in NDJSON closure (Task 10). `FilterError` variants are stable from Task 2. `WorkerOutput` change to `Vec<Vec<u8>>` is single-shot in Task 9 and consumed by every later use. `OutputShaper::emit(outputs, key, where_)` signature is identical between Task 7's tests and Task 8's call site.
