//! Streaming statistics collected while parsing a JSON (or NDJSON) input.

use crate::event::{Event, Scalar};
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

#[derive(Debug, Clone)]
pub struct StatsConfig {
    /// Maximum distinct top-level object keys to remember. Once hit, further
    /// new keys are counted in `top_level_keys_truncated` and discarded.
    pub top_level_keys_cap: usize,
}

impl Default for StatsConfig {
    fn default() -> Self {
        Self {
            top_level_keys_cap: 1024,
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
