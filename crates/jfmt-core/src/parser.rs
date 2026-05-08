//! Event-driven JSON reader built on top of `struson`.

use crate::error::{Error, Result};
use crate::event::{Event, Scalar};
use std::io::Read;
use struson::reader::{JsonReader, JsonStreamReader, ReaderError, ValueType};

/// A pull-based iterator of [`Event`]s over a JSON byte stream.
///
/// Uses constant memory proportional to nesting depth.
pub struct EventReader<R: Read> {
    inner: JsonStreamReader<R>,
    /// Stack of container kinds currently open.
    stack: Vec<Container>,
    /// `true` once the top-level value has been fully consumed.
    done: bool,
    /// `true` if the next event inside an object should be a `Name`.
    expect_name: bool,
}

#[derive(Copy, Clone, Debug, PartialEq)]
enum Container {
    Object,
    Array,
}

impl<R: Read> EventReader<R> {
    pub fn new(source: R) -> Self {
        Self {
            inner: JsonStreamReader::new(source),
            stack: Vec::new(),
            done: false,
            expect_name: false,
        }
    }

    /// Return the next event, or `Ok(None)` after the document ends.
    pub fn next_event(&mut self) -> Result<Option<Event>> {
        if self.done {
            return Ok(None);
        }
        let event = self.read_one()?;
        // If after this event no containers are open, the document is done.
        if self.stack.is_empty() {
            self.done = true;
        }
        Ok(Some(event))
    }

    fn read_one(&mut self) -> Result<Event> {
        // Inside an object, the next slot is a Name — unless the object is
        // empty, in which case we close it.
        if self.expect_name {
            if self.inner.has_next().map_err(map_err)? {
                let name = self.inner.next_name_owned().map_err(map_err)?;
                self.expect_name = false;
                return Ok(Event::Name(name));
            } else {
                self.inner.end_object().map_err(map_err)?;
                self.pop_container();
                return Ok(Event::EndObject);
            }
        }

        // Inside an array, close it if there are no more elements.
        if matches!(self.stack.last(), Some(Container::Array))
            && !self.inner.has_next().map_err(map_err)?
        {
            self.inner.end_array().map_err(map_err)?;
            self.pop_container();
            return Ok(Event::EndArray);
        }

        // Read a value (or start of container).
        let vt = self.inner.peek().map_err(map_err)?;
        let event = match vt {
            ValueType::Array => {
                self.inner.begin_array().map_err(map_err)?;
                self.stack.push(Container::Array);
                Event::StartArray
            }
            ValueType::Object => {
                self.inner.begin_object().map_err(map_err)?;
                self.stack.push(Container::Object);
                self.expect_name = true;
                Event::StartObject
            }
            ValueType::String => {
                Event::Value(Scalar::String(self.inner.next_string().map_err(map_err)?))
            }
            ValueType::Number => Event::Value(Scalar::Number(
                self.inner.next_number_as_string().map_err(map_err)?,
            )),
            ValueType::Boolean => {
                Event::Value(Scalar::Bool(self.inner.next_bool().map_err(map_err)?))
            }
            ValueType::Null => {
                self.inner.next_null().map_err(map_err)?;
                Event::Value(Scalar::Null)
            }
        };

        // After a value event inside an object, we must read a name next.
        // (A freshly pushed Object/Array is handled above via expect_name.)
        if matches!(self.stack.last(), Some(Container::Object))
            && !matches!(event, Event::StartObject | Event::StartArray)
        {
            self.expect_name = true;
        }

        Ok(event)
    }

    fn pop_container(&mut self) {
        self.stack.pop();
        // After popping, if we're now inside an object, the next slot is a Name.
        self.expect_name = matches!(self.stack.last(), Some(Container::Object));
    }

    /// Current nesting depth (0 = top level).
    pub fn depth(&self) -> usize {
        self.stack.len()
    }

    /// Byte offset of the next token in the input stream.
    ///
    /// Call before `next_event()` to capture the offset of the token about
    /// to be consumed (e.g. the offset of the opening `{` for a container
    /// that the next call returns as `StartObject`). After `next_event()`,
    /// returns the offset *past* the consumed token (e.g. the byte after
    /// the closing `}` for an `EndObject`).
    ///
    /// Returns 0 if the underlying reader does not track positions
    /// (currently always available with `JsonStreamReader`).
    pub fn byte_offset(&self) -> u64 {
        self.inner
            .current_position(false)
            .data_pos
            .unwrap_or(0)
    }

    /// After the top-level value has been consumed, verify no non-whitespace
    /// bytes remain. Consumes the reader. Call this when strict validation
    /// is required (e.g. `validate_syntax`); transcoding paths typically skip it.
    pub fn finish(self) -> Result<()> {
        self.inner.consume_trailing_whitespace().map_err(|e| {
            // consume_trailing_whitespace returns ReaderError; map via the
            // same helper the rest of the reader uses.
            match e {
                struson::reader::ReaderError::IoError { error, .. } => Error::Io(error),
                struson::reader::ReaderError::SyntaxError(se) => Error::Syntax {
                    offset: se.location.data_pos.unwrap_or(0),
                    line: se.location.line_pos.as_ref().map(|lp| lp.line),
                    column: se.location.line_pos.as_ref().map(|lp| lp.column),
                    message: format!("{:?}", se.kind),
                },
                other => Error::Syntax {
                    offset: 0,
                    line: None,
                    column: None,
                    message: format!("{other}"),
                },
            }
        })
    }
}

fn map_err(e: ReaderError) -> Error {
    match e {
        ReaderError::IoError { error, .. } => Error::Io(error),
        ReaderError::SyntaxError(se) => Error::Syntax {
            offset: se.location.data_pos.unwrap_or(0),
            line: se.location.line_pos.as_ref().map(|lp| lp.line),
            column: se.location.line_pos.as_ref().map(|lp| lp.column),
            message: format!("{:?}", se.kind),
        },
        other => Error::Syntax {
            offset: 0,
            line: None,
            column: None,
            message: format!("{other}"),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn events_of(json: &str) -> Vec<Event> {
        let mut r = EventReader::new(json.as_bytes());
        let mut out = Vec::new();
        while let Some(e) = r.next_event().unwrap() {
            out.push(e);
        }
        out
    }

    #[test]
    fn reads_scalar_string() {
        assert_eq!(
            events_of("\"hello\""),
            vec![Event::Value(Scalar::String("hello".into()))]
        );
    }

    #[test]
    fn reads_scalar_number_preserving_form() {
        assert_eq!(
            events_of("1.0"),
            vec![Event::Value(Scalar::Number("1.0".into()))]
        );
        assert_eq!(
            events_of("-0"),
            vec![Event::Value(Scalar::Number("-0".into()))]
        );
    }

    #[test]
    fn reads_empty_array() {
        assert_eq!(events_of("[]"), vec![Event::StartArray, Event::EndArray]);
    }

    #[test]
    fn reads_empty_object() {
        assert_eq!(events_of("{}"), vec![Event::StartObject, Event::EndObject]);
    }

    #[test]
    fn reads_flat_array() {
        let e = events_of("[1, true, null, \"x\"]");
        assert_eq!(
            e,
            vec![
                Event::StartArray,
                Event::Value(Scalar::Number("1".into())),
                Event::Value(Scalar::Bool(true)),
                Event::Value(Scalar::Null),
                Event::Value(Scalar::String("x".into())),
                Event::EndArray,
            ]
        );
    }

    #[test]
    fn reads_nested_object() {
        let e = events_of(r#"{"a": {"b": [1, 2]}, "c": null}"#);
        assert_eq!(
            e,
            vec![
                Event::StartObject,
                Event::Name("a".into()),
                Event::StartObject,
                Event::Name("b".into()),
                Event::StartArray,
                Event::Value(Scalar::Number("1".into())),
                Event::Value(Scalar::Number("2".into())),
                Event::EndArray,
                Event::EndObject,
                Event::Name("c".into()),
                Event::Value(Scalar::Null),
                Event::EndObject,
            ]
        );
    }

    #[test]
    fn reports_syntax_error_with_offset() {
        let mut r = EventReader::new(b"{\"a\":,}".as_slice());
        let err = loop {
            match r.next_event() {
                Ok(None) => panic!("expected error"),
                Ok(Some(_)) => continue,
                Err(e) => break e,
            }
        };
        assert!(matches!(err, Error::Syntax { .. }), "got {err:?}");
    }

    #[test]
    fn syntax_error_carries_line_and_column() {
        let mut r = EventReader::new(b"{\n  \"a\":,\n}".as_slice());
        let err = loop {
            match r.next_event() {
                Ok(None) => panic!("expected error"),
                Ok(Some(_)) => continue,
                Err(e) => break e,
            }
        };
        match err {
            Error::Syntax { line, column, .. } => {
                assert!(line.is_some(), "line not populated");
                assert!(column.is_some(), "column not populated");
            }
            other => panic!("got {other:?}"),
        }
    }

    #[test]
    fn byte_offset_advances_through_events() {
        let mut r = EventReader::new(br#"{"a":1}"#.as_slice());
        // Before first read, position is 0.
        assert_eq!(r.byte_offset(), 0);
        // StartObject consumes `{` (1 byte).
        assert!(matches!(r.next_event().unwrap(), Some(Event::StartObject)));
        let after_open = r.byte_offset();
        assert!(after_open >= 1, "got {after_open}");
        // Walk to end.
        while r.next_event().unwrap().is_some() {}
        // After EndObject, offset is past the closing brace (7 bytes total).
        assert_eq!(r.byte_offset(), 7);
    }
}
