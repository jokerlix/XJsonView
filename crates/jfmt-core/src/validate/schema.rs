//! JSON Schema validation. Wraps the `jsonschema` crate behind a
//! Send+Sync, Arc-shareable `SchemaValidator`. Normalises validation
//! errors into our `Violation` struct. See spec §4.4 + Annex C.
//!
//! The chosen jsonschema 0.18.3 API:
//! - Compile: `JSONSchema::compile(&Value) -> Result<JSONSchema, ValidationError<'static>>`
//! - Validate: `JSONSchema::validate(&self, &Value) -> Result<(), ErrorIterator>`

use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;
use thiserror::Error;

/// One validation violation. The `instance_path` is the JSON Pointer
/// inside the *validated value*; the `keyword` is the jsonschema
/// rule category that failed.
#[derive(Debug, Clone, PartialEq)]
pub struct Violation {
    pub instance_path: String,
    pub keyword: &'static str,
    pub message: String,
}

#[derive(Debug, Error)]
pub enum SchemaError {
    #[error("could not read schema file {path:?}: {source}")]
    BadSchemaFile {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("schema file is not valid JSON: {0}")]
    BadSchemaJson(#[from] serde_json::Error),

    #[error("not a valid JSON Schema: {msg}")]
    BadSchema { msg: String },
}

/// Compiled, shareable schema validator. `Clone` is cheap (Arc bump).
#[derive(Clone)]
pub struct SchemaValidator {
    inner: Arc<jsonschema::JSONSchema>,
}

impl SchemaValidator {
    /// Compile a schema from a parsed `serde_json::Value`.
    pub fn compile(schema: &Value) -> Result<Self, SchemaError> {
        let v = jsonschema::JSONSchema::compile(schema)
            .map_err(|e| SchemaError::BadSchema { msg: format!("{e}") })?;
        Ok(Self {
            inner: Arc::new(v),
        })
    }

    /// Validate one value. Returns 0..N violations.
    pub fn validate(&self, value: &Value) -> Vec<Violation> {
        match self.inner.validate(value) {
            Ok(()) => Vec::new(),
            Err(iter) => iter
                .map(|e| Violation {
                    instance_path: e.instance_path.to_string(),
                    keyword: keyword_name(&e.kind),
                    message: format!("{e}"),
                })
                .collect(),
        }
    }
}

/// Map a `ValidationErrorKind` variant to a stable keyword name.
/// Anything we don't explicitly recognise falls through to "schema".
fn keyword_name(kind: &jsonschema::error::ValidationErrorKind) -> &'static str {
    use jsonschema::error::ValidationErrorKind as K;
    match kind {
        K::Type { .. } => "type",
        K::Required { .. } => "required",
        K::Pattern { .. } => "pattern",
        K::AdditionalProperties { .. } => "additionalProperties",
        K::AdditionalItems { .. } => "additionalItems",
        K::Enum { .. } => "enum",
        K::Format { .. } => "format",
        K::Minimum { .. } => "minimum",
        K::Maximum { .. } => "maximum",
        K::ExclusiveMinimum { .. } => "exclusiveMinimum",
        K::ExclusiveMaximum { .. } => "exclusiveMaximum",
        K::MinLength { .. } => "minLength",
        K::MaxLength { .. } => "maxLength",
        K::MinItems { .. } => "minItems",
        K::MaxItems { .. } => "maxItems",
        K::MinProperties { .. } => "minProperties",
        K::MaxProperties { .. } => "maxProperties",
        K::UniqueItems => "uniqueItems",
        K::OneOfNotValid { .. } | K::OneOfMultipleValid { .. } => "oneOf",
        K::AnyOf { .. } => "anyOf",
        K::Not { .. } => "not",
        K::Constant { .. } => "const",
        _ => "schema",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn user_schema() -> Value {
        json!({
            "type": "object",
            "required": ["name", "age"],
            "properties": {
                "name": {"type": "string"},
                "age": {"type": "integer", "minimum": 0}
            }
        })
    }

    #[test]
    fn compile_happy_path() {
        SchemaValidator::compile(&user_schema()).expect("compile");
    }

    #[test]
    fn compile_rejects_invalid_schema() {
        // A schema whose `type` keyword's value is itself the wrong shape.
        let bad = json!({"type": 42});
        assert!(matches!(
            SchemaValidator::compile(&bad),
            Err(SchemaError::BadSchema { .. })
        ));
    }

    #[test]
    fn validate_passing_value_returns_empty() {
        let v = SchemaValidator::compile(&user_schema()).unwrap();
        let value = json!({"name": "alice", "age": 30});
        let violations = v.validate(&value);
        assert!(violations.is_empty(), "expected no violations: {violations:?}");
    }

    #[test]
    fn validate_failing_value_reports_violation() {
        let v = SchemaValidator::compile(&user_schema()).unwrap();
        let value = json!({"name": "bob"}); // missing `age`
        let violations = v.validate(&value);
        assert!(!violations.is_empty());
        // "required" keyword should appear somewhere in the violations.
        assert!(violations.iter().any(|x| x.keyword == "required"));
    }

    #[test]
    fn validate_reports_instance_path() {
        let v = SchemaValidator::compile(&user_schema()).unwrap();
        let value = json!({"name": "carol", "age": -5}); // age < minimum
        let violations = v.validate(&value);
        assert!(!violations.is_empty());
        let by_keyword: Vec<_> = violations
            .iter()
            .filter(|x| x.keyword == "minimum")
            .collect();
        assert!(!by_keyword.is_empty());
        // Instance path should reference the offending field.
        assert!(by_keyword[0].instance_path.contains("age"));
    }

    #[test]
    fn arc_clone_works_across_threads() {
        use std::sync::Arc as StdArc;
        use std::thread;

        let v = SchemaValidator::compile(&user_schema()).unwrap();
        let shared = StdArc::new(v);
        let bad = json!({"name": "x"}); // missing age

        let handles: Vec<_> = (0..4)
            .map(|_| {
                let v = StdArc::clone(&shared);
                let bad = bad.clone();
                thread::spawn(move || {
                    let violations = v.validate(&bad);
                    assert!(!violations.is_empty());
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
    }
}
