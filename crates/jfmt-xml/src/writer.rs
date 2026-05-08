use crate::{Result, XmlError, XmlEvent};
use std::io::Write;

#[derive(Debug, Clone, Default)]
pub struct XmlPrettyConfig {
    pub indent: u8,
    pub tabs: bool,
    pub xml_decl: bool,
}

pub trait EventWriter {
    fn write_event(&mut self, ev: &XmlEvent) -> Result<()>;
    fn finish(&mut self) -> Result<()>;
}

pub struct XmlWriter<W: Write> {
    writer: W,
    cfg: XmlPrettyConfig,
}

impl<W: Write> XmlWriter<W> {
    pub fn new(writer: W) -> Self {
        Self::with_config(writer, XmlPrettyConfig::default())
    }

    pub fn with_config(writer: W, cfg: XmlPrettyConfig) -> Self {
        Self { writer, cfg }
    }
}

impl<W: Write> EventWriter for XmlWriter<W> {
    fn write_event(&mut self, ev: &XmlEvent) -> Result<()> {
        match ev {
            XmlEvent::StartTag { name, attrs } => {
                validate_name(name)?;
                self.writer.write_all(b"<")?;
                self.writer.write_all(name.as_bytes())?;
                for (k, v) in attrs {
                    validate_name(k)?;
                    self.writer.write_all(b" ")?;
                    self.writer.write_all(k.as_bytes())?;
                    self.writer.write_all(b"=\"")?;
                    write_attr_value(&mut self.writer, v)?;
                    self.writer.write_all(b"\"")?;
                }
                self.writer.write_all(b">")?;
            }
            XmlEvent::EndTag { name } => {
                self.writer.write_all(b"</")?;
                self.writer.write_all(name.as_bytes())?;
                self.writer.write_all(b">")?;
            }
            XmlEvent::Text(t) => {
                write_text(&mut self.writer, t)?;
            }
            // Task 6 fills the rest.
            _ => {}
        }
        let _ = self.cfg.indent;
        let _ = self.cfg.tabs;
        let _ = self.cfg.xml_decl;
        Ok(())
    }

    fn finish(&mut self) -> Result<()> {
        Ok(())
    }
}

fn validate_name(name: &str) -> Result<()> {
    if name.is_empty() {
        return Err(XmlError::InvalidName("empty".into()));
    }
    let mut chars = name.chars();
    let first = chars.next().unwrap();
    if !(first.is_alphabetic() || first == '_') {
        return Err(XmlError::InvalidName(format!(
            "must start with letter or underscore: {name}"
        )));
    }
    for c in chars {
        if !(c.is_alphanumeric() || c == '_' || c == '-' || c == '.' || c == ':') {
            return Err(XmlError::InvalidName(format!(
                "invalid char {c:?} in {name}"
            )));
        }
    }
    Ok(())
}

fn write_text<W: Write>(w: &mut W, s: &str) -> Result<()> {
    for c in s.chars() {
        match c {
            '&' => w.write_all(b"&amp;")?,
            '<' => w.write_all(b"&lt;")?,
            '>' => w.write_all(b"&gt;")?,
            _ => {
                let mut buf = [0u8; 4];
                w.write_all(c.encode_utf8(&mut buf).as_bytes())?;
            }
        }
    }
    Ok(())
}

fn write_attr_value<W: Write>(w: &mut W, s: &str) -> Result<()> {
    for c in s.chars() {
        match c {
            '&' => w.write_all(b"&amp;")?,
            '<' => w.write_all(b"&lt;")?,
            '"' => w.write_all(b"&quot;")?,
            _ => {
                let mut buf = [0u8; 4];
                w.write_all(c.encode_utf8(&mut buf).as_bytes())?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::XmlEvent;

    fn render(events: &[XmlEvent]) -> String {
        let mut buf = Vec::new();
        let mut w = XmlWriter::new(&mut buf);
        for ev in events {
            w.write_event(ev).unwrap();
        }
        w.finish().unwrap();
        String::from_utf8(buf).unwrap()
    }

    #[test]
    fn empty_element() {
        let s = render(&[
            XmlEvent::StartTag {
                name: "a".into(),
                attrs: vec![],
            },
            XmlEvent::EndTag { name: "a".into() },
        ]);
        assert_eq!(s, "<a></a>");
    }

    #[test]
    fn element_with_text() {
        let s = render(&[
            XmlEvent::StartTag {
                name: "a".into(),
                attrs: vec![],
            },
            XmlEvent::Text("hi & bye".into()),
            XmlEvent::EndTag { name: "a".into() },
        ]);
        assert_eq!(s, "<a>hi &amp; bye</a>");
    }

    #[test]
    fn nested() {
        let s = render(&[
            XmlEvent::StartTag {
                name: "a".into(),
                attrs: vec![],
            },
            XmlEvent::StartTag {
                name: "b".into(),
                attrs: vec![],
            },
            XmlEvent::EndTag { name: "b".into() },
            XmlEvent::EndTag { name: "a".into() },
        ]);
        assert_eq!(s, "<a><b></b></a>");
    }
}
