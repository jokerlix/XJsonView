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
