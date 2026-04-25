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
