//! Event stream types shared by the parser and writers.

/// A JSON scalar: anything that is not a container.
#[derive(Debug, Clone, PartialEq)]
pub enum Scalar {
    /// A JSON string (already unescaped).
    String(String),
    /// A JSON number, preserved as its original lexical form so that
    /// precision is not lost. E.g. `"1.0"`, `"1e10"`, `"-0"`.
    Number(String),
    /// `true` or `false`.
    Bool(bool),
    /// `null`.
    Null,
}

/// One token in the event-driven JSON stream.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    StartObject,
    EndObject,
    StartArray,
    EndArray,
    /// A key inside an object. Always immediately followed by a value event
    /// (scalar, StartObject, or StartArray).
    Name(String),
    /// A scalar value. May appear at the top level, inside an array, or as
    /// the value of a name in an object.
    Value(Scalar),
}

impl Event {
    /// True if this event opens a new container.
    pub fn is_start(&self) -> bool {
        matches!(self, Event::StartObject | Event::StartArray)
    }

    /// True if this event closes a container.
    pub fn is_end(&self) -> bool {
        matches!(self, Event::EndObject | Event::EndArray)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_start_and_end_classify_correctly() {
        assert!(Event::StartObject.is_start());
        assert!(Event::StartArray.is_start());
        assert!(!Event::Name("x".into()).is_start());
        assert!(!Event::Value(Scalar::Null).is_start());

        assert!(Event::EndObject.is_end());
        assert!(Event::EndArray.is_end());
        assert!(!Event::StartObject.is_end());
    }

    #[test]
    fn scalar_equality_is_by_value() {
        assert_eq!(Scalar::Number("1.0".into()), Scalar::Number("1.0".into()));
        assert_ne!(Scalar::Number("1".into()), Scalar::Number("1.0".into()));
        assert_eq!(Scalar::Bool(true), Scalar::Bool(true));
    }
}
