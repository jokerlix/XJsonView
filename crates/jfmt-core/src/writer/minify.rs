//! Minified output: zero whitespace, shortest valid JSON.

use crate::escape::write_json_string;
use crate::event::{Event, Scalar};
use crate::writer::EventWriter;
use crate::{Error, Result};
use std::io::Write;

/// Minified JSON writer.
pub struct MinifyWriter<W: Write> {
    w: W,
    stack: Vec<Frame>,
}

struct Frame {
    in_object: bool,
    /// Have we emitted any child yet? Used to place commas.
    first: bool,
    /// When in an object, have we just emitted a Name and are waiting for a value?
    pending_name: bool,
}

impl<W: Write> MinifyWriter<W> {
    pub fn new(w: W) -> Self {
        Self {
            w,
            stack: Vec::new(),
        }
    }

    /// Called before writing a *value* (scalar or container start) — NOT a Name.
    fn write_separator(&mut self) -> Result<()> {
        if let Some(top) = self.stack.last_mut() {
            if top.pending_name {
                self.w.write_all(b":")?;
                top.pending_name = false;
                return Ok(());
            }
            if !top.first {
                self.w.write_all(b",")?;
            }
            top.first = false;
        }
        Ok(())
    }

    fn write_scalar(&mut self, s: &Scalar) -> Result<()> {
        match s {
            Scalar::String(s) => write_json_string(&mut self.w, s)?,
            Scalar::Number(n) => self.w.write_all(n.as_bytes())?,
            Scalar::Bool(true) => self.w.write_all(b"true")?,
            Scalar::Bool(false) => self.w.write_all(b"false")?,
            Scalar::Null => self.w.write_all(b"null")?,
        }
        Ok(())
    }
}

impl<W: Write> EventWriter for MinifyWriter<W> {
    fn write_event(&mut self, event: &Event) -> Result<()> {
        match event {
            Event::StartObject => {
                self.write_separator()?;
                self.w.write_all(b"{")?;
                self.stack.push(Frame {
                    in_object: true,
                    first: true,
                    pending_name: false,
                });
            }
            Event::EndObject => {
                let frame = self
                    .stack
                    .pop()
                    .ok_or_else(|| Error::State("EndObject without StartObject".into()))?;
                if !frame.in_object {
                    return Err(Error::State("EndObject inside array".into()));
                }
                self.w.write_all(b"}")?;
            }
            Event::StartArray => {
                self.write_separator()?;
                self.w.write_all(b"[")?;
                self.stack.push(Frame {
                    in_object: false,
                    first: true,
                    pending_name: false,
                });
            }
            Event::EndArray => {
                let frame = self
                    .stack
                    .pop()
                    .ok_or_else(|| Error::State("EndArray without StartArray".into()))?;
                if frame.in_object {
                    return Err(Error::State("EndArray inside object".into()));
                }
                self.w.write_all(b"]")?;
            }
            Event::Name(name) => {
                let top = self
                    .stack
                    .last_mut()
                    .ok_or_else(|| Error::State("Name at top level".into()))?;
                if !top.in_object {
                    return Err(Error::State("Name inside array".into()));
                }
                if !top.first {
                    self.w.write_all(b",")?;
                }
                top.first = false;
                write_json_string(&mut self.w, name)?;
                self.stack.last_mut().unwrap().pending_name = true;
            }
            Event::Value(s) => {
                self.write_separator()?;
                self.write_scalar(s)?;
            }
        }
        Ok(())
    }

    fn finish(&mut self) -> Result<()> {
        if !self.stack.is_empty() {
            return Err(Error::State(format!(
                "{} unclosed containers",
                self.stack.len()
            )));
        }
        self.w.flush()?;
        Ok(())
    }
}

impl<W: std::io::Write> crate::writer::IntoInner<W> for MinifyWriter<W> {
    fn into_inner(self) -> W {
        self.w
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{Event::*, Scalar};

    fn emit(events: &[Event]) -> String {
        let mut buf = Vec::new();
        {
            let mut w = MinifyWriter::new(&mut buf);
            for e in events {
                w.write_event(e).unwrap();
            }
            w.finish().unwrap();
        }
        String::from_utf8(buf).unwrap()
    }

    #[test]
    fn minify_scalar() {
        assert_eq!(emit(&[Value(Scalar::Number("42".into()))]), "42");
    }

    #[test]
    fn minify_empty_object_and_array() {
        assert_eq!(emit(&[StartObject, EndObject]), "{}");
        assert_eq!(emit(&[StartArray, EndArray]), "[]");
    }

    #[test]
    fn minify_flat_array() {
        assert_eq!(
            emit(&[
                StartArray,
                Value(Scalar::Number("1".into())),
                Value(Scalar::Number("2".into())),
                Value(Scalar::Bool(true)),
                EndArray,
            ]),
            "[1,2,true]"
        );
    }

    #[test]
    fn minify_object() {
        assert_eq!(
            emit(&[
                StartObject,
                Name("a".into()),
                Value(Scalar::Number("1".into())),
                Name("b".into()),
                Value(Scalar::Null),
                EndObject,
            ]),
            r#"{"a":1,"b":null}"#
        );
    }

    #[test]
    fn minify_nested() {
        assert_eq!(
            emit(&[
                StartObject,
                Name("x".into()),
                StartArray,
                StartObject,
                Name("y".into()),
                Value(Scalar::Number("1".into())),
                EndObject,
                EndArray,
                EndObject,
            ]),
            r#"{"x":[{"y":1}]}"#
        );
    }
}
