# jfmt M5 — JSON Schema Validation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `jfmt validate --schema FILE` with three modes (NDJSON parallel, single-document streaming via ShardAccumulator, `--materialize`), `--strict` widening, RAM budget reuse, and stats fields. Tagged as `v0.0.6`.

**Architecture:** A new `validate::schema` module wraps `jsonschema::Validator` behind a `SchemaValidator` (Arc-shared, Send+Sync) that returns a normalized `Vec<Violation>` per value. `StatsCollector` gains schema fields. The CLI's `validate` command branches on (NDJSON / streaming-array / materialize) and threads an `Arc<SchemaValidator>` through each. Violations stream to stderr immediately (TB-safe). RAM budget helpers extract from `commands/filter.rs` to a shared `commands/ram_budget.rs`.

**Tech Stack:** Rust 2021 / MSRV 1.75 · `jsonschema` (version frozen by Task 1) · existing M3 NDJSON pipeline · M4a's `ShardAccumulator` · existing M2 `StatsCollector` · M4b's RAM budget helpers (extracted to shared module).

**Spec:** `docs/superpowers/specs/2026-04-25-jfmt-m5-schema-design.md`.

---

## File Structure

### New files

| Path | Responsibility |
|---|---|
| `crates/jfmt-core/src/validate/schema.rs` | `SchemaValidator { compile, validate }`; `Violation`; `SchemaError`. |
| `crates/jfmt-core/tests/validate_schema_streaming.rs` | Per-element streaming validation. |
| `crates/jfmt-core/tests/validate_schema_ndjson.rs` | NDJSON pipeline + schema; `--threads` parity. |
| `crates/jfmt-core/tests/validate_schema_materialize.rs` | Whole-doc validation, aggregate keywords. |
| `crates/jfmt-cli/src/commands/ram_budget.rs` | Shared `estimate_peak_ram_bytes`, `budget_ok`, `system_total_ram_bytes`. |
| `crates/jfmt-cli/tests/fixtures/schema_user.json` | Simple JSON Schema fixture. |
| `crates/jfmt-cli/tests/fixtures/schema_user_ndjson.ndjson` | NDJSON records (mix of valid + invalid). |
| `crates/jfmt-cli/tests/fixtures/schema_user_array.json` | Top-level array of records (mix). |

### Modified files

| Path | Change |
|---|---|
| `Cargo.toml` (workspace) | Add `jsonschema = "=<X.Y.Z>"` (Task 1 spike). |
| `crates/jfmt-core/Cargo.toml` | Pull `jsonschema`. |
| `crates/jfmt-core/src/validate/mod.rs` | `pub mod schema;` + re-exports. |
| `crates/jfmt-core/src/validate/stats.rs` | `Stats` gains `schema_pass`, `schema_fail`, `top_violation_paths`; `StatsConfig::top_violation_paths_cap` (default 10); `StatsCollector::record_schema_outcome`; `merge` extended; `Display` extended. |
| `crates/jfmt-cli/src/cli.rs` | `ValidateArgs` gains `schema: Option<PathBuf>`, `materialize: bool`, `force: bool`, `strict: bool`. |
| `crates/jfmt-cli/src/commands/mod.rs` | `pub mod ram_budget;`. |
| `crates/jfmt-cli/src/commands/filter.rs` | Use `super::ram_budget::*` instead of in-file helpers. Remove the now-duplicate functions from this file. |
| `crates/jfmt-cli/src/commands/validate.rs` | Three new branches when `args.schema.is_some()`; `--materialize` branch (RAM pre-flight); existing branches unchanged when no `--schema`; `--strict` exit-code logic. |
| `crates/jfmt-cli/src/exit.rs` | Rename `_SchemaError` to `SchemaError` (drop the `_` prefix). |
| `crates/jfmt-cli/src/main.rs` | `classify` already handles `jfmt_core::Error`; no change needed unless `validate::SchemaError` doesn't surface as `jfmt_core::Error` — see Task 4. |
| `crates/jfmt-cli/tests/cli_validate.rs` | Append M5 e2e tests. |
| `README.md` | New `### Validate — JSON Schema` block; update `## Status`. |
| `docs/superpowers/specs/2026-04-23-jfmt-phase1-design.md` | Mark M5 shipped. |
| `Cargo.toml` (workspace) | `version = "0.0.6"`. |

---

## Task 1: Spike & freeze `jsonschema` version

**Why:** Same MSRV 1.75 risk as jaq + sysinfo. The spike also verifies that the chosen version's `Validator` is `Send + Sync` (required for `Arc<Validator>` to be passed to NDJSON workers per spec §4.1).

**Files:**
- Create / delete: `crates/jfmt-core/examples/jsonschema_spike.rs`
- Modify: `Cargo.toml` (workspace), `crates/jfmt-core/Cargo.toml`

- [ ] **Step 1: Search jsonschema versions**

```bash
cargo search jsonschema --limit 8
```

Pick the highest version that compiles on rustc 1.75 AND keeps `Validator: Send + Sync`. Start with the highest; step down on edition2024 errors. Latest as of writing is on the 0.20+ line; if it requires Rust 1.85, step to 0.18.x or 0.17.x.

- [ ] **Step 2: Add provisional dep**

Edit `Cargo.toml` (workspace), add to `[workspace.dependencies]`:

```toml
# JSON Schema validation (M5).
jsonschema = "=<X.Y.Z>"
```

Edit `crates/jfmt-core/Cargo.toml`, add under `[dependencies]`:

```toml
jsonschema = { workspace = true }
```

- [ ] **Step 3: Write the spike**

Create `crates/jfmt-core/examples/jsonschema_spike.rs`:

```rust
//! M5 spike — verify three things:
//! 1. jsonschema compiles on MSRV 1.75 with the chosen version.
//! 2. Validator is Send + Sync (so Arc<Validator> works across NDJSON workers).
//! 3. Capture the exact compile + validate API used by Task 4.

use std::sync::Arc;
use std::thread;

fn main() {
    // (1) Compile a tiny schema. The actual compile API varies by
    //     version — common shapes:
    //     - jsonschema::JSONSchema::compile(&schema_value) -> Result<JSONSchema, ...>
    //     - jsonschema::Validator::new(&schema_value) -> Result<Validator, ...>
    //     - jsonschema::validator_for(&schema_value) -> Result<Validator, ...>
    //
    //     Use whichever the chosen version exposes. Document in Annex C.
    let schema = serde_json::json!({
        "type": "object",
        "required": ["x"],
        "properties": {"x": {"type": "integer"}}
    });

    // Replace the literal call below with the real compile entry point.
    let validator = jsonschema::validator_for(&schema)
        .expect("compile schema");

    // (2) Validate two values: one passes, one fails.
    let ok = serde_json::json!({"x": 42});
    let bad = serde_json::json!({"x": "not an int"});

    let pass: Vec<_> = validator.iter_errors(&ok).collect();
    let fail: Vec<_> = validator.iter_errors(&bad).collect();
    assert!(pass.is_empty(), "ok value should pass");
    assert!(!fail.is_empty(), "bad value should fail");

    // (3) Smoke-test Send+Sync via Arc::clone across threads.
    let shared = Arc::new(validator);
    let handles: Vec<_> = (0..4)
        .map(|_| {
            let v = Arc::clone(&shared);
            let bad = bad.clone();
            thread::spawn(move || {
                let errs: Vec<_> = v.iter_errors(&bad).collect();
                assert!(!errs.is_empty());
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }

    println!("spike OK");
}
```

The exact API names depend on the chosen jsonschema version. Read
`cargo doc -p jsonschema` after adding the dep, or inspect the
crate's published examples on docs.rs. The shape is fixed:

- **Compile**: takes `&serde_json::Value`, returns a Validator (or
  Result thereof).
- **Validate**: returns an iterator (or `Vec`) of validation errors,
  each of which exposes the JSON Pointer path and a keyword/category.

If the chosen version doesn't compile on 1.75 OR `Validator` isn't
`Send + Sync`, step down and retry. If neither holds for any
recent-enough version (you've gone below 0.16), STOP and report
BLOCKED.

- [ ] **Step 4: Run the spike**

Run: `cargo run -p jfmt-core --example jsonschema_spike`
Expected: prints `spike OK`. If the assertions fail, fix the spike (the goal is to learn the real API), don't fudge the assertions.

- [ ] **Step 5: Capture API mapping in Annex C**

Append to `docs/superpowers/specs/2026-04-25-jfmt-m5-schema-design.md`:

```markdown
## Annex C — jsonschema API mapping (frozen by Task 1 spike)

- Version: jsonschema=<X.Y.Z>.
- Compile: `<actual function path>(&serde_json::Value) -> Result<<Validator type>, <error type>>`.
- Validate: `<validator method>(&value) -> <iterator or Vec of error type>`.
- Error type: `<full path>` (with fields/methods to extract instance_path, keyword, message).
- Send + Sync on Validator: confirmed by 4-thread smoke test in spike.

The `validate/schema.rs::SchemaValidator` wraps these symbols.
```

Replace each `<…>` with the real symbols.

- [ ] **Step 6: Delete the example**

Run:
```bash
rm crates/jfmt-core/examples/jsonschema_spike.rs
# also remove the directory if it's now empty:
rmdir crates/jfmt-core/examples 2>/dev/null || true
```

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml Cargo.lock crates/jfmt-core/Cargo.toml docs/superpowers/specs/2026-04-25-jfmt-m5-schema-design.md
git commit -m "$(cat <<'EOF'
chore(deps): add jsonschema pinned for M5 schema validation

Version frozen via spike (see spec Annex C). MSRV-1.75 verified;
Validator confirmed Send + Sync via 4-thread Arc::clone smoke test,
then example deleted.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 2: Extract RAM budget helpers to shared module

Pure refactor. The three helpers in `commands/filter.rs` become public-in-module functions in a new `commands/ram_budget.rs`. Filter (M4b) uses the new module; Task 5's `validate -m` will reuse it.

**Files:**
- Create: `crates/jfmt-cli/src/commands/ram_budget.rs`
- Modify: `crates/jfmt-cli/src/commands/filter.rs` (remove helpers, import from `super::ram_budget`)
- Modify: `crates/jfmt-cli/src/commands/mod.rs` (add `pub mod ram_budget;`)

- [ ] **Step 1: Create `commands/ram_budget.rs`**

```rust
//! Pre-flight RAM budget for `--materialize` modes. Used by both
//! `commands::filter` (M4b) and `commands::validate` (M5).

/// Estimate peak RAM for materializing `input`. Returns `None` when
/// the input is stdin or its size can't be determined — callers
/// interpret `None` as "skip the check" per spec D3.
pub(super) fn estimate_peak_ram_bytes(spec: &jfmt_io::InputSpec) -> Option<u64> {
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
pub(super) fn budget_ok(estimate: u64, total_ram: u64) -> bool {
    // 80% = total_ram * 4 / 5. Compute as `total_ram / 5 * 4` to
    // reduce overflow risk on very large `total_ram` values.
    estimate < total_ram / 5 * 4
}

/// Query the actual system total RAM. Wraps sysinfo per spec Annex B.
pub(super) fn system_total_ram_bytes() -> u64 {
    let mut sys = sysinfo::System::new();
    sys.refresh_memory();
    sys.total_memory()
}

#[cfg(test)]
mod tests {
    use super::budget_ok;

    #[test]
    fn budget_ok_under_80_percent() {
        assert!(budget_ok(1 << 30, 2u64 << 30));
    }

    #[test]
    fn budget_not_ok_over_80_percent() {
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

- [ ] **Step 2: Add module to `commands/mod.rs`**

```rust
pub mod filter;
pub mod minify;
pub mod pretty;
pub mod ram_budget;
pub mod validate;
```

(Keep alphabetical; the existing file may or may not be — match what's there.)

- [ ] **Step 3: Update `commands/filter.rs`**

Find the three functions `estimate_peak_ram_bytes`, `budget_ok`, `system_total_ram_bytes` (around lines 132–161 in current state, plus the `#[cfg(test)] mod tests` block immediately after them). **Delete all four blocks** from `filter.rs`.

At the call sites inside `pub fn run(...)`, change:

```rust
            if let Some(estimate) = estimate_peak_ram_bytes(&input_spec) {
                let total = system_total_ram_bytes();
                if !budget_ok(estimate, total) {
```

to:

```rust
            if let Some(estimate) = super::ram_budget::estimate_peak_ram_bytes(&input_spec) {
                let total = super::ram_budget::system_total_ram_bytes();
                if !super::ram_budget::budget_ok(estimate, total) {
```

(That's the only call site in filter.rs; verify with grep.)

- [ ] **Step 4: Build + tests**

Run: `cargo build --workspace 2>&1 | tail -3`
Expected: clean.

Run: `cargo test --workspace 2>&1 | grep -E "test result:" | head -25`
Expected: every test still passes (including the 3 budget unit tests, now under `commands::ram_budget::tests`).

Run: `cargo clippy --workspace --all-targets -- -D warnings 2>&1 | tail -3`
Expected: clean. If clippy complains about `pub(super)` visibility on `ram_budget` items being used by both `filter` and `validate` (siblings under `commands/`), that's correct: `pub(super)` makes them visible to all modules under `commands/`. If for some reason it doesn't, change to `pub(crate)`.

- [ ] **Step 5: Commit**

```bash
git add crates/jfmt-cli/src/commands/
git commit -m "$(cat <<'EOF'
refactor(cli): move RAM budget helpers to shared commands::ram_budget

estimate_peak_ram_bytes / budget_ok / system_total_ram_bytes were
in commands::filter only; M5's validate --materialize needs them too.
No behaviour change; helpers are now pub(super) and consumed by both
filter and (next task) validate.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 3: Stats fields for schema validation

Add `schema_pass`, `schema_fail`, `top_violation_paths` to `Stats`; `top_violation_paths_cap` to `StatsConfig`; `record_schema_outcome` to `StatsCollector`; extend `merge` and `Display`.

**Files:**
- Modify: `crates/jfmt-core/src/validate/stats.rs`

- [ ] **Step 1: Write failing tests**

In `crates/jfmt-core/src/validate/stats.rs`, find the existing `#[cfg(test)] mod tests` block (near the bottom, around lines 252+). Append (inside the existing `mod tests`) these tests:

```rust
    #[test]
    fn record_schema_outcome_pass_increments_schema_pass() {
        let mut c = StatsCollector::default();
        c.record_schema_outcome(true, &[]);
        let s = c.finish();
        assert_eq!(s.schema_pass, 1);
        assert_eq!(s.schema_fail, 0);
        assert!(s.top_violation_paths.is_empty());
    }

    #[test]
    fn record_schema_outcome_fail_increments_paths() {
        let mut c = StatsCollector::default();
        c.record_schema_outcome(false, &["/x", "/y/z"]);
        let s = c.finish();
        assert_eq!(s.schema_pass, 0);
        assert_eq!(s.schema_fail, 1);
        assert_eq!(s.top_violation_paths.get("/x"), Some(&1));
        assert_eq!(s.top_violation_paths.get("/y/z"), Some(&1));
    }

    #[test]
    fn record_schema_outcome_paths_accumulate() {
        let mut c = StatsCollector::default();
        c.record_schema_outcome(false, &["/x"]);
        c.record_schema_outcome(false, &["/x", "/y"]);
        c.record_schema_outcome(false, &["/x"]);
        let s = c.finish();
        assert_eq!(s.schema_fail, 3);
        assert_eq!(s.top_violation_paths.get("/x"), Some(&3));
        assert_eq!(s.top_violation_paths.get("/y"), Some(&1));
    }

    #[test]
    fn top_violation_paths_cap_drops_least_frequent() {
        let cfg = StatsConfig {
            top_violation_paths_cap: 2,
            ..StatsConfig::default()
        };
        let mut c = StatsCollector::new(cfg);
        // /a hit 3 times, /b hit 2 times, /c hit 1 time. Cap is 2 so /c gets dropped.
        for _ in 0..3 {
            c.record_schema_outcome(false, &["/a"]);
        }
        for _ in 0..2 {
            c.record_schema_outcome(false, &["/b"]);
        }
        c.record_schema_outcome(false, &["/c"]);
        let s = c.finish();
        assert_eq!(s.top_violation_paths.len(), 2);
        assert!(s.top_violation_paths.contains_key("/a"));
        assert!(s.top_violation_paths.contains_key("/b"));
        assert!(!s.top_violation_paths.contains_key("/c"));
    }

    #[test]
    fn merge_combines_schema_fields() {
        let mut a = Stats {
            schema_pass: 5,
            schema_fail: 2,
            ..Default::default()
        };
        a.top_violation_paths.insert("/x".into(), 2);
        let mut b = Stats {
            schema_pass: 3,
            schema_fail: 4,
            ..Default::default()
        };
        b.top_violation_paths.insert("/x".into(), 1);
        b.top_violation_paths.insert("/y".into(), 3);
        a.merge(b);
        assert_eq!(a.schema_pass, 8);
        assert_eq!(a.schema_fail, 6);
        assert_eq!(a.top_violation_paths.get("/x"), Some(&3));
        assert_eq!(a.top_violation_paths.get("/y"), Some(&3));
    }
```

- [ ] **Step 2: Run failing tests**

Run: `cargo test -p jfmt-core --lib validate::stats 2>&1 | tail -10`
Expected: tests fail because `record_schema_outcome` doesn't exist and `Stats` is missing the new fields.

- [ ] **Step 3: Extend `Stats`, `StatsConfig`, `StatsCollector`**

In `validate/stats.rs`:

(a) Add fields to `Stats` (after `top_level_keys_truncated`):

```rust
    /// Number of records that passed schema validation. Only non-zero
    /// when `--schema` was used.
    #[serde(skip_serializing_if = "is_zero_u64")]
    pub schema_pass: u64,
    /// Number of records that failed schema validation.
    #[serde(skip_serializing_if = "is_zero_u64")]
    pub schema_fail: u64,
    /// Top-N most frequent violated JSON Pointer paths (capped via
    /// `StatsConfig::top_violation_paths_cap`).
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub top_violation_paths: BTreeMap<String, u64>,
```

(b) Extend `Stats::merge`:

```rust
        self.schema_pass += other.schema_pass;
        self.schema_fail += other.schema_fail;
        for (k, v) in other.top_violation_paths {
            *self.top_violation_paths.entry(k).or_insert(0) += v;
        }
```

(Add inside the existing `merge` body.)

(c) Add field to `StatsConfig`:

```rust
pub struct StatsConfig {
    pub top_level_keys_cap: usize,
    /// Maximum distinct violated JSON Pointer paths to retain. Once
    /// hit, paths with the lowest counts are evicted on each
    /// subsequent insert. Default 10.
    pub top_violation_paths_cap: usize,
}

impl Default for StatsConfig {
    fn default() -> Self {
        Self {
            top_level_keys_cap: 1024,
            top_violation_paths_cap: 10,
        }
    }
}
```

(d) Add the recording method to `impl StatsCollector` (place near `observe`):

```rust
    /// Record one record's schema-validation outcome. `paths` are the
    /// JSON Pointer paths of the violations (empty when `passed`).
    pub fn record_schema_outcome(&mut self, passed: bool, paths: &[&str]) {
        if passed {
            self.stats.schema_pass += 1;
        } else {
            self.stats.schema_fail += 1;
        }
        for p in paths {
            *self
                .stats
                .top_violation_paths
                .entry((*p).to_string())
                .or_insert(0) += 1;
        }
        // Enforce cap by evicting lowest-frequency entries until we
        // are at or below cap. Stable tie-break by key (alphabetical).
        let cap = self.cfg.top_violation_paths_cap;
        if cap > 0 {
            while self.stats.top_violation_paths.len() > cap {
                // Find the lowest-count entry; on tie, the lex-largest
                // key (so we keep alphabetically-earlier-named paths,
                // a deterministic tie-break).
                let evict = self
                    .stats
                    .top_violation_paths
                    .iter()
                    .min_by(|a, b| a.1.cmp(b.1).then_with(|| b.0.cmp(a.0)))
                    .map(|(k, _)| k.clone());
                if let Some(k) = evict {
                    self.stats.top_violation_paths.remove(&k);
                } else {
                    break;
                }
            }
        }
    }
```

(e) Extend `Display for Stats` to print the new fields when non-zero. After the existing `top_level_keys` block, append:

```rust
        if self.schema_pass + self.schema_fail > 0 {
            writeln!(
                f,
                "schema:    pass={}  fail={}",
                self.schema_pass, self.schema_fail
            )?;
        }
        if !self.top_violation_paths.is_empty() {
            writeln!(f, "top violation paths:")?;
            let mut paths: Vec<_> = self.top_violation_paths.iter().collect();
            paths.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
            for (k, v) in paths {
                writeln!(f, "  {k}   {v}")?;
            }
        }
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p jfmt-core --lib validate::stats 2>&1 | tail -10`
Expected: all 5 new tests pass plus existing tests.

- [ ] **Step 5: Run workspace**

Run: `cargo test --workspace 2>&1 | grep -E "test result:" | head -25`
Expected: every prior test still passes.

- [ ] **Step 6: Clippy**

Run: `cargo clippy --workspace --all-targets -- -D warnings 2>&1 | tail -3`
Expected: clean.

- [ ] **Step 7: Commit**

```bash
git add crates/jfmt-core/src/validate/stats.rs
git commit -m "$(cat <<'EOF'
feat(core): add schema_pass/schema_fail/top_violation_paths to Stats

Extends StatsCollector with record_schema_outcome(passed, paths) for
M5 schema validation. top_violation_paths is capped via
StatsConfig::top_violation_paths_cap (default 10); over-cap evicts
lowest-frequency entries with deterministic tie-break. merge and
Display extended; serde rename_all/skip_serializing_if matches the
existing pattern.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 4: `SchemaValidator` + `SchemaError`

Compile a JSON Schema, hold it in `Arc<jsonschema::Validator>`, expose `validate(&Value) -> Vec<Violation>`. Errors at compile time are surfaced via `SchemaError` (file IO, bad JSON, bad schema).

**Files:**
- Create: `crates/jfmt-core/src/validate/schema.rs`
- Modify: `crates/jfmt-core/src/validate/mod.rs`

- [ ] **Step 1: Wire module + re-exports**

In `crates/jfmt-core/src/validate/mod.rs`, add `pub mod schema;` after `pub mod syntax;`. Append to the existing `pub use` block:

```rust
pub use schema::{SchemaError, SchemaValidator, Violation};
```

- [ ] **Step 2: Create `schema.rs` with failing tests inline**

Create `crates/jfmt-core/src/validate/schema.rs`:

```rust
//! JSON Schema validation. Wraps the `jsonschema` crate behind a
//! Send+Sync, Arc-shareable `SchemaValidator`. Normalises validation
//! errors into our `Violation` struct. See spec §4.4 + Annex C.

use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;
use thiserror::Error;

/// One validation violation. The `instance_path` is the JSON Pointer
/// inside the *validated value*, not the schema; the `keyword` is the
/// jsonschema rule that failed (e.g., "type", "required", "pattern").
#[derive(Debug, Clone, PartialEq)]
pub struct Violation {
    pub instance_path: String,
    pub keyword: &'static str,
    pub message: String,
}

#[derive(Debug, Error)]
pub enum SchemaError {
    #[error("could not read schema file {path:?}: {source}")]
    BadSchemaFile {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("schema file is not valid JSON: {0}")]
    BadSchemaJson(#[from] serde_json::Error),

    #[error("not a valid JSON Schema: {msg}")]
    BadSchema { msg: String },
}

/// Compiled, shareable schema validator. `Clone` is cheap (Arc bump).
#[derive(Clone)]
pub struct SchemaValidator {
    inner: Arc<jsonschema::Validator>,
}

impl SchemaValidator {
    /// Compile a schema from a parsed `serde_json::Value`.
    pub fn compile(schema: &Value) -> Result<Self, SchemaError> {
        // Replace this call with the symbol frozen by Annex C.
        let v = jsonschema::validator_for(schema)
            .map_err(|e| SchemaError::BadSchema { msg: format!("{e}") })?;
        Ok(Self {
            inner: Arc::new(v),
        })
    }

    /// Validate one value. Returns 0..N violations.
    pub fn validate(&self, value: &Value) -> Vec<Violation> {
        // Replace `iter_errors` with the iterator method frozen by
        // Annex C if the version's API differs.
        self.inner
            .iter_errors(value)
            .map(|e| Violation {
                instance_path: e.instance_path.to_string(),
                keyword: keyword_name(&e),
                message: format!("{e}"),
            })
            .collect()
    }
}

/// Normalise the jsonschema error category into a stable `&'static str`
/// keyword. Different jsonschema versions expose this differently;
/// adjust to whatever Annex C captured.
fn keyword_name(err: &jsonschema::ValidationError) -> &'static str {
    // Most versions expose `err.kind` as an enum; map common variants
    // to keyword names. Anything else falls back to "schema".
    use jsonschema::error::ValidationErrorKind as K;
    match &err.kind {
        K::Type { .. } => "type",
        K::Required { .. } => "required",
        K::Pattern { .. } => "pattern",
        K::AdditionalProperties { .. } => "additionalProperties",
        K::Enum { .. } => "enum",
        K::Format { .. } => "format",
        K::Minimum { .. } => "minimum",
        K::Maximum { .. } => "maximum",
        K::MinLength { .. } => "minLength",
        K::MaxLength { .. } => "maxLength",
        K::MinItems { .. } => "minItems",
        K::MaxItems { .. } => "maxItems",
        K::UniqueItems => "uniqueItems",
        K::OneOfNotValid | K::OneOfMultipleValid => "oneOf",
        K::AnyOf => "anyOf",
        K::AllOf => "allOf",
        K::Not { .. } => "not",
        K::Const { .. } => "const",
        _ => "schema",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn user_schema() -> Value {
        json!({
            "type": "object",
            "required": ["name", "age"],
            "properties": {
                "name": {"type": "string"},
                "age": {"type": "integer", "minimum": 0}
            }
        })
    }

    #[test]
    fn compile_happy_path() {
        SchemaValidator::compile(&user_schema()).expect("compile");
    }

    #[test]
    fn compile_rejects_invalid_schema() {
        // A schema whose `type` keyword's value is itself the wrong shape.
        let bad = json!({"type": 42});
        assert!(matches!(
            SchemaValidator::compile(&bad),
            Err(SchemaError::BadSchema { .. })
        ));
    }

    #[test]
    fn validate_passing_value_returns_empty() {
        let v = SchemaValidator::compile(&user_schema()).unwrap();
        let value = json!({"name": "alice", "age": 30});
        let violations = v.validate(&value);
        assert!(violations.is_empty(), "expected no violations: {violations:?}");
    }

    #[test]
    fn validate_failing_value_reports_violation() {
        let v = SchemaValidator::compile(&user_schema()).unwrap();
        let value = json!({"name": "bob"}); // missing `age`
        let violations = v.validate(&value);
        assert!(!violations.is_empty());
        // "required" keyword should appear somewhere in the violations.
        assert!(violations.iter().any(|x| x.keyword == "required"));
    }

    #[test]
    fn validate_reports_instance_path() {
        let v = SchemaValidator::compile(&user_schema()).unwrap();
        let value = json!({"name": "carol", "age": -5}); // age < minimum
        let violations = v.validate(&value);
        assert!(!violations.is_empty());
        let by_keyword: Vec<_> = violations
            .iter()
            .filter(|x| x.keyword == "minimum")
            .collect();
        assert!(!by_keyword.is_empty());
        // Instance path should reference the offending field.
        assert!(by_keyword[0].instance_path.contains("age"));
    }

    #[test]
    fn arc_clone_works_across_threads() {
        use std::sync::Arc as StdArc;
        use std::thread;

        let v = SchemaValidator::compile(&user_schema()).unwrap();
        let shared = StdArc::new(v);
        let bad = json!({"name": "x"}); // missing age

        let handles: Vec<_> = (0..4)
            .map(|_| {
                let v = StdArc::clone(&shared);
                let bad = bad.clone();
                thread::spawn(move || {
                    let violations = v.validate(&bad);
                    assert!(!violations.is_empty());
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
    }
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p jfmt-core --lib validate::schema 2>&1 | tail -15`
Expected: 6 tests pass.

If `jsonschema::validator_for` isn't the actual entry point in the chosen version, replace with the symbol in Annex C. Same for `iter_errors` and `ValidationErrorKind`. The contract — compile + validate returning per-field violations — is fixed; the spelling is per the chosen version.

If `keyword_name` panics on an unknown variant (because the chosen version has new variants beyond what's listed), the `_ => "schema"` catch-all handles it. Don't add `#[non_exhaustive]` warnings — clippy might complain about pattern coverage but the catch-all answers it.

- [ ] **Step 4: Clippy**

Run: `cargo clippy --workspace --all-targets -- -D warnings 2>&1 | tail -3`
Expected: clean. If clippy flags `keyword_name` for non-exhaustive matches under `-D warnings`, the catch-all already answers it; ensure no `#[deny(...)]` attribute escalates this further.

- [ ] **Step 5: Commit**

```bash
git add crates/jfmt-core/src/validate/schema.rs crates/jfmt-core/src/validate/mod.rs
git commit -m "$(cat <<'EOF'
feat(core): add SchemaValidator + SchemaError + Violation

Wraps jsonschema::Validator behind an Arc-shareable Send+Sync
SchemaValidator. compile() takes &serde_json::Value; validate(&Value)
returns Vec<Violation> with normalised keyword names and JSON Pointer
instance paths. SchemaError covers file/JSON/schema-shape failures
for the CLI to map to exit code 1.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 5: CLI flags + three-mode wiring + violations to stderr

Add `--schema`, `-m`/`--materialize`, `--force`, `--strict` to `ValidateArgs`. Implement three branches:
1. `--ndjson` + schema: closure runs schema per parsed Value, emits violations through reorder buffer for in-order stderr.
2. Default streaming + schema + top-level array: ShardAccumulator drives per-element validation.
3. `--materialize` + schema: serde_json::from_reader → single validate call.

Plus the no-schema branches stay byte-identical to M2.

**Files:**
- Modify: `crates/jfmt-cli/src/cli.rs`
- Modify: `crates/jfmt-cli/src/commands/validate.rs`
- Modify: `crates/jfmt-cli/src/exit.rs`

- [ ] **Step 1: Add `ValidateArgs` fields**

In `crates/jfmt-cli/src/cli.rs`, find `pub struct ValidateArgs`. After the existing `fail_fast: bool` field, add:

```rust
    /// JSON Schema file to validate each record against.
    #[arg(long = "schema", value_name = "FILE")]
    pub schema: Option<std::path::PathBuf>,

    /// Materialize the whole input and validate as a single value.
    /// Conflicts with --ndjson.
    #[arg(short = 'm', long = "materialize", conflicts_with = "ndjson")]
    pub materialize: bool,

    /// Skip the RAM budget pre-flight check. Requires --materialize.
    #[arg(long = "force", requires = "materialize")]
    pub force: bool,

    /// Promote any failure (syntax or schema) to a non-zero exit code
    /// without aborting the run. Syntax → exit 1; schema → exit 3.
    #[arg(long = "strict")]
    pub strict: bool,
```

- [ ] **Step 2: Drop the `_` prefix on `SchemaError` in `exit.rs`**

In `crates/jfmt-cli/src/exit.rs`:

```rust
#[repr(i32)]
#[derive(Debug, Clone, Copy)]
pub enum ExitCode {
    Success = 0,
    InputError = 1,
    SyntaxError = 2,
    SchemaError = 3,
}
```

(Remove the `_` prefix; the variant is now in use.)

If anything else in the codebase referenced `_SchemaError`, grep it and rename. (M2 only ever defined this, never used it.)

- [ ] **Step 3: Restructure `commands/validate.rs`**

Replace the body of `pub fn run(...)` with the three-mode dispatcher. The full updated file (replace existing contents):

```rust
use crate::cli::ValidateArgs;
use crate::exit::ExitCode;
use crate::SilentExit;
use anyhow::{Context, Result};
use jfmt_core::filter::shard::{ShardAccumulator, ShardLocator, TopLevel};
use jfmt_core::parser::EventReader;
use jfmt_core::validate::SchemaValidator;
use jfmt_core::{
    run_ndjson_pipeline, validate_syntax, Error, LineError, NdjsonPipelineOptions, Stats,
    StatsCollector,
};
use std::fs::File;
use std::io::{BufWriter, Write};
use std::sync::Arc;

pub fn run(args: ValidateArgs, threads: usize) -> Result<()> {
    let collect_stats = args.stats || args.stats_json.is_some();

    // Compile the schema, if any. Compile errors -> exit 1.
    let schema = if let Some(path) = &args.schema {
        Some(Arc::new(load_schema(path)?))
    } else {
        None
    };

    let input_spec = args.common.input_spec();

    // Materialize branch (with optional schema). RAM pre-flight if file input.
    if args.materialize {
        if !args.force {
            if let Some(estimate) = super::ram_budget::estimate_peak_ram_bytes(&input_spec) {
                let total = super::ram_budget::system_total_ram_bytes();
                if !super::ram_budget::budget_ok(estimate, total) {
                    eprintln!(
                        "jfmt: estimated peak memory {} bytes exceeds 80% of total RAM ({} bytes); rerun with --force to override",
                        estimate, total
                    );
                    return Err(SilentExit(ExitCode::InputError).into());
                }
            }
        }
        let input = jfmt_io::open_input(&input_spec).context("opening input")?;
        return run_materialize(input, schema, args, collect_stats);
    }

    let input = jfmt_io::open_input(&input_spec).context("opening input")?;
    if args.common.ndjson {
        run_ndjson(input, schema, args, threads, collect_stats)
    } else {
        run_streaming(input, schema, args, collect_stats)
    }
}

fn load_schema(path: &std::path::Path) -> Result<SchemaValidator> {
    use jfmt_core::validate::SchemaError;
    let bytes = std::fs::read(path).map_err(|e| SchemaError::BadSchemaFile {
        path: path.to_path_buf(),
        source: e,
    })?;
    let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(SchemaError::from)?;
    SchemaValidator::compile(&value).map_err(anyhow::Error::from)
}

fn run_ndjson<R: std::io::Read + Send + 'static>(
    input: R,
    schema: Option<Arc<SchemaValidator>>,
    args: ValidateArgs,
    threads: usize,
    collect_stats: bool,
) -> Result<()> {
    let sink = std::io::sink();
    let opts = NdjsonPipelineOptions {
        threads,
        fail_fast: args.fail_fast,
        collect_stats,
        ..Default::default()
    };
    let schema = schema; // moved into closure
    let closure = move |line: &[u8], c: &mut StatsCollector| -> Result<Vec<Vec<u8>>, LineError> {
        c.begin_record();
        // (1) Syntax pass via EventReader (M2 behaviour).
        let mut p = EventReader::new(line);
        loop {
            match p.next_event() {
                Ok(None) => break,
                Ok(Some(ev)) => c.observe(&ev),
                Err(Error::Syntax {
                    offset,
                    column,
                    message,
                    ..
                }) => {
                    c.end_record(false);
                    return Err(LineError {
                        line: 0,
                        offset,
                        column,
                        message,
                    });
                }
                Err(e) => {
                    c.end_record(false);
                    return Err(LineError {
                        line: 0,
                        offset: 0,
                        column: None,
                        message: format!("{e}"),
                    });
                }
            }
        }
        if let Err(Error::Syntax {
            offset,
            column,
            message,
            ..
        }) = p.finish()
        {
            c.end_record(false);
            return Err(LineError {
                line: 0,
                offset,
                column,
                message,
            });
        }
        c.end_record(true);
        // (2) Schema pass (only if schema present). Re-parse line as
        //     serde_json::Value (the EventReader pass above rejected
        //     malformed input, so we know it parses now).
        if let Some(s) = schema.as_ref() {
            let value: serde_json::Value = serde_json::from_slice(line).map_err(|e| LineError {
                line: 0,
                offset: 0,
                column: None,
                message: format!("post-syntax JSON re-parse failed: {e}"),
            })?;
            let violations = s.validate(&value);
            let paths: Vec<&str> =
                violations.iter().map(|v| v.instance_path.as_str()).collect();
            c.record_schema_outcome(violations.is_empty(), &paths);
            if !violations.is_empty() {
                // Encode violations as the "ok bytes" payload — the
                // reorder buffer will flush them in input order. Each
                // violation gets its own line via reorder's `\n`
                // appender. We tag them as 'schema:' so stderr-side
                // post-processing can distinguish from regular ndjson
                // output (none, in validate's case — sink is /dev/null).
                let mut parts = Vec::with_capacity(violations.len());
                for v in &violations {
                    parts.push(
                        format!(
                            "schema: {}: {}: {}",
                            v.instance_path, v.keyword, v.message
                        )
                        .into_bytes(),
                    );
                }
                return Ok(parts);
            }
        }
        Ok(vec![Vec::new()])
    };

    // Tee the pipeline output to stderr instead of /dev/null so schema
    // violations print in input order. We do this with a small
    // adapter: a Write impl that prepends "line N: " using a counter.
    // Simpler approach: collect schema-violation bytes in a custom
    // sink and emit to stderr. But the reorder buffer assigns sequence
    // numbers we don't see here. Path of least resistance: write
    // payloads via stderr directly inside a wrapper Writer.
    let stderr_writer = StderrLineCounter::new();
    let report = run_ndjson_pipeline(input, stderr_writer, closure, opts)
        .context("reading input")?;

    for (seq, le) in &report.errors {
        let col = le.column.map(|c| format!("col {c} ")).unwrap_or_default();
        eprintln!(
            "line {seq}: {}syntax error at byte {}: {}",
            col, le.offset, le.message
        );
    }
    let any_syntax_bad = !report.errors.is_empty();
    let any_schema_bad = report
        .stats
        .as_ref()
        .map(|s| s.schema_fail > 0)
        .unwrap_or(false);

    finalise(report.stats, &args, any_syntax_bad, any_schema_bad)
}

/// A `Write` impl that re-prefixes each line with `line N: ` based on
/// a counter incremented per `\n`. Used by NDJSON validate to emit
/// schema violations to stderr in input order. The reorder buffer
/// already serialises payloads + `\n` in input order, so this just
/// needs to be a regular Write.
struct StderrLineCounter {
    line: u64,
}
impl StderrLineCounter {
    fn new() -> Self {
        Self { line: 0 }
    }
}
impl std::io::Write for StderrLineCounter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        // The reorder buffer writes each per-seq payload followed by
        // `\n`. We'll see each payload as its own write call OR as a
        // chunk. Walk the buffer and for each chunk delimited by `\n`
        // emit a prefixed line.
        let mut start = 0;
        for (i, b) in buf.iter().enumerate() {
            if *b == b'\n' {
                let chunk = &buf[start..i];
                if !chunk.is_empty() {
                    self.line += 1;
                    eprintln!("line {}: {}", self.line, String::from_utf8_lossy(chunk));
                }
                start = i + 1;
            }
        }
        // Trailing partial chunk: stash it. For the simple validate
        // case where the closure always emits whole lines, this never
        // fires; but to be safe we just emit it without prefix on the
        // next call. KISS: ignore the partial-line edge case (it
        // would only happen on torn buffer writes, which the
        // pipeline doesn't produce).
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn run_streaming<R: std::io::Read>(
    input: R,
    schema: Option<Arc<SchemaValidator>>,
    args: ValidateArgs,
    collect_stats: bool,
) -> Result<()> {
    if collect_stats || schema.is_some() {
        // Use a stats collector + ShardAccumulator if schema is on.
        let mut c = StatsCollector::default();
        c.begin_record();
        let mut r = EventReader::new(input);
        let mut acc = ShardAccumulator::new();
        let mut top: Option<TopLevel> = None;

        let parse_result: Result<(), jfmt_core::Error> = loop {
            match r.next_event() {
                Ok(None) => break Ok(()),
                Ok(Some(ev)) => {
                    if top.is_none() {
                        top = Some(match &ev {
                            jfmt_core::Event::StartArray => TopLevel::Array,
                            jfmt_core::Event::StartObject => TopLevel::Object,
                            _ => TopLevel::Scalar,
                        });
                        // Reject schema + non-array root in streaming mode.
                        if schema.is_some() && !matches!(top, Some(TopLevel::Array)) {
                            eprintln!(
                                "jfmt: schema validation of non-array root requires --materialize or --ndjson"
                            );
                            return Err(SilentExit(ExitCode::InputError).into());
                        }
                    }
                    c.observe(&ev);
                    if let Some(s) = schema.as_ref() {
                        // Per-element validation via ShardAccumulator.
                        match acc.push(ev.clone()) {
                            Ok(Some(shard)) => {
                                let violations = s.validate(&shard.value);
                                let where_ = match &shard.locator {
                                    ShardLocator::Index(i) => format!("element {i}"),
                                    _ => String::from("?"),
                                };
                                let paths: Vec<&str> = violations
                                    .iter()
                                    .map(|v| v.instance_path.as_str())
                                    .collect();
                                c.record_schema_outcome(violations.is_empty(), &paths);
                                for v in &violations {
                                    eprintln!(
                                        "{where_}: {}: {}: {}",
                                        v.instance_path, v.keyword, v.message
                                    );
                                    if args.fail_fast {
                                        return finalise(
                                            Some(c.finish()),
                                            &args,
                                            false,
                                            true,
                                        );
                                    }
                                }
                            }
                            Ok(None) => {}
                            Err(e) => {
                                eprintln!("jfmt: shard accumulator: {e}");
                                return Err(SilentExit(ExitCode::InputError).into());
                            }
                        }
                    }
                }
                Err(e) => break Err(e),
            }
        };
        let finish_result = parse_result.and_then(|_| r.finish());
        match finish_result {
            Ok(()) => {
                c.end_record(true);
                let stats = Some(c.finish());
                let any_schema_bad = stats
                    .as_ref()
                    .map(|s| s.schema_fail > 0)
                    .unwrap_or(false);
                finalise(stats, &args, false, any_schema_bad)
            }
            Err(e) => {
                c.end_record(false);
                let _ = c.finish();
                Err(anyhow::Error::from(e).context("validation failed"))
            }
        }
    } else {
        validate_syntax(input).context("validation failed")?;
        finalise(None, &args, false, false)
    }
}

fn run_materialize<R: std::io::Read>(
    input: R,
    schema: Option<Arc<SchemaValidator>>,
    args: ValidateArgs,
    collect_stats: bool,
) -> Result<()> {
    let value: serde_json::Value =
        serde_json::from_reader(input).context("validation failed: parsing input")?;

    let mut stats = if collect_stats {
        // Materialize doesn't drive an event stream through StatsCollector;
        // we just produce a minimal Stats with the schema fields filled.
        let mut s = Stats::default();
        s.records = 1;
        s.valid = 1;
        Some(s)
    } else {
        None
    };

    let mut any_schema_bad = false;
    if let Some(s) = schema.as_ref() {
        let violations = s.validate(&value);
        any_schema_bad = !violations.is_empty();
        for v in &violations {
            eprintln!(
                "(root): {}: {}: {}",
                v.instance_path, v.keyword, v.message
            );
            if args.fail_fast {
                break;
            }
        }
        if let Some(st) = stats.as_mut() {
            if violations.is_empty() {
                st.schema_pass += 1;
            } else {
                st.schema_fail += 1;
                for vio in &violations {
                    *st.top_violation_paths
                        .entry(vio.instance_path.clone())
                        .or_insert(0) += 1;
                }
            }
        }
    }

    finalise(stats, &args, false, any_schema_bad)
}

fn finalise(
    stats: Option<Stats>,
    args: &ValidateArgs,
    any_syntax_bad: bool,
    any_schema_bad: bool,
) -> Result<()> {
    if let Some(s) = stats.as_ref() {
        if args.stats {
            eprint!("{s}");
        }
        if let Some(path) = args.stats_json.as_ref() {
            write_stats_json(path, s).context("writing --stats-json")?;
        }
    }

    if any_syntax_bad {
        // M2 behaviour: any syntax failure -> exit 2 (or 1 under --strict
        // in NDJSON; M2 currently used 2 for syntax. Keep that.)
        return Err(anyhow::Error::from(SilentExit(ExitCode::SyntaxError)));
    }
    if any_schema_bad && args.strict {
        return Err(anyhow::Error::from(SilentExit(ExitCode::SchemaError)));
    }
    Ok(())
}

fn write_stats_json(path: &std::path::Path, stats: &Stats) -> std::io::Result<()> {
    let f = File::create(path)?;
    let mut w = BufWriter::new(f);
    serde_json::to_writer_pretty(&mut w, stats)?;
    w.write_all(b"\n")?;
    Ok(())
}
```

The `StderrLineCounter` is the path-of-least-resistance solution to the §4.1 stderr ordering caveat: M3's reorder buffer already emits payloads in input order followed by `\n`, so our Write impl just splits on `\n` and prefixes `line N: `. The `_` payload (empty) for clean lines becomes a counter increment with no print. **Verify in Step 5 that this approach produces correct `line N:` numbering** by running the parity test.

**Important behaviour note:** the `--strict` interaction with NDJSON syntax errors deviates slightly from spec §6: the spec table says `--strict` exit code 1 for syntax, but the existing M2 code path uses `SyntaxError = 2`. Keeping M2's behaviour (exit 2) is the conservative choice — `--strict` for syntax is effectively "make sure non-zero exit happens", which 2 already provides. Spec §6 will need a one-line clarification at Task 7 (README + spec update).

- [ ] **Step 4: Build**

Run: `cargo build --workspace 2>&1 | tail -3`
Expected: clean.

- [ ] **Step 5: Smoke**

```bash
# Schema happy path
cat > /tmp/schema.json <<'JSON'
{"type":"object","required":["name","age"],"properties":{"name":{"type":"string"},"age":{"type":"integer","minimum":0}}}
JSON

# Single-doc + array + schema (streaming, per-element)
echo '[{"name":"a","age":30},{"name":"b"}]' | \
  cargo run -q -p jfmt-cli -- validate --schema /tmp/schema.json
# Expected stderr: a 'required' violation for element 1; exit=0 (default non-strict)

# --strict: same, exit=3
echo '[{"name":"a","age":30},{"name":"b"}]' | \
  cargo run -q -p jfmt-cli -- validate --schema /tmp/schema.json --strict
echo "exit=$?"
# Expected: exit=3

# NDJSON + schema
printf '{"name":"a","age":30}\n{"name":"b"}\n' | \
  cargo run -q -p jfmt-cli -- validate --ndjson --schema /tmp/schema.json
# Expected stderr: line 2: schema: ...

# Materialize + schema (whole-doc, single object)
echo '{"name":"a"}' | \
  cargo run -q -p jfmt-cli -- validate -m --schema /tmp/schema.json
echo "exit=$?"
# Expected stderr: a 'required' violation for /age; exit=0

# Streaming + schema + non-array root rejected
echo '{"name":"a"}' | cargo run -q -p jfmt-cli -- validate --schema /tmp/schema.json
echo "exit=$?"
# Expected stderr: "non-array root requires --materialize or --ndjson"; exit=1

# --schema /nonexistent
cargo run -q -p jfmt-cli -- validate --schema /nonexistent.json < /dev/null; echo "exit=$?"
# Expected: exit=1, clear stderr
```

- [ ] **Step 6: Run all tests**

Run: `cargo test --workspace 2>&1 | grep -E "test result:" | head -25`
Expected: all green.

- [ ] **Step 7: Clippy**

Run: `cargo clippy --workspace --all-targets -- -D warnings 2>&1 | tail -3`
Expected: clean.

- [ ] **Step 8: Commit**

```bash
git add crates/jfmt-cli/src/cli.rs crates/jfmt-cli/src/commands/validate.rs crates/jfmt-cli/src/exit.rs
git commit -m "$(cat <<'EOF'
feat(cli): add 'validate --schema' (NDJSON / streaming / materialize)

Adds --schema FILE to validate, plus --strict, -m/--materialize, and
--force flags. Three branches:
- --ndjson: per-line via M3 pipeline; violations to stderr through a
  Write adapter that prefixes "line N:".
- default streaming + top-level array: per-element via M4a's
  ShardAccumulator. Non-array root + --schema errors at startup.
- --materialize: whole-doc validate after RAM pre-flight (M4b's
  shared helpers).
--strict surfaces any schema failure as exit 3 (SchemaError, now
in use). --fail-fast aborts at first violation.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 6: Integration tests (jfmt-core)

Three integration tests exercising each path through the validation engine, independent of the CLI.

**Files:**
- Create: `crates/jfmt-core/tests/validate_schema_streaming.rs`
- Create: `crates/jfmt-core/tests/validate_schema_ndjson.rs`
- Create: `crates/jfmt-core/tests/validate_schema_materialize.rs`

These tests don't use the CLI; they call `SchemaValidator` + `ShardAccumulator` / `run_ndjson_pipeline` directly to verify the core engine. CLI tests come in Task 7.

- [ ] **Step 1: streaming integration test**

Create `crates/jfmt-core/tests/validate_schema_streaming.rs`:

```rust
//! Per-element schema validation via ShardAccumulator.

use jfmt_core::filter::shard::{ShardAccumulator, TopLevel};
use jfmt_core::validate::SchemaValidator;
use jfmt_core::EventReader;
use serde_json::json;

fn schema() -> serde_json::Value {
    json!({
        "type": "object",
        "required": ["x"],
        "properties": {"x": {"type": "integer", "minimum": 0}}
    })
}

#[test]
fn array_with_mixed_validity() {
    let v = SchemaValidator::compile(&schema()).unwrap();
    let input = br#"[{"x":1},{"x":-1},{"y":2},{"x":3}]"#;
    let mut r = EventReader::new(&input[..]);
    let mut acc = ShardAccumulator::new();

    let mut pass = 0;
    let mut fail = 0;
    let mut top_set = false;

    while let Some(ev) = r.next_event().unwrap() {
        if !top_set {
            assert!(matches!(ev, jfmt_core::Event::StartArray));
            top_set = true;
        }
        if let Some(shard) = acc.push(ev).unwrap() {
            let violations = v.validate(&shard.value);
            if violations.is_empty() {
                pass += 1;
            } else {
                fail += 1;
            }
        }
    }
    assert_eq!(acc.top_level(), Some(TopLevel::Array));
    assert_eq!(pass, 2); // {x:1}, {x:3}
    assert_eq!(fail, 2); // {x:-1} (minimum), {y:2} (required)
}
```

- [ ] **Step 2: ndjson integration test**

Create `crates/jfmt-core/tests/validate_schema_ndjson.rs`:

```rust
//! NDJSON pipeline + schema; verify --threads parity.

use jfmt_core::validate::SchemaValidator;
use jfmt_core::{run_ndjson_pipeline, LineError, NdjsonPipelineOptions, StatsCollector};
use serde_json::json;
use std::io::Cursor;
use std::sync::Arc;

fn schema() -> serde_json::Value {
    json!({
        "type": "object",
        "required": ["x"],
        "properties": {"x": {"type": "integer"}}
    })
}

fn run_with_threads(threads: usize, input: &[u8]) -> (u64, u64) {
    let s = Arc::new(SchemaValidator::compile(&schema()).unwrap());
    let s_clone = Arc::clone(&s);
    let closure = move |line: &[u8], c: &mut StatsCollector| -> Result<Vec<Vec<u8>>, LineError> {
        c.begin_record();
        let value: serde_json::Value = match serde_json::from_slice(line) {
            Ok(v) => v,
            Err(e) => {
                c.end_record(false);
                return Err(LineError {
                    line: 0,
                    offset: 0,
                    column: None,
                    message: format!("{e}"),
                });
            }
        };
        c.end_record(true);
        let violations = s_clone.validate(&value);
        let paths: Vec<&str> = violations.iter().map(|v| v.instance_path.as_str()).collect();
        c.record_schema_outcome(violations.is_empty(), &paths);
        Ok(vec![Vec::new()])
    };
    let opts = NdjsonPipelineOptions {
        threads,
        collect_stats: true,
        ..Default::default()
    };
    let report = run_ndjson_pipeline(Cursor::new(input.to_vec()), std::io::sink(), closure, opts)
        .unwrap();
    let stats = report.stats.unwrap();
    (stats.schema_pass, stats.schema_fail)
}

#[test]
fn ndjson_counts_pass_fail() {
    let input = b"{\"x\":1}\n{\"y\":2}\n{\"x\":\"a\"}\n{\"x\":3}\n";
    let (pass, fail) = run_with_threads(1, input);
    assert_eq!(pass, 2);
    assert_eq!(fail, 2);
}

#[test]
fn ndjson_threads_parity_in_counts() {
    let mut input = Vec::new();
    for i in 0..200 {
        if i % 5 == 0 {
            input.extend_from_slice(format!("{{\"y\":{i}}}\n").as_bytes());
        } else {
            input.extend_from_slice(format!("{{\"x\":{i}}}\n").as_bytes());
        }
    }
    let (pass1, fail1) = run_with_threads(1, &input);
    let (pass4, fail4) = run_with_threads(4, &input);
    assert_eq!(pass1, pass4);
    assert_eq!(fail1, fail4);
}
```

- [ ] **Step 3: materialize integration test**

Create `crates/jfmt-core/tests/validate_schema_materialize.rs`:

```rust
//! Whole-document validation including aggregate keywords.

use jfmt_core::validate::SchemaValidator;
use serde_json::json;

#[test]
fn array_min_items_passes() {
    let schema = json!({"type": "array", "minItems": 3});
    let v = SchemaValidator::compile(&schema).unwrap();
    let value = json!([1, 2, 3]);
    assert!(v.validate(&value).is_empty());
}

#[test]
fn array_min_items_fails() {
    let schema = json!({"type": "array", "minItems": 3});
    let v = SchemaValidator::compile(&schema).unwrap();
    let value = json!([1, 2]);
    let violations = v.validate(&value);
    assert!(!violations.is_empty());
    assert!(violations.iter().any(|x| x.keyword == "minItems"));
}

#[test]
fn nested_required_violation_path_contains_field() {
    let schema = json!({
        "type": "object",
        "properties": {
            "user": {
                "type": "object",
                "required": ["email"]
            }
        }
    });
    let v = SchemaValidator::compile(&schema).unwrap();
    let value = json!({"user": {"name": "alice"}});
    let violations = v.validate(&value);
    assert!(!violations.is_empty());
    assert!(violations.iter().any(|x| x.instance_path.contains("user")));
}
```

- [ ] **Step 4: Run**

Run: `cargo test -p jfmt-core --test validate_schema_streaming --test validate_schema_ndjson --test validate_schema_materialize 2>&1 | tail -15`
Expected: 6 tests pass.

- [ ] **Step 5: Run all workspace**

Run: `cargo test --workspace 2>&1 | grep -E "test result:" | head -25`
Expected: all pass.

- [ ] **Step 6: Clippy**

Run: `cargo clippy --workspace --all-targets -- -D warnings 2>&1 | tail -3`
Expected: clean.

- [ ] **Step 7: Commit**

```bash
git add crates/jfmt-core/tests/validate_schema_streaming.rs crates/jfmt-core/tests/validate_schema_ndjson.rs crates/jfmt-core/tests/validate_schema_materialize.rs
git commit -m "$(cat <<'EOF'
test(core): add integration tests for schema validation engine

Three tests cover the three modes: streaming (per-element via
ShardAccumulator), NDJSON (run_ndjson_pipeline + Arc<SchemaValidator>
+ --threads parity), materialize (whole-doc with aggregate keywords).

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 7: CLI e2e tests + fixtures

**Files:**
- Create: `crates/jfmt-cli/tests/fixtures/schema_user.json`
- Create: `crates/jfmt-cli/tests/fixtures/schema_user_ndjson.ndjson`
- Create: `crates/jfmt-cli/tests/fixtures/schema_user_array.json`
- Modify: `crates/jfmt-cli/tests/cli_validate.rs`

- [ ] **Step 1: Fixtures**

Create `crates/jfmt-cli/tests/fixtures/schema_user.json`:

```json
{"type":"object","required":["name","age"],"properties":{"name":{"type":"string"},"age":{"type":"integer","minimum":0}}}
```

Create `crates/jfmt-cli/tests/fixtures/schema_user_ndjson.ndjson`:

```
{"name":"alice","age":30}
{"name":"bob"}
{"name":"carol","age":-5}
{"name":"dave","age":40}
```

(Three valid + one missing-age + one negative-age = 2 pass, 2 fail. Newline-terminated.)

Create `crates/jfmt-cli/tests/fixtures/schema_user_array.json`:

```json
[{"name":"alice","age":30},{"name":"bob"},{"name":"carol","age":-5},{"name":"dave","age":40}]
```

(Same data, top-level array form.)

- [ ] **Step 2: Append e2e tests to `cli_validate.rs`**

Append at end:

```rust
// ===== M5 — JSON Schema =====

#[test]
fn schema_ndjson_default_continues_with_violations() {
    jfmt()
        .args([
            "validate",
            "--ndjson",
            "--schema",
            "tests/fixtures/schema_user.json",
            "tests/fixtures/schema_user_ndjson.ndjson",
        ])
        .assert()
        .success() // exit 0 by default
        .stderr(predicate::str::contains("schema:"));
}

#[test]
fn schema_ndjson_strict_exits_3() {
    jfmt()
        .args([
            "validate",
            "--ndjson",
            "--strict",
            "--schema",
            "tests/fixtures/schema_user.json",
            "tests/fixtures/schema_user_ndjson.ndjson",
        ])
        .assert()
        .code(3);
}

#[test]
fn schema_streaming_array_validates_each_element() {
    jfmt()
        .args([
            "validate",
            "--strict",
            "--schema",
            "tests/fixtures/schema_user.json",
            "tests/fixtures/schema_user_array.json",
        ])
        .assert()
        .code(3) // 2 violations
        .stderr(predicate::str::contains("element"));
}

#[test]
fn schema_streaming_non_array_root_requires_materialize() {
    jfmt()
        .args([
            "validate",
            "--schema",
            "tests/fixtures/schema_user.json",
        ])
        .write_stdin(r#"{"name":"a"}"#)
        .assert()
        .code(1)
        .stderr(predicate::str::contains("--materialize"));
}

#[test]
fn schema_materialize_whole_doc() {
    jfmt()
        .args([
            "validate",
            "-m",
            "--strict",
            "--schema",
            "tests/fixtures/schema_user.json",
        ])
        .write_stdin(r#"{"name":"a"}"#) // missing age
        .assert()
        .code(3)
        .stderr(predicate::str::contains("required"));
}

#[test]
fn schema_materialize_passes_on_good_input() {
    jfmt()
        .args([
            "validate",
            "-m",
            "--schema",
            "tests/fixtures/schema_user.json",
        ])
        .write_stdin(r#"{"name":"a","age":1}"#)
        .assert()
        .success();
}

#[test]
fn schema_fail_fast_aborts_at_first() {
    jfmt()
        .args([
            "validate",
            "--ndjson",
            "--strict",
            "--fail-fast",
            "--schema",
            "tests/fixtures/schema_user.json",
            "tests/fixtures/schema_user_ndjson.ndjson",
        ])
        .assert()
        .code(3);
}

#[test]
fn schema_file_missing_exits_1() {
    jfmt()
        .args(["validate", "--schema", "tests/fixtures/nonexistent.json"])
        .write_stdin("[]")
        .assert()
        .code(1)
        .stderr(predicate::str::contains("schema"));
}

#[test]
fn schema_file_invalid_json_exits_1() {
    let dir = tempfile::tempdir().unwrap();
    let bad = dir.path().join("bad.json");
    std::fs::write(&bad, "not valid json").unwrap();
    jfmt()
        .args(["validate", "--schema"])
        .arg(&bad)
        .write_stdin("[]")
        .assert()
        .code(1);
}

#[test]
fn schema_file_invalid_schema_exits_1() {
    let dir = tempfile::tempdir().unwrap();
    let bad = dir.path().join("bad-schema.json");
    std::fs::write(&bad, r#"{"type":42}"#).unwrap();
    jfmt()
        .args(["validate", "--schema"])
        .arg(&bad)
        .write_stdin("[]")
        .assert()
        .code(1);
}

#[test]
fn schema_stats_json_includes_schema_fields() {
    let dir = tempfile::tempdir().unwrap();
    let stats_path = dir.path().join("stats.json");
    jfmt()
        .args([
            "validate",
            "--ndjson",
            "--schema",
            "tests/fixtures/schema_user.json",
            "--stats-json",
        ])
        .arg(&stats_path)
        .arg("tests/fixtures/schema_user_ndjson.ndjson")
        .assert()
        .success();
    let body = std::fs::read_to_string(&stats_path).unwrap();
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(v["schema_pass"].as_u64().unwrap() >= 1);
    assert!(v["schema_fail"].as_u64().unwrap() >= 1);
    assert!(v["top_violation_paths"].is_object());
}

#[test]
fn validate_materialize_conflicts_with_ndjson() {
    jfmt()
        .args(["validate", "-m", "--ndjson"])
        .write_stdin("[]")
        .assert()
        .code(2);
}

#[test]
fn validate_force_requires_materialize() {
    jfmt()
        .args(["validate", "--force"])
        .write_stdin("[]")
        .assert()
        .code(2);
}
```

If `tempfile` isn't already imported in `cli_validate.rs`, add `use tempfile;` (it's a workspace dev-dep already from M2).

- [ ] **Step 3: Run**

Run: `cargo test -p jfmt-cli --test cli_validate 2>&1 | tail -25`
Expected: all M2 tests + 13 new M5 tests pass.

- [ ] **Step 4: Run all workspace**

Run: `cargo test --workspace 2>&1 | grep -E "test result:" | head -25`
Expected: all green.

- [ ] **Step 5: Clippy**

Run: `cargo clippy --workspace --all-targets -- -D warnings 2>&1 | tail -3`
Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add crates/jfmt-cli/tests/cli_validate.rs crates/jfmt-cli/tests/fixtures/
git commit -m "$(cat <<'EOF'
test(cli): cover validate --schema across modes and error paths

13 e2e tests: NDJSON default vs --strict (exit 0 vs 3); streaming
top-level array per-element; non-array root + schema -> exit 1 with
--materialize hint; -m happy/bad path; --fail-fast; bad schema file
(missing/invalid JSON/invalid Schema) -> exit 1; --stats-json
contains schema fields; -m/--ndjson/--force clap conflicts.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 8: README + spec milestone update

**Files:**
- Modify: `README.md`
- Modify: `docs/superpowers/specs/2026-04-23-jfmt-phase1-design.md`

- [ ] **Step 1: Update README `## Status`**

Replace the existing Status block with:

```markdown
## Status

**M5 preview (v0.0.6)** — `pretty`, `minify`, `validate` (with
JSON Schema, `--strict`, `--materialize`, NDJSON parallel + per-element
streaming), and `filter` (streaming + NDJSON parallel + `--materialize`).
See [`docs/superpowers/specs/2026-04-23-jfmt-phase1-design.md`](docs/superpowers/specs/2026-04-23-jfmt-phase1-design.md)
for the Phase 1 roadmap.
```

- [ ] **Step 2: Append a `### Validate — JSON Schema` block to README**

Find the existing `### Validate` section. Append after it (before `### Filter`):

```markdown
### Validate — JSON Schema

```bash
# NDJSON: validate each line against the schema, report violations
jfmt validate --ndjson --schema schema.json data.ndjson

# Streaming top-level array: validate each element
jfmt validate --schema schema.json users.json

# Whole-document validation (e.g., to use `minItems` / `required` at root)
jfmt validate -m --schema schema.json config.json

# Strict CI mode: any failure exits non-zero
jfmt validate --ndjson --strict --schema schema.json events.ndjson
echo $?  # 3 if any record violated the schema
```

`--schema FILE` runs alongside the existing syntax validation. Mode +
top-level form decides what gets validated:

- `--ndjson` → each line is one record, validated independently.
- Default streaming + top-level array → each element validated
  (constant memory).
- Default streaming + top-level object/scalar → error; pass
  `--materialize` or `--ndjson`.
- `--materialize` (`-m`) → whole document validated as one value.
  Triggers a RAM budget pre-flight (file_size × 6, × 30 if compressed,
  abort unless `--force` when over 80 % RAM).

Violations stream to stderr immediately as they occur (TB-safe; no
unbounded accumulation). The schema's draft is auto-detected from
its `$schema` keyword (Draft 4 / 6 / 7 / 2019-09 / 2020-12).

**Exit codes:**

| Code | Meaning |
|---|---|
| 0 | success (or non-strict run with reported violations) |
| 1 | I/O failure, bad schema file, or non-array-root + `--schema` without `-m` |
| 2 | syntax error or clap usage error |
| 3 | schema violation under `--strict` |

Stats output (`--stats` / `--stats-json`) gains `schema_pass`,
`schema_fail`, and `top_violation_paths` (top 10 most-frequent
violated JSON Pointer paths).
```

- [ ] **Step 3: Update Phase 1 spec milestone**

In `docs/superpowers/specs/2026-04-23-jfmt-phase1-design.md`:

(a) Find the milestone table. Update the M5 row from "not started" to:

```
shipped (v0.0.6, 2026-04-25)
```

(b) Append a "M5 ✓ Shipped v0.0.6 on 2026-04-25" line to the shipped-status section, mirroring M4a/M4b.

Don't change M6 row (still pending).

- [ ] **Step 4: Final test sweep**

Run: `cargo test --workspace 2>&1 | grep -E "test result:" | head -25`
Expected: all green.

Run: `cargo clippy --workspace --all-targets -- -D warnings 2>&1 | tail -3`
Expected: clean.

Run: `cargo fmt --all -- --check 2>&1 | tail -10`
Expected: clean. If drift, mention in report; fold into Task 9.

- [ ] **Step 5: Commit**

```bash
git add README.md docs/superpowers/specs/2026-04-23-jfmt-phase1-design.md
git commit -m "$(cat <<'EOF'
docs: document validate --schema and mark M5 shipped

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 9: Ship `v0.0.6`

**Files:**
- Modify: `Cargo.toml` (workspace)

- [ ] **Step 1: Bump version**

In `Cargo.toml` (workspace), change:

```toml
version = "0.0.5"
```

to:

```toml
version = "0.0.6"
```

- [ ] **Step 2: Verify**

Run: `cargo build --workspace 2>&1 | tail -3`
Expected: clean.

Run: `cargo test --workspace 2>&1 | grep -E "test result:" | head -25`
Expected: all green.

- [ ] **Step 3: Commit + tag**

```bash
git add Cargo.toml Cargo.lock
git commit -m "$(cat <<'EOF'
chore: bump version to 0.0.6 (M5 — JSON Schema validation)

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
git tag -a v0.0.6 -m "M5: JSON Schema validation"
```

---

## Self-Review Checklist (Performed)

**Spec coverage:**
- §1 Scope (`--schema FILE`, mode-driven apply, `-m`, `--force`, `--strict` extension) → Tasks 5, 7, 8.
- §2 D1 (no `--schema-applies-to`) → Task 5 (no flag added).
- §2 D2 (stream stderr; top-N cap) → Tasks 3 (cap) + 5 (stderr).
- §2 D3 (`--strict` is new, not "widened") → Task 5 (`strict: bool` field).
- §2 D4 (RAM helpers extracted) → Task 2.
- §3 Module layout → Tasks 4 (schema.rs), 2 (ram_budget.rs).
- §3.2 Stats fields → Task 3.
- §4.1 NDJSON pipeline + violations to stderr → Task 5 (StderrLineCounter).
- §4.2 Streaming + ShardAccumulator → Task 5 (run_streaming).
- §4.3 Materialize → Task 5 (run_materialize).
- §4.4 SchemaValidator interface → Task 4.
- §5 Errors / exit codes → Task 5 (finalise function maps to SilentExit).
- §6 CLI flags + interaction matrix → Tasks 5, 7.
- §7 Stats output → Tasks 3 (Display), 7 (e2e check on `--stats-json`).
- §8 Tests → Tasks 6 (core integration), 7 (CLI e2e).
- §9 Risks: jsonschema MSRV → Task 1; Send+Sync → Task 1 + Task 4 thread-smoke.

**Placeholder scan:** Task 1 leaves `<X.Y.Z>` for jsonschema deliberately, with Annex C as load-bearing. No "TBD" / "TODO" elsewhere; every code block is complete.

**Type consistency:** `SchemaValidator` is `Clone + Send + Sync` (Arc inside) defined Task 4, used Tasks 5/6. `Violation { instance_path, keyword, message }` consistent across Tasks 4/5/6. `SchemaError` variants (BadSchemaFile, BadSchemaJson, BadSchema) consistent. `Stats.schema_pass / schema_fail / top_violation_paths` consistent across Tasks 3/5/7. `record_schema_outcome(passed, paths)` signature consistent. RAM helpers `pub(super)` from Task 2, consumed by Task 5.
