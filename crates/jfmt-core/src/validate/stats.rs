//! Streaming statistics collected while parsing a JSON (or NDJSON) input.

use crate::event::{Event, Scalar};
use serde::Serialize;
use std::collections::BTreeMap;
use std::fmt;

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
}

fn is_zero_u64(n: &u64) -> bool {
    *n == 0
}

impl Stats {
    /// Merge `other` into `self`. Commutative. Cap on
    /// `top_level_keys` is a per-pass guard, not a post-merge
    /// invariant — merged maps may exceed any individual collector's
    /// cap.
    pub fn merge(&mut self, other: Stats) {
        self.records += other.records;
        self.valid += other.valid;
        self.invalid += other.invalid;
        if other.max_depth > self.max_depth {
            self.max_depth = other.max_depth;
        }
        for (k, v) in other.top_level_types {
            *self.top_level_types.entry(k).or_insert(0) += v;
        }
        for (k, v) in other.top_level_keys {
            *self.top_level_keys.entry(k).or_insert(0) += v;
        }
        self.top_level_keys_truncated += other.top_level_keys_truncated;
        self.schema_pass += other.schema_pass;
        self.schema_fail += other.schema_fail;
        for (k, v) in other.top_violation_paths {
            *self.top_violation_paths.entry(k).or_insert(0) += v;
        }
    }
}

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
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct StatsConfig {
    /// Maximum distinct top-level object keys to remember. Once hit, further
    /// new keys are counted in `top_level_keys_truncated` and discarded.
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

/// Streaming collector. Lifecycle per record:
///   `begin_record()` → `observe(&Event)` × N → `end_record(valid)`
/// Call `finish()` once to consume the collector and get the [`Stats`].
pub struct StatsCollector {
    cfg: StatsConfig,
    stats: Stats,
    /// Current container depth inside the in-flight record (0 = not inside).
    depth: u64,
    /// `true` between a `StartObject` at depth 0→1 and its `EndObject`.
    in_top_level_object: bool,
    /// `true` once the record has contributed a type to the distribution.
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
                self.expecting_name_at_depth_1 = self.depth == 1 && self.in_top_level_object;
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
                self.expecting_name_at_depth_1 = self.depth == 1 && self.in_top_level_object;
            }
        }
    }

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
        // are at or below cap. Stable tie-break by key (lex-largest
        // gets evicted first so we keep alphabetically-earlier names).
        let cap = self.cfg.top_violation_paths_cap;
        if cap > 0 {
            while self.stats.top_violation_paths.len() > cap {
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
        let j: serde_json::Value = serde_json::to_value(s).unwrap();
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
        assert!(
            text.contains("records: 2 (1 valid, 1 invalid)"),
            "got: {text}"
        );
        assert!(text.contains("max depth: 3"));
        assert!(text.contains("top-level types:"));
        assert!(text.contains("  object: 2"));
        assert!(text.contains("top-level keys:"));
        assert!(text.contains("  a: 2"));
    }

    #[test]
    fn merge_sums_counts_and_unions_keys() {
        let mut a = Stats {
            records: 2,
            valid: 2,
            invalid: 0,
            max_depth: 3,
            ..Default::default()
        };
        a.top_level_types.insert("object".into(), 2);
        a.top_level_keys.insert("x".into(), 2);
        a.top_level_keys.insert("only_in_a".into(), 1);
        a.top_level_keys_truncated = 1;

        let mut b = Stats {
            records: 1,
            valid: 0,
            invalid: 1,
            max_depth: 5,
            ..Default::default()
        };
        b.top_level_types.insert("object".into(), 1);
        b.top_level_types.insert("array".into(), 1);
        b.top_level_keys.insert("x".into(), 3);
        b.top_level_keys.insert("only_in_b".into(), 2);
        b.top_level_keys_truncated = 4;

        a.merge(b);

        assert_eq!(a.records, 3);
        assert_eq!(a.valid, 2);
        assert_eq!(a.invalid, 1);
        assert_eq!(a.max_depth, 5);
        assert_eq!(a.top_level_types.get("object"), Some(&3));
        assert_eq!(a.top_level_types.get("array"), Some(&1));
        assert_eq!(a.top_level_keys.get("x"), Some(&5));
        assert_eq!(a.top_level_keys.get("only_in_a"), Some(&1));
        assert_eq!(a.top_level_keys.get("only_in_b"), Some(&2));
        assert_eq!(a.top_level_keys_truncated, 5);
    }

    #[test]
    fn display_shows_truncated_note() {
        let mut s = Stats::default();
        s.top_level_keys.insert("a".into(), 1);
        s.top_level_keys_truncated = 5;
        let text = format!("{s}");
        assert!(
            text.contains("5 distinct keys beyond cap dropped"),
            "got: {text}"
        );
    }

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
}

#[cfg(test)]
mod collector_tests {
    use super::*;

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
        let mut c = StatsCollector::new(StatsConfig {
            top_level_keys_cap: 2,
            ..StatsConfig::default()
        });
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
    fn repeated_keys_increment_counter() {
        let mut c = StatsCollector::default();
        c.begin_record();
        c.observe(&Event::StartObject);
        for _ in 0..3 {
            c.observe(&Event::Name("a".into()));
            c.observe(&Event::Value(Scalar::Null));
        }
        c.observe(&Event::EndObject);
        c.end_record(true);
        let s = c.finish();
        assert_eq!(s.top_level_keys.get("a"), Some(&3));
    }

    #[test]
    fn invalid_record_counts_separately() {
        let mut c = StatsCollector::default();
        c.begin_record();
        c.observe(&Event::StartObject);
        c.end_record(false);
        let s = c.finish();
        assert_eq!(s.records, 1);
        assert_eq!(s.valid, 0);
        assert_eq!(s.invalid, 1);
    }

    #[test]
    fn multiple_records_accumulate() {
        let mut c = StatsCollector::default();
        for _ in 0..2 {
            c.begin_record();
            c.observe(&Event::StartObject);
            c.observe(&Event::Name("x".into()));
            c.observe(&Event::Value(Scalar::Number("1".into())));
            c.observe(&Event::EndObject);
            c.end_record(true);
        }
        let s = c.finish();
        assert_eq!(s.records, 2);
        assert_eq!(s.top_level_types.get("object"), Some(&2));
        assert_eq!(s.top_level_keys.get("x"), Some(&2));
    }
}
