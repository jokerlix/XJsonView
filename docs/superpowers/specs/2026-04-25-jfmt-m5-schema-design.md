# jfmt M5 — JSON Schema Validation (Design)

**Status:** approved 2026-04-25 (brainstormed with user)
**Predecessor specs:** Phase 1 design §4.1, §7.2, §7.3; M4b design `docs/superpowers/specs/2026-04-25-jfmt-m4b-materialize-design.md`
**Predecessor plan:** `docs/superpowers/plans/2026-04-25-jfmt-m4b-materialize.md` (shipped as `v0.0.5`)
**Target tag:** `v0.0.6`

## 1. Scope

M5 extends the existing `validate` subcommand with JSON Schema validation:

```
jfmt validate [INPUT] [-o OUTPUT]
              [--schema FILE] [-m | --materialize] [--force]
              [--strict] [--fail-fast] [--ndjson]
              [--stats] [--stats-json FILE]
```

- `--schema FILE`: JSON Schema file. The `jsonschema` crate auto-detects
  the draft (4 / 6 / 7 / 2019-09 / 2020-12) from the schema's `$schema`
  keyword. Without `--schema`, behaviour is identical to M2.
- The validator is applied per mode + top-level form (no
  `--schema-applies-to` flag — choice is implicit):
  - `--ndjson`: parse each line into a `serde_json::Value`, validate.
  - Default streaming + top-level array: per-element validation via
    M4a's `ShardAccumulator` (constant memory).
  - Default streaming + top-level object/scalar **with** `--schema`:
    error — "schema validation of non-array root requires
    `--materialize` or `--ndjson`".
  - `--materialize`: validate the whole document as one value.
- `validate` gains `-m` / `--materialize` and `--force` flags. Reuses the
  M4b RAM budget logic (`file_size × 6`, ×30 if compressed, abort
  unless `--force` when over 80 % of total RAM; stdin skips the check).
- Schema violations are emitted to stderr as they occur (no
  unbounded accumulation). `StatsCollector` maintains a top-N
  violation-path frequency map (default N = 10).
- `--strict` + any violation → exit 3 (`SchemaError`). Default → exit 0.
- `--fail-fast` aborts at the first syntax or schema violation,
  inheriting M2's behaviour but extending it to schema.

### 1.1 Out of scope (M5)

- Multiple `--schema` arguments (validate against several schemas).
- External `$ref` (cross-file or remote URL). Schemas may use
  *internal* `$ref` (intra-file `#/...`).
- A `--schema-draft` override flag — `$schema` keyword is enough.
- Schema source via stdin or HTTP.
- Custom keyword / format registration.

## 2. Design decisions

| # | Decision |
|---|---|
| D1 | No `--schema-applies-to` flag. Mode + top-level form decides per §1. YAGNI on the explicit override; ShardAccumulator already in tree. |
| D2 | Violations stream to stderr immediately; never accumulate. `StatsCollector` keeps a bounded top-N path frequency map. TB-safe. |
| D3 | `--strict` is **added** to `validate` in M5. M2 shipped only `--fail-fast`. The new `--strict` flag promotes any schema or syntax violation to a non-zero exit code (3 / 1 respectively) without aborting the run. Matches Phase 1 spec §7.2. |
| D4 | `validate -m` reuses M4b's RAM budget helpers (`estimate_peak_ram_bytes`, `budget_ok`, `system_total_ram_bytes`). Helpers move from `commands/filter.rs` to a shared `commands/ram_budget.rs`. |

## 3. Module layout

### 3.1 New file (`jfmt-core`)

| Path | Responsibility |
|---|---|
| `crates/jfmt-core/src/validate/schema.rs` | `SchemaValidator { compile(&Value), validate(&Value) }`; `Violation { instance_path, keyword, message }`; `SchemaError` (compile-time errors). |

### 3.2 Modified files (`jfmt-core`)

| Path | Change |
|---|---|
| `crates/jfmt-core/src/validate/mod.rs` | `pub mod schema;` + re-export `SchemaValidator`, `Violation`, `SchemaError`. |
| `crates/jfmt-core/src/validate/stats.rs` | `Stats` gains `schema_pass: u64`, `schema_fail: u64`, `top_violation_paths: BTreeMap<String, u64>`. `StatsConfig` gains `top_violation_paths_cap: usize` (default 10). `StatsCollector` gains `record_schema_outcome(passed: bool, paths: &[&str])`. `merge` extended to merge the new fields. |
| `Cargo.toml` (workspace) | Add `jsonschema = "=<X.Y.Z>"` (version frozen by Task 1's spike). |
| `crates/jfmt-core/Cargo.toml` | Pull `jsonschema`. |

### 3.3 Modified files (`jfmt-cli`)

| Path | Change |
|---|---|
| `crates/jfmt-cli/src/cli.rs` | `ValidateArgs` gains `schema: Option<PathBuf>`, `materialize: bool` (`-m`/`--materialize`, `conflicts_with = "ndjson"`), `force: bool` (`requires = "materialize"`), `strict: bool` (NEW — M2 didn't have it). |
| `crates/jfmt-cli/src/commands/validate.rs` | Branch on (`materialize`, `ndjson`, has `schema`); wire schema through each branch (NDJSON closure / streaming ShardAccumulator / materialize one-shot). |
| `crates/jfmt-cli/src/commands/ram_budget.rs` (NEW) | Houses `estimate_peak_ram_bytes`, `budget_ok`, `system_total_ram_bytes`. Moved from `commands/filter.rs`. |
| `crates/jfmt-cli/src/commands/filter.rs` | Use `super::ram_budget` instead of in-file helpers (mechanical refactor). |
| `crates/jfmt-cli/src/exit.rs` | Drop the `_` prefix on `_SchemaError`; export it as `SchemaError = 3`. |
| `crates/jfmt-cli/src/main.rs` | `classify` maps `validate::SchemaError` to `InputError`; the schema-violation-on-strict-exit-3 logic lives in `commands/validate.rs`. |

## 4. Core data flow

### 4.1 NDJSON + schema (parallel)

Reuses the M3 pipeline. Per worker:

```
parse_line → Value
           → schema.validate(&Value)
           → if !violations.empty():
                 - send each Violation to a stderr-bound channel
                 - collector.record_schema_outcome(false, &paths)
             else:
                 - collector.record_schema_outcome(true, &[])
```

`Arc<SchemaValidator>` is cloned into each worker closure (cheap Arc bump).

**Stderr ordering caveat:** M3 currently writes worker outputs to stderr
out of input order (workers run in parallel; only the stdout payload
goes through the reorder buffer). For violations to appear in input
order, the violation messages must also flow through the reorder
buffer alongside the (empty) stdout payload. Implementation detail
flagged here so Plan Task 5 doesn't lose it.

### 4.2 Default streaming (top-level array) + schema

```
EventReader → ShardAccumulator → per-element Value
                                       ↓
                                schema.validate(&Value)
                                       ↓
                              violations -> stderr (in input order)
                              + collector.record_schema_outcome
```

Memory upper bound = one shard. Top-level object/scalar with
`--schema` errors at startup with `NonArrayRootNeedsMaterialize`.

### 4.3 `--materialize` + schema

```
RAM budget pre-flight (file inputs only) → if over 80% AND !force: abort.
serde_json::from_reader → Value (whole document)
                              ↓
                       schema.validate(&Value)
                              ↓
                      violations -> stderr
                      + collector.record_schema_outcome
```

`-m` may be combined with any top-level form (root validation works on
arrays, objects, and scalars).

### 4.4 SchemaValidator interface

```rust
pub struct SchemaValidator {
    inner: Arc<jsonschema::Validator>,
}

#[derive(Debug, Clone)]
pub struct Violation {
    /// JSON Pointer into the validated value.
    pub instance_path: String,
    /// Violated keyword (e.g. "pattern", "required", "type").
    pub keyword: &'static str,
    /// Human-readable message.
    pub message: String,
}

#[derive(Debug, thiserror::Error)]
pub enum SchemaError {
    #[error("could not read schema file {path}: {source}")]
    BadSchemaFile {
        path: std::path::PathBuf,
        source: std::io::Error,
    },
    #[error("schema file is not valid JSON: {0}")]
    BadSchemaJson(#[from] serde_json::Error),
    #[error("not a valid JSON Schema: {msg}")]
    BadSchema { msg: String },
    #[error(
        "schema validation of non-array root requires --materialize \
         or --ndjson"
    )]
    NonArrayRootNeedsMaterialize,
}

impl SchemaValidator {
    pub fn compile(schema: &serde_json::Value) -> Result<Self, SchemaError>;
    pub fn validate(&self, value: &serde_json::Value) -> Vec<Violation>;
}

// SchemaValidator: Clone + Send + Sync (Arc<Validator> inside).
```

The exact mapping from `jsonschema`'s validation-error type to our
`Violation` struct is fixed by Task 1's spike (jsonschema's API
varies across releases — instance_path is sometimes a `JsonPointer`,
sometimes a `String`).

## 5. Errors and exit codes

| Variant | When | Default exit | `--strict` exit |
|---|---|---|---|
| `BadSchemaFile` / `BadSchemaJson` | startup, schema unreadable / invalid JSON | 1 | 1 |
| `BadSchema` | startup, schema is invalid JSON-Schema | 1 | 1 |
| `NonArrayRootNeedsMaterialize` | startup, mode mismatch | 1 | 1 |
| `BudgetExceeded` (M4b) | startup, `-m` over 80 % RAM, no `--force` | 1 | 1 |
| Per-record schema violation count > 0 | stderr report, **continue** | 0 | **3** (SchemaError) |
| Per-record syntax error (NDJSON) | M2: stderr + continue, exit 0 | stderr, abort, exit 2 | stderr, continue, **exit 1** if any | stderr, abort, exit 2 |
| Per-record syntax error (single doc) | M2: exit 2 immediately | exit 2 | exit 2 | exit 2 |

Note: M2's `validate` had no `--strict` flag; only `--fail-fast`. M5
introduces `--strict` (per Phase 1 spec §7.2). Default behaviour for
syntax errors is unchanged from M2. `--strict` adds the "any failure
→ non-zero exit" semantics for both syntax (exit 1) and schema
(exit 3).

`crates/jfmt-cli/src/exit.rs::ExitCode::SchemaError = 3` is already
reserved (under-prefixed name `_SchemaError`); this milestone removes
the prefix and consumes the variant.

## 6. CLI

```
jfmt validate [INPUT] [-o OUTPUT]
    [--schema FILE]
    [-m, --materialize]              # conflicts_with = "ndjson"
    [--force]                        # requires = "materialize"
    [--strict]
    [--fail-fast]
    [--ndjson]
    [--stats] [--stats-json FILE]
```

clap relations:
- `materialize.conflicts_with("ndjson")`
- `force.requires("materialize")`
- `--schema` is optional and unconstrained.

Interaction matrix:

| Scenario | Default | `--fail-fast` | `--strict` | `--fail-fast --strict` |
|---|---|---|---|---|
| Syntax fail (NDJSON line) | stderr, continue, exit 0 | stderr, abort, exit 1 | stderr, continue, exit 1 if any | stderr, abort, exit 1 |
| Schema fail | stderr, continue, exit 0 | stderr, abort, exit 3 | stderr, continue, exit 3 if any | stderr, abort, exit 3 |

## 7. Stats output

`Stats` gains three serialised fields (kept omitted when empty for
backwards compat with M2's `--stats-json` consumers):

```json
{
  "records": 1234,
  "valid": 1200,
  "invalid": 34,
  "schema_pass": 1180,
  "schema_fail": 20,
  "top_violation_paths": {
    "/address/zip": 12,
    "/email": 5,
    "/age": 3
  },
  "top_level_types": { "object": 1234 },
  "max_depth": 7,
  "top_level_keys": { ... }
}
```

`StatsConfig::top_violation_paths_cap` defaults to 10. The cap is
enforced like the existing `top_level_keys_cap`: when full, lower-frequency
entries get dropped to keep the highest counts.

Human-readable `--stats` output appends two lines under the existing
summary:

```
schema:    pass=1180  fail=20
top violation paths:
  /address/zip   12
  /email          5
  /age            3
```

## 8. Testing strategy

### 8.1 jfmt-core unit tests

- `validate/schema.rs`:
  - Compile happy path; compile rejects invalid schema with `BadSchema`.
  - `validate` returns 0 violations on a passing value.
  - `validate` returns N violations on a failing value with correct
    `instance_path` and `keyword`.
  - `Arc<Validator>` cloning works; concurrent `validate` calls from
    multiple threads produce identical results (smoke test, not stress).
- `validate/stats.rs`:
  - `record_schema_outcome(true, &[])` increments `schema_pass`.
  - `record_schema_outcome(false, &["/x"])` increments `schema_fail`
    and the path's count in `top_violation_paths`.
  - Top-N cap drops least-frequent on overflow.
  - `merge` combines per-worker schema fields correctly.

### 8.2 jfmt-core integration tests

- `validate_schema_streaming.rs`: top-level array fixture + simple schema;
  mixed pass/fail elements; assert violation count, stats fields.
- `validate_schema_ndjson.rs`: NDJSON fixture through the M3 pipeline;
  assert `--threads 1` and `--threads 4` produce byte-identical stderr
  (after sorting by line number) and identical stats.
- `validate_schema_materialize.rs`: whole-doc validation including
  aggregate keywords (`minItems`, `maxLength`, `required`).
- `validate_schema_compile.rs`: `BadSchemaFile`, `BadSchemaJson`,
  `BadSchema` error paths produce the right variant.

### 8.3 jfmt-cli e2e (append to `tests/cli_validate.rs`)

- Happy path: `jfmt validate --schema schema.json data.json` → exit 0.
- Default non-strict: NDJSON with bad records → stderr violations,
  exit 0.
- `--strict`: same → exit 3.
- `--fail-fast --strict`: stops at first violation, exit 3.
- `--stats-json`: post-violation JSON file contains
  `schema_pass`, `schema_fail`, `top_violation_paths`.
- `-m --schema`: top-level object validates root.
- Default streaming + `--schema` + top-level object stdin: exit 1,
  stderr suggests `--materialize` or `--ndjson`.
- `--threads N` parity: NDJSON + schema, N=1 vs N=4, identical
  ordered stderr violations (after sort) and identical exit code.
- `--schema /nonexistent` → exit 1, clear message.
- `--schema not-json.txt` → exit 1, "not valid JSON".
- `--schema invalid-schema.json` (valid JSON, broken schema) → exit 1,
  "not a valid JSON Schema".

## 9. Risks and mitigations

1. **`jsonschema` MSRV.** Same drill as jaq + sysinfo. Plan Task 1
   spikes the latest version that compiles on rustc 1.75 and pins.
   Fallback is to step down minor versions; a pinned 0.17 / 0.18 line
   is the realistic target if 0.20+ has marched past 1.75.

2. **`Validator: Send + Sync`.** Required for `Arc<Validator>` to share
   across NDJSON workers. Spike must verify this on the chosen version.
   If not Sync, fallback is per-worker compile (acceptable cost since
   schema compile is one-shot — it just gets paid N times instead of
   once).

3. **Default-draft for schemas without `$schema`.** Different jsonschema
   versions default differently. Spike captures the current behaviour
   in spec Annex C; documented in `--help` and README so users with
   draft-sensitive schemas know to declare `$schema` explicitly.

4. **Stderr ordering under NDJSON parallel.** Per §4.1 caveat. Plan
   Task 5 must route violations through the reorder buffer alongside
   the (empty) stdout payload — otherwise tests will see scrambled
   output. The `--threads` parity test guards this.

5. **`Validator::validate` API shape.** jsonschema returns its errors
   differently across versions (iterator of `ValidationError`, sometimes
   borrowed `instance_path`). Schema.rs normalises to our `Violation`
   struct; the conversion is local.

## 10. Acceptance criteria

- `cargo test --workspace` green.
- `cargo clippy --workspace --all-targets -- -D warnings` clean.
- `jfmt validate --schema schema.json data.ndjson` works for typical
  cases.
- `jfmt validate -m --schema schema.json file.json` works for whole-doc.
- `jfmt validate --schema schema.json --strict --ndjson bad.ndjson`
  exits 3.
- `jfmt validate --schema /nope.json data.json` exits 1 with a clear
  error.
- `--stats-json` output contains `schema_pass`, `schema_fail`,
  `top_violation_paths` when schema validation runs.
- README updated with a `### Validate — JSON Schema` block.
- Phase 1 spec marked: M5 shipped as `v0.0.6`.

## Annex C — jsonschema API mapping (frozen by Task 1 spike)

- Version: jsonschema=0.18.3.
- Compile: `jsonschema::JSONSchema::compile(&serde_json::Value) -> Result<jsonschema::JSONSchema, jsonschema::ValidationError<'static>>`.
- Validate: `jsonschema::JSONSchema::validate(&self, &'instance serde_json::Value) -> Result<(), jsonschema::ErrorIterator<'instance>>`
  where `ErrorIterator<'a> = Box<dyn Iterator<Item = ValidationError<'a>> + Sync + Send + 'a>`.
  Fast-path boolean check: `JSONSchema::is_valid(&self, &serde_json::Value) -> bool`.
- Error type: `jsonschema::ValidationError<'a>` — public fields:
  - `instance_path: jsonschema::paths::JSONPointer` (renders to JSON Pointer via `Display`/`to_string()`),
  - `schema_path: jsonschema::paths::JSONPointer`,
  - `kind: jsonschema::error::ValidationErrorKind` (variant name = keyword/category, e.g. `Type`, `Required`, `MinLength`),
  - `instance: std::borrow::Cow<'a, serde_json::Value>`.
  `Display` impl on `ValidationError` yields the human message used for `--stats-json` reporting.
- Send + Sync on `JSONSchema`: confirmed by 4-thread `Arc::clone` smoke test in spike (run 2026-04-25 on rustc 1.75.0).

The `validate/schema.rs::SchemaValidator` wraps these symbols.

### Transitive precise pins required for MSRV 1.75

`jsonschema 0.18.3` pulls `url 2.5.8 → idna 1.1.0 → idna_adapter 1.2.1`, whose
default-features path resolves to `icu_*` 2.x crates that require rustc ≥ 1.86.
The following `cargo update --precise` pins were added to `Cargo.lock` to keep
the build green on rustc 1.75:

- `idna_adapter = 1.2.0` (forces icu 1.x family instead of 2.x)
- `uuid = 1.10.0` (1.23.x needs rustc 1.85)
- `time = 0.3.36` + `deranged = 0.3.11` (0.3.44 / 0.5.x need rustc 1.85)
- `litemap = 0.7.4` + `zerofrom = 0.1.5` (0.7.5 / 0.1.7 need rustc 1.81)

Cargo.lock is committed, so these pins persist. Bumping `jsonschema` later
will require re-evaluating this list.
