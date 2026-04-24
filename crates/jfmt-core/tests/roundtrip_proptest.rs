//! Property tests for parser + writer round-trips.

use jfmt_core::{transcode, MinifyWriter, PrettyWriter};
use proptest::prelude::*;
use serde_json::{json, Value};

/// Generator for small arbitrary JSON values (bounded depth to keep tests fast).
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

fn minify_via_core(input: &str) -> String {
    let mut out = Vec::new();
    transcode(input.as_bytes(), MinifyWriter::new(&mut out)).unwrap();
    String::from_utf8(out).unwrap()
}

fn pretty_via_core(input: &str) -> String {
    let mut out = Vec::new();
    transcode(input.as_bytes(), PrettyWriter::new(&mut out)).unwrap();
    String::from_utf8(out).unwrap()
}

proptest! {
    #[test]
    fn minify_preserves_semantics(v in arb_value()) {
        let text = serde_json::to_string(&v).unwrap();
        let minified = minify_via_core(&text);
        let reparsed: Value = serde_json::from_str(&minified).unwrap();
        prop_assert_eq!(reparsed, v);
    }

    #[test]
    fn pretty_preserves_semantics(v in arb_value()) {
        let text = serde_json::to_string(&v).unwrap();
        let pretty = pretty_via_core(&text);
        let reparsed: Value = serde_json::from_str(&pretty).unwrap();
        prop_assert_eq!(reparsed, v);
    }

    #[test]
    fn pretty_then_minify_is_canonical(v in arb_value()) {
        let text = serde_json::to_string(&v).unwrap();
        let via_pretty = minify_via_core(&pretty_via_core(&text));
        let direct = minify_via_core(&text);
        prop_assert_eq!(via_pretty, direct);
    }
}
