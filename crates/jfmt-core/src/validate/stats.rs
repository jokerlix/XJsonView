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
