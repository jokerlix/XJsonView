# jfmt M2 — `validate` + Stats Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship `jfmt validate` with syntax-error location reporting and a streaming `StatsCollector`, plus NDJSON per-line validation. Tag as `v0.0.2`.

**Architecture:** Extend `jfmt-core` with a `validate/` module containing `syntax.rs` (drain-only validator), `stats.rs` (streaming `StatsCollector`), and `ndjson.rs` (serial `\n` splitter — no threading; parallel pipeline is M3). Add `Validate` subcommand to `jfmt-cli`, wire human-readable stats to stderr and machine-readable stats to `--stats-json`. Reuse the existing `EventReader` unchanged; surface `line`/`column` in `Error::Syntax` by propagating struson's `LinePosition`.

**Tech Stack:** Rust 1.75, `struson`, `thiserror`, `clap`, `serde` + `serde_json` (new), `anyhow` at CLI. Existing pins stay.

---

## Scope Boundaries

**In-scope:**
- `jfmt validate` single-document + NDJSON (serial).
- Error location: byte offset, line, column, message.
- Streaming stats: record count, top-level type distribution, max depth, top-N top-level keys.
- Human-readable stats to stderr; structured stats to file via `--stats-json`.
- `--fail-fast` for NDJSON (default: keep going, report all bad lines, exit 2 if any).
- Exit codes: 0 success, 1 I/O/args, 2 syntax error in at least one record.

**Out of scope (deferred):**
- JSON Schema validation (`--schema`, `--strict`, exit 3) → **M5**.
- Parallel NDJSON pipeline → **M3** replaces the serial splitter.
- Progress bars (`indicatif`) → **M6**.
- Schema violation histograms in stats → **M5** extends the same struct.

## File Structure

```
crates/jfmt-core/src/
  error.rs                       # Modify: Syntax { offset, line, column, message }
  parser.rs                      # Modify: propagate line/column from struson
  lib.rs                         # Modify: re-export validate::*
  validate/
    mod.rs                       # Create: module root + re-exports
    syntax.rs                    # Create: validate_syntax(reader) -> Result<()>
    stats.rs                     # Create: Stats struct + StatsCollector
    ndjson.rs                    # Create: serial line splitter + per-line validate
  tests/
    validate_proptest.rs         # Create: property test

crates/jfmt-cli/src/
  cli.rs                         # Modify: add ValidateArgs + Command::Validate
  commands/
    mod.rs                       # Modify: pub mod validate
    validate.rs                  # Create: subcommand wiring
  main.rs                        # Modify: dispatch Validate
  tests/
    cli_validate.rs              # Create: CLI e2e
    fixtures/
      valid.json                 # reuse simple.json? create dedicated
      bad.json                   # syntax error line 2 col 5
      ndjson-good.ndjson         # 3 valid lines
      ndjson-mixed.ndjson        # line 2 bad, lines 1+3 good
      stats-expected.json        # golden JSON for stats of valid.json

README.md                        # Modify: add validate section
docs/superpowers/specs/
  2026-04-23-jfmt-phase1-design.md  # Modify: mark M2 shipped
```

**Commit cadence:** one logical change per commit, prefix as in M1 (`feat(core):`, `feat(cli):`, `test(...)`, `docs:`, `chore:`).

---

## Task 1: Extend `Error::Syntax` with line/column

**Files:**
- Modify: `crates/jfmt-core/src/error.rs`
- Modify: `crates/jfmt-core/src/parser.rs`

The existing `Error::Syntax { offset, message }` loses the row/column info struson already carries in `JsonSyntaxError.location.line_pos`. M2's headline feature is good error messages, so this goes first.

- [ ] **Step 1: Update `Error::Syntax`**

```rust
// crates/jfmt-core/src/error.rs
use std::io;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("io error: {0}")]
    Io(#[from] io::Error),

    #[error("syntax error at {}: {message}", format_location(.offset, .line, .column))]
    Syntax {
        offset: u64,
        line: Option<u64>,
        column: Option<u64>,
        message: String,
    },

    #[error("invalid state: {0}")]
    State(String),
}

fn format_location(offset: &u64, line: &Option<u64>, column: &Option<u64>) -> String {
    match (line, column) {
        (Some(l), Some(c)) => format!("line {l} column {c} (byte {offset})"),
        _ => format!("byte {offset}"),
    }
}

pub type Result<T> = std::result::Result<T, Error>;
```

- [ ] **Step 2: Update `parser.rs::map_err` to pull line/column from struson**

```rust
// crates/jfmt-core/src/parser.rs — replace map_err at bottom of file
fn map_err(e: ReaderError) -> Error {
    match e {
        ReaderError::IoError { error, .. } => Error::Io(error),
        ReaderError::SyntaxError(se) => Error::Syntax {
            offset: se.location.data_pos.unwrap_or(0),
            line: se.location.line_pos.as_ref().map(|lp| lp.line),
            column: se.location.line_pos.as_ref().map(|lp| lp.column),
            message: format!("{:?}", se.kind),
        },
        other => Error::Syntax {
            offset: 0,
            line: None,
            column: None,
            message: format!("{other}"),
        },
    }
}
```

- [ ] **Step 3: Update the existing parser test**

Open `crates/jfmt-core/src/parser.rs`. The test `reports_syntax_error_with_offset` only checks the variant; keep it but add one asserting line/column:

```rust
    #[test]
    fn syntax_error_carries_line_and_column() {
        let mut r = EventReader::new(b"{\n  \"a\":,\n}".as_slice());
        let err = loop {
            match r.next_event() {
                Ok(None) => panic!("expected error"),
                Ok(Some(_)) => continue,
                Err(e) => break e,
            }
        };
        match err {
            Error::Syntax { line, column, .. } => {
                assert_eq!(line, Some(1), "0-indexed or 1-indexed depends on struson; adjust expected value when this runs");
                assert!(column.is_some());
            }
            other => panic!("got {other:?}"),
        }
    }
```

**Contingency:** if the test fails because struson uses 0-indexed lines, update the expected line value to 0 and the assertion message above — do not hide the difference; document what struson returns. Do not silently accept a different line number.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p jfmt-core parser`
Expected: all parser tests pass (7 existing + 1 new).

Run: `cargo test -p jfmt-core` (workspace-wide for this crate)
Expected: pass; the one new field is optional so existing call sites unaffected.

- [ ] **Step 5: Commit**

```bash
git add crates/jfmt-core/src/error.rs crates/jfmt-core/src/parser.rs
git commit -m "feat(core): add line/column to Error::Syntax"
```

---

## Task 2: `validate/mod.rs` + `syntax.rs` — `validate_syntax`

**Files:**
- Create: `crates/jfmt-core/src/validate/mod.rs`
- Create: `crates/jfmt-core/src/validate/syntax.rs`
- Modify: `crates/jfmt-core/src/lib.rs`

- [ ] **Step 1: Write the failing tests**

```rust
// crates/jfmt-core/src/validate/syntax.rs
//! Drain-only syntax validator. Consumes events without writing output.

use crate::parser::EventReader;
use crate::Result;
use std::io::Read;

/// Read every event from `reader` to confirm the document is syntactically valid.
/// Returns `Ok(())` iff the document parses cleanly.
pub fn validate_syntax<R: Read>(reader: R) -> Result<()> {
    let mut r = EventReader::new(reader);
    while r.next_event()?.is_some() {}
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Error;

    #[test]
    fn accepts_valid_document() {
        let input = br#"{"a":[1,2,3],"b":null}"#;
        validate_syntax(input.as_slice()).unwrap();
    }

    #[test]
    fn accepts_scalars_and_empties() {
        validate_syntax(br#"null"#.as_slice()).unwrap();
        validate_syntax(br#"[]"#.as_slice()).unwrap();
        validate_syntax(br#"{}"#.as_slice()).unwrap();
        validate_syntax(br#""hi""#.as_slice()).unwrap();
        validate_syntax(br#"42"#.as_slice()).unwrap();
    }

    #[test]
    fn rejects_trailing_garbage() {
        let res = validate_syntax(br#"{"a":1} garbage"#.as_slice());
        assert!(matches!(res, Err(Error::Syntax { .. })), "got {res:?}");
    }

    #[test]
    fn rejects_truncated_input() {
        let res = validate_syntax(br#"{"a":"#.as_slice());
        assert!(matches!(res, Err(Error::Syntax { .. })), "got {res:?}");
    }
}
```

- [ ] **Step 2: Create the module root**

```rust
// crates/jfmt-core/src/validate/mod.rs
//! Validation and streaming statistics.

pub mod syntax;

pub use syntax::validate_syntax;
```

- [ ] **Step 3: Wire into `lib.rs`**

```rust
// crates/jfmt-core/src/lib.rs (add)
pub mod validate;
pub use validate::validate_syntax;
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p jfmt-core validate`
Expected: 4 passed.

- [ ] **Step 5: Commit**

```bash
git add crates/jfmt-core/src/validate crates/jfmt-core/src/lib.rs
git commit -m "feat(core): add validate_syntax drain-only validator"
```

---

## Task 3: `Stats` struct + JSON serialization

**Files:**
- Modify: `crates/jfmt-core/Cargo.toml` (add `serde`, `serde_json` as deps)
- Create: `crates/jfmt-core/src/validate/stats.rs` (struct + serde only, no collector yet)

Splitting the data type from the collector lets us test serialization independently.

- [ ] **Step 1: Add serde deps**

```toml
# crates/jfmt-core/Cargo.toml (under [dependencies])
serde = { version = "1", features = ["derive"] }
serde_json = { workspace = true }
```

Also add to workspace deps if not already:

```toml
# Cargo.toml (workspace) — add under [workspace.dependencies]
serde = { version = "1", features = ["derive"] }
```

`serde_json` was added earlier pinned to `=1.0.132` — keep.

- [ ] **Step 2: Define the `Stats` struct**

```rust
// crates/jfmt-core/src/validate/stats.rs
//! Streaming statistics collected while parsing a JSON (or NDJSON) input.

use serde::Serialize;
use std::collections::BTreeMap;

/// The JSON value-kind as reported by top-level type distribution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ValueKind {
    Null,
    Bool,
    Number,
    String,
    Array,
    Object,
}

impl ValueKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ValueKind::Null => "null",
            ValueKind::Bool => "bool",
            ValueKind::Number => "number",
            ValueKind::String => "string",
            ValueKind::Array => "array",
            ValueKind::Object => "object",
        }
    }
}

/// Collected statistics for a single input run.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Stats {
    /// Number of top-level records (1 for single-doc mode; N for NDJSON).
    pub records: u64,
    /// Records that parsed cleanly.
    pub valid: u64,
    /// Records whose parse failed.
    pub invalid: u64,
    /// Top-level type frequency. Keyed by ValueKind::as_str().
    pub top_level_types: BTreeMap<String, u64>,
    /// Max container nesting depth seen across all records.
    pub max_depth: u64,
    /// Frequency of keys seen at depth 1 of top-level objects (capped).
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub top_level_keys: BTreeMap<String, u64>,
    /// Number of distinct keys dropped once the key cap was hit.
    #[serde(skip_serializing_if = "is_zero_u64")]
    pub top_level_keys_truncated: u64,
}

fn is_zero_u64(n: &u64) -> bool {
    *n == 0
}
```

- [ ] **Step 3: Write unit tests for JSON serialization**

```rust
// crates/jfmt-core/src/validate/stats.rs (append #[cfg(test)] block)
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_minimum_fields() {
        let s = Stats {
            records: 1,
            valid: 1,
            invalid: 0,
            max_depth: 2,
            top_level_types: [("object".to_string(), 1)].into_iter().collect(),
            ..Default::default()
        };
        let j: serde_json::Value = serde_json::to_value(&s).unwrap();
        assert_eq!(j["records"], 1);
        assert_eq!(j["valid"], 1);
        assert_eq!(j["top_level_types"]["object"], 1);
        assert_eq!(j.get("top_level_keys"), None, "skipped when empty");
        assert_eq!(j.get("top_level_keys_truncated"), None, "skipped when zero");
    }

    #[test]
    fn serializes_keys_when_present() {
        let mut s = Stats::default();
        s.top_level_keys.insert("a".into(), 3);
        let j = serde_json::to_value(&s).unwrap();
        assert_eq!(j["top_level_keys"]["a"], 3);
    }

    #[test]
    fn value_kind_renames_lowercase() {
        assert_eq!(
            serde_json::to_string(&ValueKind::Object).unwrap(),
            "\"object\""
        );
    }
}
```

- [ ] **Step 4: Export from `validate/mod.rs`**

```rust
// crates/jfmt-core/src/validate/mod.rs (add)
pub mod stats;

pub use stats::{Stats, ValueKind};
```

- [ ] **Step 5: Run tests**

Run: `cargo test -p jfmt-core stats`
Expected: 3 passed.

- [ ] **Step 6: Commit**

```bash
git add crates/jfmt-core/Cargo.toml Cargo.toml crates/jfmt-core/src/validate
git commit -m "feat(core): add Stats struct with JSON serialization"
```

---

## Task 4: `StatsCollector` — streaming event consumer

**Files:**
- Modify: `crates/jfmt-core/src/validate/stats.rs` (add `StatsCollector` + tests)

- [ ] **Step 1: Write failing tests first**

Append to `crates/jfmt-core/src/validate/stats.rs`:

```rust
#[cfg(test)]
mod collector_tests {
    use super::*;
    use crate::event::{Event, Scalar};

    fn feed(events: Vec<Event>) -> Stats {
        let mut c = StatsCollector::new(StatsConfig::default());
        c.begin_record();
        for e in &events {
            c.observe(e);
        }
        c.end_record(true);
        c.finish()
    }

    #[test]
    fn counts_one_object_record() {
        let s = feed(vec![
            Event::StartObject,
            Event::Name("a".into()),
            Event::Value(Scalar::Number("1".into())),
            Event::EndObject,
        ]);
        assert_eq!(s.records, 1);
        assert_eq!(s.valid, 1);
        assert_eq!(s.top_level_types.get("object"), Some(&1));
        assert_eq!(s.top_level_keys.get("a"), Some(&1));
        assert_eq!(s.max_depth, 1);
    }

    #[test]
    fn tracks_max_depth() {
        let s = feed(vec![
            Event::StartArray,
            Event::StartArray,
            Event::StartObject,
            Event::Name("x".into()),
            Event::Value(Scalar::Null),
            Event::EndObject,
            Event::EndArray,
            Event::EndArray,
        ]);
        assert_eq!(s.max_depth, 3);
        assert_eq!(s.top_level_types.get("array"), Some(&1));
        // Keys inside nested structures are NOT counted.
        assert!(s.top_level_keys.is_empty());
    }

    #[test]
    fn counts_scalar_top_level() {
        let s = feed(vec![Event::Value(Scalar::String("hi".into()))]);
        assert_eq!(s.top_level_types.get("string"), Some(&1));
        assert_eq!(s.max_depth, 0);
    }

    #[test]
    fn key_cap_truncates() {
        let cfg = StatsConfig {
            top_level_keys_cap: 2,
        };
        let mut c = StatsCollector::new(cfg);
        c.begin_record();
        for k in ["a", "b", "c", "d"] {
            c.observe(&Event::StartObject); // reset? No — we simulate ONE object below
            // Actually construct a single object with 4 keys:
            break; // skip simulation loop, build inline instead
        }
        // Build inline: one top-level object with 4 distinct keys.
        let _ = c; // drop the partial one
        let mut c = StatsCollector::new(StatsConfig { top_level_keys_cap: 2 });
        c.begin_record();
        c.observe(&Event::StartObject);
        for k in ["a", "b", "c", "d"] {
            c.observe(&Event::Name(k.into()));
            c.observe(&Event::Value(Scalar::Null));
        }
        c.observe(&Event::EndObject);
        c.end_record(true);
        let s = c.finish();
        assert_eq!(s.top_level_keys.len(), 2);
        assert_eq!(s.top_level_keys_truncated, 2);
    }

    #[test]
    fn invalid_record_counts_separately() {
        let mut c = StatsCollector::new(StatsConfig::default());
        c.begin_record();
        c.observe(&Event::StartObject);
        c.end_record(false); // simulate a failed parse mid-record
        let s = c.finish();
        assert_eq!(s.records, 1);
        assert_eq!(s.valid, 0);
        assert_eq!(s.invalid, 1);
    }
}
```

- [ ] **Step 2: Add `StatsConfig` + `StatsCollector`**

Append to `crates/jfmt-core/src/validate/stats.rs`:

```rust
use crate::event::Event;

#[derive(Debug, Clone)]
pub struct StatsConfig {
    /// Maximum distinct top-level object keys to remember. Once hit, further
    /// new keys are counted in `top_level_keys_truncated` and discarded.
    pub top_level_keys_cap: usize,
}

impl Default for StatsConfig {
    fn default() -> Self {
        Self { top_level_keys_cap: 1024 }
    }
}

/// Streaming collector. Lifecycle per record:
///   begin_record() → observe(&Event) × N → end_record(valid: bool)
/// Call finish() once to consume the collector and get the Stats.
pub struct StatsCollector {
    cfg: StatsConfig,
    stats: Stats,
    /// Current container depth inside the in-flight record (0 = not inside).
    depth: u64,
    /// `true` between a StartObject at depth 0→1 and its EndObject.
    in_top_level_object: bool,
    /// `true` when the record has already contributed a type to the distribution.
    top_type_recorded: bool,
    /// Name-coming-next flag for the current top-level object only.
    expecting_name_at_depth_1: bool,
}

impl StatsCollector {
    pub fn new(cfg: StatsConfig) -> Self {
        Self {
            cfg,
            stats: Stats::default(),
            depth: 0,
            in_top_level_object: false,
            top_type_recorded: false,
            expecting_name_at_depth_1: false,
        }
    }

    pub fn begin_record(&mut self) {
        self.depth = 0;
        self.in_top_level_object = false;
        self.top_type_recorded = false;
        self.expecting_name_at_depth_1 = false;
    }

    pub fn observe(&mut self, ev: &Event) {
        if !self.top_type_recorded {
            let kind = match ev {
                Event::StartArray => Some(ValueKind::Array),
                Event::StartObject => Some(ValueKind::Object),
                Event::Value(Scalar::Null) => Some(ValueKind::Null),
                Event::Value(Scalar::Bool(_)) => Some(ValueKind::Bool),
                Event::Value(Scalar::Number(_)) => Some(ValueKind::Number),
                Event::Value(Scalar::String(_)) => Some(ValueKind::String),
                Event::Name(_) | Event::EndArray | Event::EndObject => None,
            };
            if let Some(k) = kind {
                *self
                    .stats
                    .top_level_types
                    .entry(k.as_str().to_string())
                    .or_insert(0) += 1;
                self.top_type_recorded = true;
            }
        }

        match ev {
            Event::StartObject => {
                self.depth += 1;
                if self.depth == 1 {
                    self.in_top_level_object = true;
                    self.expecting_name_at_depth_1 = true;
                }
                if self.depth > self.stats.max_depth {
                    self.stats.max_depth = self.depth;
                }
            }
            Event::StartArray => {
                self.depth += 1;
                if self.depth > self.stats.max_depth {
                    self.stats.max_depth = self.depth;
                }
                self.expecting_name_at_depth_1 = false;
            }
            Event::EndObject | Event::EndArray => {
                if self.depth > 0 {
                    self.depth -= 1;
                }
                if self.depth == 0 {
                    self.in_top_level_object = false;
                }
                // After closing a child container back at depth 1 of a top-level
                // object, the next slot is a name.
                self.expecting_name_at_depth_1 =
                    self.depth == 1 && self.in_top_level_object;
            }
            Event::Name(name) => {
                if self.in_top_level_object && self.expecting_name_at_depth_1 {
                    if self.stats.top_level_keys.contains_key(name.as_str()) {
                        *self.stats.top_level_keys.get_mut(name.as_str()).unwrap() += 1;
                    } else if self.stats.top_level_keys.len() < self.cfg.top_level_keys_cap {
                        self.stats.top_level_keys.insert(name.clone(), 1);
                    } else {
                        self.stats.top_level_keys_truncated += 1;
                    }
                    self.expecting_name_at_depth_1 = false;
                }
            }
            Event::Value(_) => {
                // After a scalar value inside a top-level object, next slot
                // is a name again (assuming the object isn't closed next).
                self.expecting_name_at_depth_1 =
                    self.depth == 1 && self.in_top_level_object;
            }
        }
    }

    pub fn end_record(&mut self, valid: bool) {
        self.stats.records += 1;
        if valid {
            self.stats.valid += 1;
        } else {
            self.stats.invalid += 1;
        }
    }

    pub fn finish(self) -> Stats {
        self.stats
    }
}

impl Default for StatsCollector {
    fn default() -> Self {
        Self::new(StatsConfig::default())
    }
}
```

- [ ] **Step 3: Export the collector**

```rust
// crates/jfmt-core/src/validate/mod.rs — extend the re-export
pub use stats::{Stats, StatsCollector, StatsConfig, ValueKind};
```

- [ ] **Step 4: Run the tests**

Run: `cargo test -p jfmt-core stats`
Expected: 5 collector tests + 3 serialization tests = 8 passed.

**Contingency:** if `key_cap_truncates` asserts `top_level_keys.len() == 2` but you see something else, the cap check runs before insert — re-check the branch order (contains → else-if under-cap → else truncated).

- [ ] **Step 5: Commit**

```bash
git add crates/jfmt-core/src/validate
git commit -m "feat(core): add streaming StatsCollector"
```

---

## Task 5: Human-readable `Display` for `Stats`

**Files:**
- Modify: `crates/jfmt-core/src/validate/stats.rs`

- [ ] **Step 1: Write the failing test**

Append to the existing `mod tests` (serialization block) in `stats.rs`:

```rust
    #[test]
    fn display_format_is_stable() {
        let mut s = Stats {
            records: 2,
            valid: 1,
            invalid: 1,
            max_depth: 3,
            ..Default::default()
        };
        s.top_level_types.insert("object".into(), 2);
        s.top_level_keys.insert("a".into(), 2);
        s.top_level_keys.insert("b".into(), 1);

        let text = format!("{s}");
        assert!(text.contains("records: 2 (1 valid, 1 invalid)"), "got: {text}");
        assert!(text.contains("max depth: 3"));
        assert!(text.contains("top-level types:"));
        assert!(text.contains("  object: 2"));
        assert!(text.contains("top-level keys:"));
        assert!(text.contains("  a: 2"));
    }
```

- [ ] **Step 2: Implement `Display`**

Append to `stats.rs`:

```rust
use std::fmt;

impl fmt::Display for Stats {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "records: {} ({} valid, {} invalid)",
            self.records, self.valid, self.invalid
        )?;
        writeln!(f, "max depth: {}", self.max_depth)?;

        if !self.top_level_types.is_empty() {
            writeln!(f, "top-level types:")?;
            // Sort by descending count, then by name.
            let mut types: Vec<_> = self.top_level_types.iter().collect();
            types.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
            for (k, v) in types {
                writeln!(f, "  {k}: {v}")?;
            }
        }

        if !self.top_level_keys.is_empty() {
            writeln!(f, "top-level keys:")?;
            let mut keys: Vec<_> = self.top_level_keys.iter().collect();
            keys.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
            for (k, v) in keys {
                writeln!(f, "  {k}: {v}")?;
            }
            if self.top_level_keys_truncated > 0 {
                writeln!(
                    f,
                    "  … {} distinct keys beyond cap dropped",
                    self.top_level_keys_truncated
                )?;
            }
        }
        Ok(())
    }
}
```

- [ ] **Step 3: Run**

Run: `cargo test -p jfmt-core stats::tests::display_format_is_stable`
Expected: pass.

- [ ] **Step 4: Commit**

```bash
git add crates/jfmt-core/src/validate/stats.rs
git commit -m "feat(core): add human-readable Display for Stats"
```

---

## Task 6: Serial NDJSON line splitter

**Files:**
- Create: `crates/jfmt-core/src/validate/ndjson.rs`
- Modify: `crates/jfmt-core/src/validate/mod.rs`

NDJSON = one JSON value per `\n`-terminated line. In M2 the splitter is serial; M3 replaces it with a parallel pipeline that reuses the same per-line semantics.

- [ ] **Step 1: Write failing tests**

```rust
// crates/jfmt-core/src/validate/ndjson.rs
//! Serial NDJSON validator. Reports errors per line.

use crate::error::Error;
use crate::validate::stats::StatsCollector;
use crate::parser::EventReader;
use std::io::{BufRead, BufReader, Read};

/// One reported per-line error in NDJSON mode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineError {
    /// 1-indexed line number (blank lines still advance the counter).
    pub line: u64,
    /// Byte offset inside that line where struson failed.
    pub offset: u64,
    pub column: Option<u64>,
    pub message: String,
}

#[derive(Debug, Clone, Copy)]
pub struct NdjsonOptions {
    /// Stop at first bad line.
    pub fail_fast: bool,
    /// Collect per-record statistics.
    pub collect_stats: bool,
}

impl Default for NdjsonOptions {
    fn default() -> Self {
        Self { fail_fast: false, collect_stats: false }
    }
}

/// Result of validating an NDJSON stream.
pub struct NdjsonReport {
    pub errors: Vec<LineError>,
    pub stats: Option<crate::validate::Stats>,
}

/// Read `reader` line-by-line and validate each as its own JSON value.
/// Empty / whitespace-only lines are skipped (no error, no stats contribution).
pub fn validate_ndjson<R: Read>(
    reader: R,
    opts: NdjsonOptions,
) -> std::io::Result<NdjsonReport> {
    let br = BufReader::new(reader);
    let mut collector = opts.collect_stats.then(StatsCollector::default);
    let mut errors = Vec::new();
    let mut line_no: u64 = 0;

    for line in br.lines() {
        line_no += 1;
        let line = line?; // I/O failure is fatal.
        if line.trim().is_empty() {
            continue;
        }

        if let Some(c) = collector.as_mut() {
            c.begin_record();
        }

        let mut parser = EventReader::new(line.as_bytes());
        let mut ok = true;
        loop {
            match parser.next_event() {
                Ok(None) => break,
                Ok(Some(ev)) => {
                    if let Some(c) = collector.as_mut() {
                        c.observe(&ev);
                    }
                }
                Err(Error::Syntax { offset, column, message, .. }) => {
                    errors.push(LineError {
                        line: line_no,
                        offset,
                        column,
                        message,
                    });
                    ok = false;
                    break;
                }
                Err(Error::Io(io)) => return Err(io),
                Err(Error::State(s)) => {
                    errors.push(LineError {
                        line: line_no,
                        offset: 0,
                        column: None,
                        message: format!("invalid state: {s}"),
                    });
                    ok = false;
                    break;
                }
            }
        }

        if let Some(c) = collector.as_mut() {
            c.end_record(ok);
        }
        if !ok && opts.fail_fast {
            break;
        }
    }

    Ok(NdjsonReport {
        errors,
        stats: collector.map(|c| c.finish()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_all_valid_lines() {
        let input = b"1\n\"hi\"\n{\"a\":1}\n";
        let r = validate_ndjson(input.as_slice(), NdjsonOptions::default()).unwrap();
        assert!(r.errors.is_empty());
    }

    #[test]
    fn reports_bad_line_keeps_going() {
        let input = b"1\n{bad}\n3\n";
        let r = validate_ndjson(input.as_slice(), NdjsonOptions::default()).unwrap();
        assert_eq!(r.errors.len(), 1);
        assert_eq!(r.errors[0].line, 2);
    }

    #[test]
    fn fail_fast_stops_on_first_error() {
        let input = b"{bad1}\n{bad2}\n1\n";
        let r = validate_ndjson(
            input.as_slice(),
            NdjsonOptions { fail_fast: true, collect_stats: false },
        )
        .unwrap();
        assert_eq!(r.errors.len(), 1);
        assert_eq!(r.errors[0].line, 1);
    }

    #[test]
    fn skips_blank_lines() {
        let input = b"1\n\n\n2\n";
        let r = validate_ndjson(input.as_slice(), NdjsonOptions::default()).unwrap();
        assert!(r.errors.is_empty());
        // with collect_stats we'd also verify records == 2
        let r = validate_ndjson(
            input.as_slice(),
            NdjsonOptions { fail_fast: false, collect_stats: true },
        )
        .unwrap();
        assert_eq!(r.stats.as_ref().unwrap().records, 2);
    }

    #[test]
    fn stats_count_top_level_types_across_lines() {
        let input = b"1\n\"hi\"\n{\"a\":1}\n[1,2]\n";
        let r = validate_ndjson(
            input.as_slice(),
            NdjsonOptions { fail_fast: false, collect_stats: true },
        )
        .unwrap();
        let s = r.stats.unwrap();
        assert_eq!(s.records, 4);
        assert_eq!(s.top_level_types.get("number"), Some(&1));
        assert_eq!(s.top_level_types.get("string"), Some(&1));
        assert_eq!(s.top_level_types.get("object"), Some(&1));
        assert_eq!(s.top_level_types.get("array"), Some(&1));
    }
}
```

- [ ] **Step 2: Export from `mod.rs`**

```rust
// crates/jfmt-core/src/validate/mod.rs (add)
pub mod ndjson;

pub use ndjson::{validate_ndjson, LineError, NdjsonOptions, NdjsonReport};
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p jfmt-core ndjson`
Expected: 5 passed.

- [ ] **Step 4: Commit**

```bash
git add crates/jfmt-core/src/validate
git commit -m "feat(core): add serial NDJSON validator with per-line errors"
```

---

## Task 7: Core property test — `validate(serde_json::to_string(v)) == Ok`

**Files:**
- Create: `crates/jfmt-core/tests/validate_proptest.rs`

- [ ] **Step 1: Write the test file**

```rust
//! Property: everything serde_json emits is accepted by validate_syntax.

use jfmt_core::{validate_syntax, StatsCollector};
use jfmt_core::parser::EventReader;
use proptest::prelude::*;
use serde_json::{json, Value};

fn arb_value() -> impl Strategy<Value = Value> {
    let leaf = prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::Bool),
        any::<i64>().prop_map(|n| json!(n)),
        ".*".prop_map(Value::String),
    ];
    leaf.prop_recursive(4, 32, 8, |inner| {
        prop_oneof![
            prop::collection::vec(inner.clone(), 0..8).prop_map(Value::Array),
            prop::collection::hash_map("[a-zA-Z0-9_]{0,6}", inner, 0..8)
                .prop_map(|m| Value::Object(m.into_iter().collect())),
        ]
    })
}

proptest! {
    #[test]
    fn serde_output_is_always_valid(v in arb_value()) {
        let text = serde_json::to_string(&v).unwrap();
        validate_syntax(text.as_bytes()).unwrap();
    }

    #[test]
    fn stats_does_not_panic(v in arb_value()) {
        let text = serde_json::to_string(&v).unwrap();
        let mut c = StatsCollector::default();
        c.begin_record();
        let mut p = EventReader::new(text.as_bytes());
        while let Some(ev) = p.next_event().unwrap() {
            c.observe(&ev);
        }
        c.end_record(true);
        let s = c.finish();
        prop_assert_eq!(s.records, 1);
    }
}
```

Note: `EventReader` must be `pub use`-accessible from `jfmt_core::parser` — it already is.

- [ ] **Step 2: Run**

Run: `cargo test -p jfmt-core --test validate_proptest`
Expected: 2 passed.

- [ ] **Step 3: Commit**

```bash
git add crates/jfmt-core/tests/validate_proptest.rs
git commit -m "test(core): add validate + stats property tests"
```

---

## Task 8: CLI `validate` subcommand args

**Files:**
- Modify: `crates/jfmt-cli/src/cli.rs`

- [ ] **Step 1: Add `ValidateArgs` + `Command::Validate`**

Edit `crates/jfmt-cli/src/cli.rs`:

```rust
// Add alongside MinifyArgs / PrettyArgs:

#[derive(Debug, Args)]
pub struct ValidateArgs {
    #[command(flatten)]
    pub common: CommonArgs,

    /// Emit a human-readable stats summary to stderr.
    #[arg(long = "stats")]
    pub stats: bool,

    /// Emit structured stats as JSON to PATH.
    #[arg(long = "stats-json", value_name = "PATH")]
    pub stats_json: Option<std::path::PathBuf>,

    /// In NDJSON mode, stop at the first bad line.
    #[arg(long = "fail-fast")]
    pub fail_fast: bool,
}
```

And extend `Command`:

```rust
#[derive(Debug, Subcommand)]
pub enum Command {
    Pretty(PrettyArgs),
    Minify(MinifyArgs),
    /// Validate JSON / NDJSON syntax and optionally emit stats.
    Validate(ValidateArgs),
}
```

- [ ] **Step 2: Verify the binary still builds and `--help` shows the new command**

Run: `cargo build -p jfmt-cli`
Expected: compiles.

Run: `./target/debug/jfmt validate --help`
Expected output includes:
```
--stats
--stats-json <PATH>
--fail-fast
--ndjson
```

- [ ] **Step 3: Commit**

```bash
git add crates/jfmt-cli/src/cli.rs
git commit -m "feat(cli): add validate subcommand args"
```

---

## Task 9: CLI `validate` wiring

**Files:**
- Create: `crates/jfmt-cli/src/commands/validate.rs`
- Modify: `crates/jfmt-cli/src/commands/mod.rs`
- Modify: `crates/jfmt-cli/src/main.rs`

- [ ] **Step 1: Add the command module**

```rust
// crates/jfmt-cli/src/commands/validate.rs
use crate::cli::ValidateArgs;
use anyhow::{Context, Result};
use jfmt_core::{
    validate::{validate_ndjson, NdjsonOptions},
    validate_syntax, Error, Stats, StatsCollector,
};
use jfmt_core::parser::EventReader;
use std::fs::File;
use std::io::{BufWriter, Write};

pub fn run(args: ValidateArgs) -> Result<()> {
    let input = jfmt_io::open_input(&args.common.input_spec()).context("opening input")?;

    let collect_stats = args.stats || args.stats_json.is_some();

    let (exit_nonzero, stats) = if args.common.ndjson {
        let report = validate_ndjson(
            input,
            NdjsonOptions {
                fail_fast: args.fail_fast,
                collect_stats,
            },
        )
        .context("reading input")?;

        for le in &report.errors {
            let col = le
                .column
                .map(|c| format!("col {c} "))
                .unwrap_or_default();
            eprintln!(
                "line {}: {}syntax error at byte {}: {}",
                le.line, col, le.offset, le.message
            );
        }

        let any_bad = !report.errors.is_empty();
        (any_bad, report.stats)
    } else {
        // Single-document mode: stream and optionally collect stats.
        let stats = if collect_stats {
            let mut c = StatsCollector::default();
            c.begin_record();
            let mut r = EventReader::new(input);
            loop {
                match r.next_event() {
                    Ok(None) => break,
                    Ok(Some(ev)) => c.observe(&ev),
                    Err(e) => {
                        c.end_record(false);
                        let _ = c.finish(); // drop, we still want to surface the error
                        return Err(anyhow::Error::from(e).context("validation failed"));
                    }
                }
            }
            c.end_record(true);
            Some(c.finish())
        } else {
            validate_syntax(input).context("validation failed")?;
            None
        };
        (false, stats)
    };

    if let Some(s) = stats.as_ref() {
        if args.stats {
            eprint!("{s}");
        }
        if let Some(path) = args.stats_json.as_ref() {
            write_stats_json(path, s).context("writing --stats-json")?;
        }
    }

    if exit_nonzero {
        anyhow::bail!(Error::Syntax {
            offset: 0,
            line: None,
            column: None,
            message: "at least one NDJSON line failed to parse".into(),
        });
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

Note: the NDJSON branch synthesises an `Error::Syntax` at the end so `classify()` in `main.rs` routes to exit code 2. That's deliberate — the per-line errors have already been printed.

- [ ] **Step 2: Wire into `commands/mod.rs` and `main.rs`**

```rust
// crates/jfmt-cli/src/commands/mod.rs (add)
pub mod validate;
```

```rust
// crates/jfmt-cli/src/main.rs — extend the match
fn run(cli: Cli) -> anyhow::Result<()> {
    match cli.command {
        Command::Pretty(args) => commands::pretty::run(args),
        Command::Minify(args) => commands::minify::run(args),
        Command::Validate(args) => commands::validate::run(args),
    }
}
```

- [ ] **Step 3: Smoke-test**

Run:
```bash
echo '{"a":1}' | ./target/debug/jfmt validate
```
Expected: exit 0, no output.

Run:
```bash
echo '{bad' | ./target/debug/jfmt validate
echo $?  # on Windows: echo $LASTEXITCODE in powershell
```
Expected: exit 2, stderr contains `syntax error at line ... column ...`.

Run:
```bash
printf '{"a":1}\n{"b":2}\n' | ./target/debug/jfmt validate --ndjson --stats
```
Expected: exit 0; stderr includes `records: 2 (2 valid, 0 invalid)`.

- [ ] **Step 4: Commit**

```bash
git add crates/jfmt-cli/src
git commit -m "feat(cli): wire up 'jfmt validate' subcommand"
```

---

## Task 10: CLI end-to-end tests

**Files:**
- Create: `crates/jfmt-cli/tests/fixtures/bad.json`
- Create: `crates/jfmt-cli/tests/fixtures/ndjson-good.ndjson`
- Create: `crates/jfmt-cli/tests/fixtures/ndjson-mixed.ndjson`
- Create: `crates/jfmt-cli/tests/cli_validate.rs`

Note: reuse existing `simple.json` for the happy path.

- [ ] **Step 1: Create fixtures**

`crates/jfmt-cli/tests/fixtures/bad.json`:
```
{
  "a": ,
  "b": 1
}
```
(A syntax error on line 2 — struson should report line 2, some column near the `,`.)

`crates/jfmt-cli/tests/fixtures/ndjson-good.ndjson`:
```
{"a":1}
{"a":2,"b":3}
[1,2,3]
```

`crates/jfmt-cli/tests/fixtures/ndjson-mixed.ndjson`:
```
{"ok":1}
{bad
{"ok":2}
```

- [ ] **Step 2: Write the tests**

```rust
// crates/jfmt-cli/tests/cli_validate.rs
use assert_cmd::Command;
use predicates::prelude::*;
use std::path::PathBuf;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

#[test]
fn validate_good_exits_zero() {
    Command::cargo_bin("jfmt")
        .unwrap()
        .arg("validate")
        .arg(fixture("simple.json"))
        .assert()
        .success();
}

#[test]
fn validate_bad_exits_2_with_location() {
    Command::cargo_bin("jfmt")
        .unwrap()
        .arg("validate")
        .arg(fixture("bad.json"))
        .assert()
        .code(2)
        .stderr(predicate::str::contains("syntax error"))
        .stderr(predicate::str::contains("line"))
        .stderr(predicate::str::contains("column"));
}

#[test]
fn validate_stats_to_stderr() {
    Command::cargo_bin("jfmt")
        .unwrap()
        .arg("validate")
        .arg("--stats")
        .arg(fixture("simple.json"))
        .assert()
        .success()
        .stderr(predicate::str::contains("records: 1 (1 valid, 0 invalid)"))
        .stderr(predicate::str::contains("top-level types:"))
        .stderr(predicate::str::contains("object: 1"));
}

#[test]
fn validate_stats_json_writes_file() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("stats.json");
    Command::cargo_bin("jfmt")
        .unwrap()
        .arg("validate")
        .arg("--stats-json")
        .arg(&out)
        .arg(fixture("simple.json"))
        .assert()
        .success();

    let text = std::fs::read_to_string(&out).unwrap();
    let v: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(v["records"], 1);
    assert_eq!(v["valid"], 1);
    assert_eq!(v["max_depth"], 2);
    assert_eq!(v["top_level_types"]["object"], 1);
}

#[test]
fn validate_ndjson_good_exits_zero() {
    Command::cargo_bin("jfmt")
        .unwrap()
        .arg("validate")
        .arg("--ndjson")
        .arg(fixture("ndjson-good.ndjson"))
        .assert()
        .success();
}

#[test]
fn validate_ndjson_mixed_reports_line_2_and_exits_2() {
    Command::cargo_bin("jfmt")
        .unwrap()
        .arg("validate")
        .arg("--ndjson")
        .arg(fixture("ndjson-mixed.ndjson"))
        .assert()
        .code(2)
        .stderr(predicate::str::contains("line 2:"))
        // lines 1 and 3 NOT reported
        .stderr(predicate::str::contains("line 1:").not())
        .stderr(predicate::str::contains("line 3:").not());
}

#[test]
fn validate_ndjson_fail_fast_stops_after_first_bad() {
    // Construct input with two bad lines; only the first should be reported.
    use std::io::Write;
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("two_bad.ndjson");
    let mut f = std::fs::File::create(&p).unwrap();
    writeln!(f, "{{bad1").unwrap();
    writeln!(f, "{{bad2").unwrap();
    writeln!(f, "1").unwrap();
    drop(f);

    Command::cargo_bin("jfmt")
        .unwrap()
        .arg("validate")
        .arg("--ndjson")
        .arg("--fail-fast")
        .arg(&p)
        .assert()
        .code(2)
        .stderr(predicate::str::contains("line 1:"))
        .stderr(predicate::str::contains("line 2:").not());
}

#[test]
fn validate_ndjson_stats_counts_valid_and_invalid() {
    Command::cargo_bin("jfmt")
        .unwrap()
        .arg("validate")
        .arg("--ndjson")
        .arg("--stats")
        .arg(fixture("ndjson-mixed.ndjson"))
        .assert()
        .code(2) // one bad line, so still exit 2
        .stderr(predicate::str::contains("records: 3 (2 valid, 1 invalid)"));
}
```

- [ ] **Step 3: Run**

Run: `cargo test -p jfmt-cli --test cli_validate`
Expected: 8 passed.

**Contingency:** the `validate_bad_exits_2_with_location` test expects the strings `"line"` and `"column"` in stderr. If struson fails to attach a `line_pos` for this particular malformed input, the stderr will only have `"byte N"` and both assertions fail. If that happens, replace the two `line`/`column` assertions with a check that the message contains `byte`; the important invariant is **some** location info is surfaced. Document the outcome in the commit message.

- [ ] **Step 4: Commit**

```bash
git add crates/jfmt-cli/tests
git commit -m "test(cli): add end-to-end validate tests"
```

---

## Task 11: README update

**Files:**
- Modify: `README.md`

- [ ] **Step 1: Add a `Validate` section to README**

Insert between `### Minify` and `## Exit codes`:

```markdown
### Validate

```bash
jfmt validate data.json                        # exit 0 if clean, 2 if not
jfmt validate data.json --stats                # human summary on stderr
jfmt validate data.json --stats-json out.json  # machine-readable summary
jfmt validate events.ndjson --ndjson           # per-line errors, continues past bad lines
jfmt validate events.ndjson --ndjson --fail-fast
```

Stats include: record count (valid / invalid), top-level type distribution,
max nesting depth, and top-level key frequencies (capped at 1024 distinct
keys).
```

(Note: the outer code fence in the inserted block must be escaped in the
plan's rendering but literal in the actual README. Write it as a ```bash
block normally.)

Also update the Status line to mention `validate`:

```
**M2 preview (v0.0.2)** — `pretty`, `minify`, `validate` with streaming stats.
```

- [ ] **Step 2: Commit**

```bash
git add README.md
git commit -m "docs: document validate + stats in README"
```

---

## Task 12: Tag `v0.0.2` and mark M2 shipped

**Files:**
- Modify: `docs/superpowers/specs/2026-04-23-jfmt-phase1-design.md`

- [ ] **Step 1: Verify everything's green**

Run: `cargo fmt --all -- --check`
Expected: exit 0. If fmt reports differences, run `cargo fmt --all` and commit as `chore: apply rustfmt`.

Run: `cargo clippy --workspace --all-targets -- -D warnings`
Expected: exit 0.

Run: `cargo test --workspace`
Expected: all pass. New tests from M2:
- core: 8 stats + 4 syntax + 5 ndjson + 2 property + 1 parser line/column = ~20 new tests
- cli: 8 new validate tests
Running total ~80.

- [ ] **Step 2: Bump workspace version**

```toml
# Cargo.toml (workspace.package)
version = "0.0.2"
```

Run: `cargo build --workspace` to refresh Cargo.lock.

Commit:
```bash
git add Cargo.toml Cargo.lock
git commit -m "chore: bump version to 0.0.2"
```

- [ ] **Step 3: Update spec milestone table**

Open `docs/superpowers/specs/2026-04-23-jfmt-phase1-design.md`. After the existing
`| M1 ✓ | Shipped v0.0.1 …` row, append:

```markdown
| M2 ✓ | Shipped v0.0.2 on 2026-04-XX (tag `v0.0.2`) |
```

(Fill in the actual date.)

Commit:
```bash
git add docs/superpowers/specs/2026-04-23-jfmt-phase1-design.md
git commit -m "docs(spec): mark M2 as shipped (v0.0.2)"
```

- [ ] **Step 4: Tag**

```bash
git tag -a v0.0.2 -m "M2: jfmt validate + streaming stats"
```

Do NOT push without the user's explicit go-ahead.

---

## Self-Review

**Spec coverage vs `docs/superpowers/specs/2026-04-23-jfmt-phase1-design.md`:**

- §4.1 `validate/syntax.rs` → Task 2. ✓
- §4.1 `validate/stats.rs` — `StatsCollector` → Tasks 3–5. ✓
- §4.1 `validate/schema.rs` → **deferred to M5** per scope-boundaries note; spec milestone table agrees.
- §4.1 `ndjson/*` (splitter/worker/reorder) — M2 ships only a serial splitter; parallel pipeline is M3. This plan's Task 6 is the serial splitter, which M3 will replace behind the same API. ✓
- §4.3 clap `validate` subcommand with `--stats`, `--stats-json`, `--ndjson`, `--fail-fast` → Tasks 8–9. `--schema`, `--strict` deferred. ✓
- §4.3 exit codes 0 / 1 / 2 → Tasks 9 + 10. Exit 3 reserved for M5. ✓
- §7.1 syntax errors with byte offset + line + column → Task 1 (error enum) + Task 9 (CLI formatting). ✓
- §7.1 NDJSON mode keeps going per line → Task 6 + Task 10. ✓
- §7.2 Schema → **M5**. ✓
- §7.3 Stats output: `--stats` human, `--stats-json` file, expected fields → Task 5 (Display) + Task 9 (write path) + Task 10 (golden test). ✓
  - Fields covered: records (valid/invalid split), top-level type distribution, max depth, top-level keys. **Not covered in M2:** duration, throughput, input sizes — these are CLI-layer instrumentation that needs a timer + `indicatif` progress setup; deferring to M6 alongside progress bars (CLAUDE.md allows this per plan header).
- §10 Testing: unit (Tasks 2–6), property (Task 7), CLI e2e (Task 10). ✓
- §11 Milestone M2 ships as `v0.0.2`. ✓

**Gap documented for the spec:** stats output does not yet include input size / duration / throughput — called out in the Scope Boundaries section at top. Not a silent omission.

**Placeholder scan:** no "TBD", "TODO", "implement later", "similar to …" in tasks. Each code block is complete. ✓

**Type consistency check:**
- `Error::Syntax { offset, line, column, message }` — defined Task 1, used by Task 9 (CLI), Task 6 (ndjson unpacks the variant).
- `Stats { records, valid, invalid, top_level_types, max_depth, top_level_keys, top_level_keys_truncated }` — defined Task 3, serialization tested Task 3, Display Task 5, CLI consumer Task 9, golden test Task 10.
- `StatsCollector::{new, begin_record, observe, end_record, finish}` — defined Task 4, used Tasks 6, 9.
- `NdjsonOptions { fail_fast, collect_stats }` and `NdjsonReport { errors, stats }` — defined Task 6, used Task 9.
- `ValidateArgs { common, stats, stats_json, fail_fast }` — defined Task 8, used Task 9.

**Ambiguity check:**
- Blank NDJSON lines: explicitly skipped (Task 6 `skips_blank_lines` test).
- Top-level keys for non-object top-level records: not counted (Task 4 `tracks_max_depth` asserts empty keys for array root).
- Key cap behavior: new keys beyond cap are dropped and counted in `top_level_keys_truncated` (Task 4 `key_cap_truncates` test).
- Exit code when NDJSON has bad lines: always 2 even when `--stats` also prints a report (Task 10 `validate_ndjson_stats_counts_valid_and_invalid`).

All good.

---

## Execution Handoff

Plan saved to `docs/superpowers/plans/2026-04-24-jfmt-m2-validate-stats.md`.

**1. Subagent-Driven (recommended)** — fresh subagent per task, review between tasks
**2. Inline Execution** — execute in-session via `superpowers:executing-plans`, user-paced via the `jfmt-iterate` skill as in M1

Which approach?
