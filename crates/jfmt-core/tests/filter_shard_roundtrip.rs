//! Property: any serde_json::Value, when serialized and re-parsed
//! through the EventReader and ShardAccumulator, reproduces an
//! equivalent Value sequence.

use jfmt_core::filter::shard::{ShardAccumulator, ShardLocator, TopLevel};
use jfmt_core::EventReader;
use proptest::prelude::*;
use serde_json::Value;

fn arb_value() -> impl Strategy<Value = Value> {
    let leaf = prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::Bool),
        any::<i32>().prop_map(|n| serde_json::json!(n)),
        "[a-zA-Z0-9 ]{0,8}".prop_map(Value::String),
    ];
    leaf.prop_recursive(3, 24, 6, |inner| {
        prop_oneof![
            prop::collection::vec(inner.clone(), 0..6).prop_map(Value::Array),
            prop::collection::hash_map("[a-z]{1,3}", inner, 0..4)
                .prop_map(|m| Value::Object(m.into_iter().collect())),
        ]
    })
}

proptest! {
    #[test]
    fn roundtrip_preserves_value(v in arb_value()) {
        let bytes = serde_json::to_vec(&v).unwrap();
        let mut reader = EventReader::new(&bytes[..]);
        let mut acc = ShardAccumulator::new();
        let mut shards = Vec::new();
        while let Some(ev) = reader.next_event().unwrap() {
            if let Some(s) = acc.push(ev).unwrap() {
                shards.push(s);
            }
        }
        let top = acc.top_level().expect("top decided");
        match top {
            TopLevel::Array => {
                let want = v.as_array().unwrap();
                prop_assert_eq!(shards.len(), want.len());
                for (i, s) in shards.iter().enumerate() {
                    prop_assert_eq!(s.locator.clone(), ShardLocator::Index(i as u64));
                    prop_assert_eq!(&s.value, &want[i]);
                }
            }
            TopLevel::Object => {
                let want = v.as_object().unwrap();
                prop_assert_eq!(shards.len(), want.len());
                for s in &shards {
                    let key = match &s.locator {
                        ShardLocator::Key(k) => k,
                        _ => panic!("expected Key locator"),
                    };
                    prop_assert_eq!(&s.value, want.get(key).unwrap());
                }
            }
            TopLevel::Scalar => {
                prop_assert_eq!(shards.len(), 1);
                prop_assert_eq!(&shards[0].value, &v);
            }
        }
    }
}
