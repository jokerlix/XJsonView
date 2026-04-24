//! Pretty-printed output with configurable indentation.

use crate::escape::write_json_string;
use crate::event::{Event, Scalar};
use crate::writer::EventWriter;
use crate::{Error, Result};
use std::io::Write;

/// Configuration for [`PrettyWriter`].
#[derive(Debug, Clone, Copy)]
pub struct PrettyConfig {
    /// Number of spaces per indent level. Default 2.
    pub indent: u8,
    /// Use tabs instead of spaces. When true, `indent` is ignored.
    pub use_tabs: bool,
    /// Line ending to emit. Default `\n`.
    pub newline: &'static str,
}

impl Default for PrettyConfig {
    fn default() -> Self {
        Self {
            indent: 2,
            use_tabs: false,
            newline: "\n",
        }
    }
}

/// Pretty JSON writer.
pub struct PrettyWriter<W: Write> {
    w: W,
    cfg: PrettyConfig,
    stack: Vec<Frame>,
    /// Reusable indent byte string (rebuilt on push/pop).
    indent_buf: Vec<u8>,
}

struct Frame {
    in_object: bool,
    first: bool,
    pending_name: bool,
    /// `true` if this container is empty so far — affects whether we emit a
    /// newline before the closing brace.
    empty: bool,
}

impl<W: Write> PrettyWriter<W> {
    pub fn new(w: W) -> Self {
        Self::with_config(w, PrettyConfig::default())
    }

    pub fn with_config(w: W, cfg: PrettyConfig) -> Self {
        Self {
            w,
            cfg,
            stack: Vec::new(),
            indent_buf: Vec::new(),
        }
    }

    fn push_indent(&mut self) {
        if self.cfg.use_tabs {
            self.indent_buf.push(b'\t');
        } else {
            for _ in 0..self.cfg.indent {
                self.indent_buf.push(b' ');
            }
        }
    }

    fn pop_indent(&mut self) {
        let n = if self.cfg.use_tabs {
            1
        } else {
            self.cfg.indent as usize
        };
        let new_len = self.indent_buf.len().saturating_sub(n);
        self.indent_buf.truncate(new_len);
    }

    fn write_newline_and_indent(&mut self) -> Result<()> {
        self.w.write_all(self.cfg.newline.as_bytes())?;
        self.w.write_all(&self.indent_buf)?;
        Ok(())
    }

    /// Called before writing a *value* (scalar or container start), NOT a Name.
    fn before_child(&mut self) -> Result<()> {
        if let Some(top) = self.stack.last_mut() {
            if top.pending_name {
                // Between name and value in an object: `": "`.
                self.w.write_all(b": ")?;
                top.pending_name = false;
                return Ok(());
            }
            if !top.first {
                self.w.write_all(b",")?;
            }
            top.first = false;
            top.empty = false;
        }
        // Indent before each item (but not before the top-level first value).
        if !self.stack.is_empty() {
            self.write_newline_and_indent()?;
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

impl<W: Write> EventWriter for PrettyWriter<W> {
    fn write_event(&mut self, event: &Event) -> Result<()> {
        match event {
            Event::StartObject => {
                self.before_child()?;
                self.w.write_all(b"{")?;
                self.stack.push(Frame {
                    in_object: true,
                    first: true,
                    pending_name: false,
                    empty: true,
                });
                self.push_indent();
            }
            Event::EndObject => {
                let frame = self
                    .stack
                    .pop()
                    .ok_or_else(|| Error::State("EndObject without StartObject".into()))?;
                if !frame.in_object {
                    return Err(Error::State("EndObject inside array".into()));
                }
                self.pop_indent();
                if !frame.empty {
                    self.write_newline_and_indent()?;
                }
                self.w.write_all(b"}")?;
            }
            Event::StartArray => {
                self.before_child()?;
                self.w.write_all(b"[")?;
                self.stack.push(Frame {
                    in_object: false,
                    first: true,
                    pending_name: false,
                    empty: true,
                });
                self.push_indent();
            }
            Event::EndArray => {
                let frame = self
                    .stack
                    .pop()
                    .ok_or_else(|| Error::State("EndArray without StartArray".into()))?;
                if frame.in_object {
                    return Err(Error::State("EndArray inside object".into()));
                }
                self.pop_indent();
                if !frame.empty {
                    self.write_newline_and_indent()?;
                }
                self.w.write_all(b"]")?;
            }
            Event::Name(name) => {
                {
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
                    top.empty = false;
                }
                self.write_newline_and_indent()?;
                write_json_string(&mut self.w, name)?;
                self.stack.last_mut().unwrap().pending_name = true;
            }
            Event::Value(s) => {
                self.before_child()?;
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
        // Trailing newline for POSIX friendliness.
        self.w.write_all(self.cfg.newline.as_bytes())?;
        self.w.flush()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{Event::*, Scalar};

    fn emit(events: &[Event]) -> String {
        emit_cfg(events, PrettyConfig::default())
    }

    fn emit_cfg(events: &[Event], cfg: PrettyConfig) -> String {
        let mut buf = Vec::new();
        {
            let mut w = PrettyWriter::with_config(&mut buf, cfg);
            for e in events {
                w.write_event(e).unwrap();
            }
            w.finish().unwrap();
        }
        String::from_utf8(buf).unwrap()
    }

    #[test]
    fn pretty_scalar() {
        assert_eq!(emit(&[Value(Scalar::Number("42".into()))]), "42\n");
    }

    #[test]
    fn pretty_empty_containers_stay_on_one_line() {
        assert_eq!(emit(&[StartObject, EndObject]), "{}\n");
        assert_eq!(emit(&[StartArray, EndArray]), "[]\n");
    }

    #[test]
    fn pretty_flat_array_indent_2() {
        assert_eq!(
            emit(&[
                StartArray,
                Value(Scalar::Number("1".into())),
                Value(Scalar::Number("2".into())),
                EndArray,
            ]),
            "[\n  1,\n  2\n]\n"
        );
    }

    #[test]
    fn pretty_object_indent_4() {
        let cfg = PrettyConfig {
            indent: 4,
            ..Default::default()
        };
        assert_eq!(
            emit_cfg(
                &[
                    StartObject,
                    Name("a".into()),
                    Value(Scalar::Number("1".into())),
                    Name("b".into()),
                    Value(Scalar::Null),
                    EndObject,
                ],
                cfg
            ),
            "{\n    \"a\": 1,\n    \"b\": null\n}\n"
        );
    }

    #[test]
    fn pretty_nested() {
        let s = emit(&[
            StartObject,
            Name("x".into()),
            StartArray,
            StartObject,
            Name("y".into()),
            Value(Scalar::Number("1".into())),
            EndObject,
            EndArray,
            EndObject,
        ]);
        assert_eq!(s, "{\n  \"x\": [\n    {\n      \"y\": 1\n    }\n  ]\n}\n");
    }

    #[test]
    fn pretty_tabs() {
        let cfg = PrettyConfig {
            use_tabs: true,
            ..Default::default()
        };
        let s = emit_cfg(
            &[StartArray, Value(Scalar::Number("1".into())), EndArray],
            cfg,
        );
        assert_eq!(s, "[\n\t1\n]\n");
    }
}
