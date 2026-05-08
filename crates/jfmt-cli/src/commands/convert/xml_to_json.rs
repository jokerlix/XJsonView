//! Streaming XML → JSON translator. Implements the @attr / #text mapping
//! with always-array default. --array-rule and --strict land in Task 9.

use crate::cli::ConvertArgs;
use anyhow::{Context, Result};
use jfmt_xml::{EventReader, XmlEvent};
use std::io::{Read, Write};

pub fn translate<R: Read, W: Write>(input: R, mut output: W, _args: &ConvertArgs) -> Result<()> {
    let mut reader = EventReader::new(input);
    let mut writer = JsonEmitter::new(&mut output);

    loop {
        let ev = reader.next_event().context("XML parse")?;
        let Some(ev) = ev else { break };
        match ev {
            XmlEvent::StartTag { name, attrs } => writer.start_element(&name, &attrs)?,
            XmlEvent::EndTag { .. } => writer.end_element()?,
            XmlEvent::Text(t) | XmlEvent::CData(t) => writer.text(&t)?,
            XmlEvent::Decl { .. } | XmlEvent::Comment(_) | XmlEvent::Pi { .. } => {} // dropped per spec §4.1
        }
    }
    writer.finish()?;
    Ok(())
}

/// Streaming emitter. Maintains a stack of elements; each frame
/// remembers whether its `{` has been opened yet, whether a comma is
/// needed before the next field, and the running `#text` buffer.
struct JsonEmitter<W: Write> {
    w: W,
    stack: Vec<Frame>,
    /// True before any output has been written. Used to insert the
    /// enclosing object braces around the document root.
    document_started: bool,
}

struct Frame {
    #[allow(dead_code)]
    name: String,
    /// Comma needed before next field of this element's body.
    needs_comma: bool,
    /// Accumulated `#text` content.
    text_buf: String,
    /// Last child element name we emitted, for "still in same array?" detection.
    last_child_name: Option<String>,
    /// True while we are currently inside an open `[...]` array of children
    /// for `last_child_name`. False between siblings of different names.
    in_child_array: bool,
}

impl<W: Write> JsonEmitter<W> {
    fn new(w: W) -> Self {
        Self {
            w,
            stack: Vec::new(),
            document_started: false,
        }
    }

    fn start_element(&mut self, name: &str, attrs: &[(String, String)]) -> Result<()> {
        // Open the document object on the first element.
        if !self.document_started {
            self.w.write_all(b"{")?;
            self.document_started = true;
        }

        // Always-array: open `[` then `{`.
        let continuing = self
            .stack
            .last()
            .map(|f| f.in_child_array && f.last_child_name.as_deref() == Some(name))
            .unwrap_or(false);

        if !continuing {
            // First occurrence in current run: close any prior open array,
            // then write `,"name":[`.
            if let Some(parent) = self.stack.last_mut() {
                if parent.in_child_array {
                    // Different child name -> close previous array.
                    self.w.write_all(b"]")?;
                    parent.in_child_array = false;
                    parent.last_child_name = None;
                }
                if parent.needs_comma {
                    self.w.write_all(b",")?;
                }
                parent.needs_comma = true;
            }
            write_string(&mut self.w, name)?;
            self.w.write_all(b":[")?;
        } else {
            // Continuing an open array: `,`.
            self.w.write_all(b",")?;
        }

        // Object opens.
        self.w.write_all(b"{")?;
        let mut frame = Frame {
            name: name.to_owned(),
            needs_comma: false,
            text_buf: String::new(),
            last_child_name: None,
            in_child_array: false,
        };
        for (k, v) in attrs {
            if frame.needs_comma {
                self.w.write_all(b",")?;
            }
            write_string(&mut self.w, &format!("@{k}"))?;
            self.w.write_all(b":")?;
            write_string(&mut self.w, v)?;
            frame.needs_comma = true;
        }
        // Mark this element on the parent so siblings know the run is open.
        if let Some(parent) = self.stack.last_mut() {
            parent.last_child_name = Some(name.to_owned());
            parent.in_child_array = true;
        }
        self.stack.push(frame);
        Ok(())
    }

    fn text(&mut self, t: &str) -> Result<()> {
        if let Some(frame) = self.stack.last_mut() {
            frame.text_buf.push_str(t);
        }
        Ok(())
    }

    fn end_element(&mut self) -> Result<()> {
        let mut frame = self.stack.pop().expect("unbalanced");
        // Close any open child array of this element.
        if frame.in_child_array {
            self.w.write_all(b"]")?;
            frame.in_child_array = false;
            frame.last_child_name = None;
        }
        // Flush #text if any non-whitespace content (or any content at all
        // — match xml-js behavior).
        if !frame.text_buf.is_empty() {
            if frame.needs_comma {
                self.w.write_all(b",")?;
            }
            write_string(&mut self.w, "#text")?;
            self.w.write_all(b":")?;
            write_string(&mut self.w, &frame.text_buf)?;
        }
        self.w.write_all(b"}")?;
        Ok(())
    }

    fn finish(&mut self) -> Result<()> {
        // Drain stack (should only fire on malformed input that omits
        // root close — proper XML always lands stack at depth 0 when EOF
        // arrives).
        while !self.stack.is_empty() {
            self.end_element()?;
        }
        if self.document_started {
            // Close the outermost open array `]` and the document `}`.
            self.w.write_all(b"]}")?;
        }
        Ok(())
    }
}

fn write_string<W: Write>(w: &mut W, s: &str) -> Result<()> {
    w.write_all(b"\"")?;
    for c in s.chars() {
        match c {
            '"' => w.write_all(b"\\\"")?,
            '\\' => w.write_all(b"\\\\")?,
            '\n' => w.write_all(b"\\n")?,
            '\r' => w.write_all(b"\\r")?,
            '\t' => w.write_all(b"\\t")?,
            c if (c as u32) < 0x20 => {
                write!(w, "\\u{:04x}", c as u32)?;
            }
            _ => {
                let mut buf = [0u8; 4];
                w.write_all(c.encode_utf8(&mut buf).as_bytes())?;
            }
        }
    }
    w.write_all(b"\"")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::ConvertArgs;

    fn run(xml: &str) -> String {
        let args = ConvertArgs {
            input: None,
            output: None,
            from: None,
            to: None,
            array_rule: None,
            root: None,
            pretty: false,
            indent: None,
            tabs: false,
            xml_decl: false,
            strict: false,
        };
        let mut out = Vec::new();
        translate(xml.as_bytes(), &mut out, &args).unwrap();
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn empty_root() {
        assert_eq!(run("<a/>"), r#"{"a":[{}]}"#);
    }

    #[test]
    fn root_with_attrs() {
        assert_eq!(run(r#"<a x="1" y="2"/>"#), r#"{"a":[{"@x":"1","@y":"2"}]}"#);
    }

    #[test]
    fn root_with_text() {
        assert_eq!(run("<a>hi</a>"), r##"{"a":[{"#text":"hi"}]}"##);
    }

    #[test]
    fn nested_repeated_children() {
        assert_eq!(run("<a><b/><b/></a>"), r#"{"a":[{"b":[{},{}]}]}"#);
    }

    #[test]
    fn mixed_content_concatenates_text() {
        assert_eq!(
            run("<a>before<b/>after</a>"),
            r##"{"a":[{"b":[{}],"#text":"beforeafter"}]}"##
        );
    }

    #[test]
    fn namespace_attribute_preserved() {
        assert_eq!(
            run(r#"<ns:foo xmlns:ns="http://x"/>"#),
            r#"{"ns:foo":[{"@xmlns:ns":"http://x"}]}"#
        );
    }
}
