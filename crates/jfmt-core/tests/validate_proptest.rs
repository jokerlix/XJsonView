//! Property: everything serde_json emits is accepted by validate_syntax.

use jfmt_core::parser::EventReader;
use jfmt_core::{validate_syntax, StatsCollector};
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
