//! Property test: arbitrary JSON → write to bytes → index → reconstruct via
//! get_value at root → must equal serde_json::to_value of the original.
//!
//! NOTE: The root value is always an Object or Array because Session::get_value
//! looks up NodeId::ROOT in the sparse index, which only contains container
//! entries.  Top-level scalars produce an empty index and would return
//! InvalidNode.  Restricting the root to containers keeps the API simple.

use jfmt_viewer_core::{NodeId, Session};
use proptest::prelude::*;
use serde_json::Value;
use std::io::Write;

fn arb_leaf() -> impl Strategy<Value = Value> {
    prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::Bool),
        any::<i32>().prop_map(|n| Value::Number(n.into())),
        "[a-z0-9]{0,16}".prop_map(Value::String),
    ]
}

fn arb_value(depth: u32) -> impl Strategy<Value = Value> {
    let leaf = arb_leaf();
    leaf.prop_recursive(depth, 32, 8, move |inner| {
        prop_oneof![
            proptest::collection::vec(inner.clone(), 0..6).prop_map(Value::Array),
            proptest::collection::vec(("[a-z]{1,8}", inner), 0..6).prop_map(|kv| {
                let mut m = serde_json::Map::new();
                for (k, v) in kv {
                    m.insert(k, v);
                }
                Value::Object(m)
            }),
        ]
    })
}

/// Always produce an Object or Array at the root — top-level scalars are not
/// supported by Session::get_value (sparse index only indexes containers).
fn arb_container(depth: u32) -> impl Strategy<Value = Value> {
    let inner = arb_value(depth).boxed();
    prop_oneof![
        proptest::collection::vec(inner.clone(), 0..6).prop_map(Value::Array),
        proptest::collection::vec(("[a-z]{1,8}", inner), 0..6).prop_map(|kv| {
            let mut m = serde_json::Map::new();
            for (k, v) in kv {
                m.insert(k, v);
            }
            Value::Object(m)
        }),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn root_get_value_matches_input(v in arb_container(4)) {
        let serialized = serde_json::to_vec(&v).unwrap();
        let mut tmp = tempfile::Builder::new()
            .suffix(".json")
            .tempfile()
            .unwrap();
        tmp.write_all(&serialized).unwrap();
        tmp.flush().unwrap();

        let s = Session::open(tmp.path()).unwrap();
        let resp = s.get_value(NodeId::ROOT, None).unwrap();
        let parsed: Value = serde_json::from_str(&resp.json).unwrap();
        prop_assert_eq!(parsed, v);
    }
}
