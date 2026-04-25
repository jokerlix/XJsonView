# jfmt M4a — Streaming + NDJSON Filter (Design)

**Status:** approved 2026-04-25 (brainstormed with user)
**Predecessor spec:** `docs/superpowers/specs/2026-04-23-jfmt-phase1-design.md` §4.1, §5.1, §5.2, §6.1
**Predecessor plan:** `docs/superpowers/plans/2026-04-25-jfmt-m3-ndjson-pipeline.md` (shipped as `v0.0.3`)
**Target tag:** `v0.0.4`

## 1. Scope

M4a delivers the first half of the Phase 1 `filter` subcommand:

```
jfmt filter EXPR [INPUT] [-o OUTPUT] [--ndjson] [--strict]
                 [--compact | --pretty [--indent N]]
```

Two execution modes:

- **Single-document streaming** — default. Per-shard jq evaluation, constant
  memory ≈ one shard.
- **NDJSON parallel** — `--ndjson` (auto on `.ndjson` / `.jsonl`). Reuses the
  M3 splitter / worker / reorder pipeline.

Both modes embed a jaq evaluator, run a static-check pass that rejects
expressions requiring whole-document semantics, and intercept
`input` / `inputs` at runtime as a safety net.

### 1.1 Out of scope (deferred to M4b)

- `-m` / `--materialize` mode and `--force` memory-budget override.
- Full-jq semantics (`length`, `sort_by`, `group_by`, aggregates, …).
- Memory-budget estimation.

### 1.2 Out of scope (Phase 1 entirely)

- Custom jq module loading (`-L`, `import`).
- Non-string-formatter IO builtins.

## 2. Design decisions

These were pinned during brainstorming; the rest of the spec assumes them.

| # | Decision |
|---|---|
| D1 | Split M4 into M4a (this spec) and M4b (`--materialize`). M4a alone unlocks the TB-scale log-filter use case from Phase 1 spec §3 use case 3. |
| D2 | Single-document streaming output is **shape-preserving**: array→array, object→object, scalar→scalar. Object/scalar inputs receiving N>1 jq outputs raise a runtime error. |
| D3 | Static check is **blacklist + runtime guard**. Spec §4.1 lists the explicit blacklist; runtime supplies an empty `inputs` iterator so missed cases fail loudly via jaq's own runtime error. Whitelist not adopted. |
| D4 | Default runtime-error policy is **skip + stderr** (continue). `--strict` promotes runtime errors to fatal exit. Mirrors the existing `validate --strict` semantics. |

## 3. Module layout

New under `crates/jfmt-core/src/filter/`:

```
filter/
  mod.rs          public API: run_streaming, run_ndjson_line; FilterError
  compile.rs      parse expression + static check → compiled jaq Filter
  static_check.rs blacklist AST scan
  runtime.rs      jaq executor wrapper; empty inputs iterator + error mapping
  shard.rs        Event ↔ serde_json::Value bridge (ShardAccumulator)
  output.rs       shape-preserving emitter (array expand, object 0/1, scalar)
```

`crates/jfmt-core/src/lib.rs` exposes `pub mod filter`.

`crates/jfmt-cli/src/cmd/filter.rs` is new, parallel to `pretty.rs` /
`minify.rs` / `validate.rs`.

### 3.1 Dependencies

Pinned with `=` in `[workspace.dependencies]`. Exact versions chosen during a
plan-phase 30-minute spike (cargo search + minimal `select(.x>0)` round-trip)
because the `jaq-*` crate split has shifted across releases.

Likely set (subject to spike confirmation):

- `jaq-core` — interpreter
- `jaq-std` — builtins (regex via the `regex` crate, transitively)
- `jaq-syn` (or `jaq-parse` on older lines) — parser → AST
- `jaq-json` (if the current line ships it) — `serde_json::Value` adapter

If the API has moved enough that the abstraction in `compile.rs` / `runtime.rs`
must change, the plan adjusts; the design above stays.

## 4. Core data flow

### 4.1 Single-document streaming

```
Read → parser::Event → ShardAccumulator → serde_json::Value
                                               ↓
                                         Filter::run (Ctx, inputs=empty)
                                               ↓
                                         0/1/N values
                                               ↓
                                         OutputShaper (per top-level form)
                                               ↓
                                         writer::pretty | writer::minify → Write
```

**ShardAccumulator** state machine:

- Reads the first non-whitespace `Event` to decide top-level form
  (`Array` / `Object` / `Scalar`).
- **Array**: each completed element emits a `Value` shard tagged with
  positional index (used only for error reporting).
- **Object**: each completed `Name + Value` pair emits a `Value` shard tagged
  with the key name. The key is preserved for output shaping and error
  reporting.
- **Scalar**: the entire document is a single shard.
- Memory upper bound = the largest shard, **not** the document.

**OutputShaper** rules (D2):

| Input form | jq outputs per shard | Behaviour |
|---|---|---|
| Array | 0 | shard is dropped from the array |
| Array | 1 | written as one element |
| Array | N>1 | expanded into N consecutive elements |
| Object | 0 | the key is dropped |
| Object | 1 | written as `key: value` |
| Object | N>1 | `FilterError::OutputShape` (runtime) — see §5 |
| Scalar | 0 | output is empty (zero bytes written). Exit 0. |
| Scalar | 1 | written as the single value |
| Scalar | N>1 | `FilterError::OutputShape` (runtime) |

The "scalar / 0 outputs" choice (empty file) matches `jq` behaviour and is
documented in `filter --help`.

### 4.2 NDJSON parallel

Reuses M3's splitter / worker / reorder pipeline unchanged structurally; the
worker gains a new operation:

```
WorkerOp::Filter { compiled: Arc<Filter> }
```

Worker per line: `parse → Value → compiled.run(value) → collect 0..N → serialize each → emit Vec<Bytes>`.

The reorder buffer's per-`seq` payload changes from a single `Bytes` to
`Vec<Bytes>`; the writer stage iterates and emits each in order. `pretty`,
`minify`, `validate` paths produce length-1 `Vec`s, preserving their existing
behaviour (verified by re-running M3's parity tests).

Runtime errors per line follow the existing M3 "bad line" path: stderr report,
skip line, continue.

### 4.3 Static check + runtime interception

`compile.rs`:

1. `jaq_syn::parse(expr)` → AST.
2. `static_check::scan(ast)` — DFS. Reject if any of the following appears:
   - Builtin call to `length`, `sort_by`, `group_by`, `add`, `min`, `max`,
     `unique`, `unique_by`, `any` (over all), `all` (over all).
   - Top-level array constructor that aggregates: `[ ... ]` containing
     comprehensions over the full input. **Heuristic only** — D3 accepts that
     this is best-effort.
   - Identifier `input` or `inputs`.
   - User-defined `def` whose body recursively references any of the above
     (single-pass; we do not chase imports because none are supported).

   On rejection: `FilterError::Aggregate { name, suggest_materialize: true }`.

3. `jaq_core::Compiler::compile(ast, defs)` → `Filter`.

`runtime.rs` calls `filter.run(Ctx { inputs: empty_iter() })` so any
static-check miss for `input` / `inputs` becomes a clean jaq runtime error,
which we map to `FilterError::Runtime`.

## 5. Errors and exit codes

`filter::FilterError` (`thiserror`):

| Variant | When raised | Default exit | `--strict` exit |
|---|---|---|---|
| `Parse { msg, span }` | jaq parser failure | 2 | 2 |
| `Aggregate { name }` | static-check blacklist hit | 2, stderr suggests `--ndjson` or `--materialize` | 2 |
| `Runtime { line, msg }` | per-shard / per-line jaq runtime error | stderr report, **continue**, exit 0 | exit 1 |
| `OutputShape { shard, reason }` | object/scalar shard yields N>1 outputs | same as `Runtime` | exit 1 |
| `Io(...)` | reader/writer IO failure | 1 | 1 |

`crates/jfmt-cli/src/exit.rs` adds a downcast arm for `FilterError`.

The "continue" semantic for `Runtime` and `OutputShape` mirrors M3's NDJSON
"bad line" handling, so a TB-scale log file with a few dirty records does not
abort the whole run unless the user opts into `--strict`.

## 6. CLI surface

```
jfmt filter EXPR [INPUT]
    [-o, --output FILE]
    [--ndjson]               # forced; auto on .ndjson / .jsonl
    [--strict]               # runtime errors fatal
    [--compact | --pretty]   # default --compact
    [--indent N]             # only with --pretty
```

- Missing `EXPR` → clap usage error.
- `INPUT` omitted or `-` → stdin.
- `-o` omitted → stdout.
- `--threads N` (already global from M3) honoured in `--ndjson` mode; ignored
  in single-document mode.
- `--compact` and `--pretty` are mutually exclusive (clap conflict group).
- In `--ndjson` mode, `--pretty` is rejected (NDJSON requires per-line compact
  output).

Compression detection (`.gz`, `.zst`) follows existing `jfmt-io` behaviour.

## 7. Testing strategy

### 7.1 jfmt-core unit tests

In each `filter/<file>.rs`:

- `compile.rs` — every blacklist entry triggers `Aggregate`; legal expressions
  compile; `inputs` rejected by static check; `input` rejected.
- `shard.rs` — top-level Array / Object / Scalar accumulator paths; nested
  containers; empty `[]` and `{}`; escaped strings; multi-byte UTF-8 keys.
- `output.rs` — array N-output expansion; object 0/1 outputs; object N>1
  error; scalar 0/1 outputs; scalar N>1 error.

### 7.2 jfmt-core integration tests (`crates/jfmt-core/tests/`)

- `filter_streaming.rs` — typical expressions (`select`, `map`, `.foo`,
  `test`); assert output equals `serde_json::Value` re-serialisation of the
  expected result.
- `filter_ndjson.rs` — pipeline path: 0/1/N expansion per line, ordering,
  single bad line stderr report.
- `filter_parity.rs` (`proptest`) — random NDJSON + per-line-safe expression;
  assert `--threads 1` output is byte-identical to `--threads 4` (M3 parity
  template).
- `shard_roundtrip.rs` (`proptest`) — `Value → Events → Value` round-trip
  equivalence; guards the ShardAccumulator implementation.

### 7.3 jfmt-cli e2e tests (`crates/jfmt-cli/tests/cli_filter.rs`)

- Happy path: `echo '[{"x":1},{"x":2}]' | jfmt filter 'select(.x > 1)'`
  → `[{"x":2}]`. Streaming mode evaluates the expression per shard, so the
  user writes `select(.x > 1)` (operating on one element), **not**
  `.[] | select(.x > 1)` (which is the materialize-style form). The CLI
  prints the streaming-mode hint to stderr once on first run, per Phase 1
  spec §6.1.
- NDJSON: file with extension `.ndjson`; `select(.level == "error")`; assert
  output line count and order.
- NDJSON multi-output: `(.a, .b)` produces 2 lines per input line; assert
  count = 2 × input lines.
- Compile error: `jfmt filter 'length'` exits 2; stderr contains
  `--materialize` suggestion.
- Runtime error default: input has one record where `.x` is a string;
  `select(.x > 0)`; stderr reports the bad shard; exit 0; output contains the
  remaining records.
- Runtime error `--strict`: same input + `--strict`; exit 1.
- `--threads` parity: NDJSON fixture filtered with `--threads 1` and
  `--threads 4`; outputs byte-identical.

### 7.4 Not tested

- jaq's own jq-language semantics. We pin `=` versions and trust upstream.
- Inputs > 1 GiB — gated behind the existing `big-tests` feature in CI only.

## 8. Risks and mitigations

1. **jaq API drift.** `jaq-parse` was folded into `jaq-syn` in 1.x; `Filter::run`
   signatures have shifted. *Mitigation:* plan starts with a 30-minute spike
   that compiles and runs a minimal `select` round-trip; freeze versions with
   `=` immediately.

2. **Incomplete static check.** A new aggregate builtin in a future jaq
   release may slip through. *Mitigation:* the runtime empty-`inputs` guard
   plus runtime-error mapping (D3); document `--strict` as the way to
   surface silent miss-evaluations; CI bumps jaq deliberately, not via
   floating versions.

3. **ShardAccumulator complexity.** Reconstructing `serde_json::Value` from
   `Event` is new code, and event-level edge cases (escapes, deeply nested
   structures, large keys) are bug-prone. *Mitigation:* `proptest`
   round-trip invariant `Value → Events → Value`; per-shard unit tests cover
   nesting, escapes, Unicode.

4. **M3 reorder buffer change.** Switching the per-`seq` payload from
   `Bytes` to `Vec<Bytes>` touches code shared by `pretty` / `minify` /
   `validate`. *Mitigation:* M3's parity test suite runs unchanged after the
   switch; non-filter operations always produce length-1 `Vec`s, so behaviour
   is identical.

## 9. Rejected alternatives

- **Uniform NDJSON-stream output for single-document mode (Q2 option b).**
  Rejected: violates the user expectation that "filtering an array yields
  an array."
- **Whitelist of jq builtins (Q3 option b).** Rejected: front-loaded effort
  is large and false-positives on legitimate expressions are user-hostile.
- **Auto-fallback from streaming to materialize when the expression is an
  aggregate.** Rejected: silently consuming TB-of-RAM is more dangerous
  than a clear error; the user must opt into `--materialize` (M4b).

## 10. Acceptance criteria for M4a

- `cargo test --workspace` green.
- `cargo clippy --workspace --all-targets -- -D warnings` clean.
- `jfmt filter EXPR file.json` works for all examples in §7.3.
- `jfmt filter EXPR file.ndjson --threads N` produces byte-identical output
  for any `N`.
- README updated with a `filter` section pointing to the streaming-mode hint
  and the `--materialize` deferral notice.
- Phase 1 spec marked: M4a shipped as `v0.0.4`; M4b still pending.
