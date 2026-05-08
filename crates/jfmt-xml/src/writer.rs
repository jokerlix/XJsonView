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
    /// Stack depth used for indentation. Each StartTag pushes; EndTag pops.
    depth: usize,
    /// True after the StartTag of an element with no children yet.
    /// We use this to write `<a></a>` on the same line vs `<a>\n  child\n</a>`.
    just_opened: bool,
    /// True if the current element's body contains only text/cdata (no
    /// nested elements). Suppresses indentation around text content.
    text_only: bool,
    /// Set after the first event so we know whether to inject `<?xml?>`.
    decl_emitted: bool,
}

impl<W: Write> XmlWriter<W> {
    pub fn new(writer: W) -> Self {
        Self::with_config(writer, XmlPrettyConfig::default())
    }

    pub fn with_config(writer: W, cfg: XmlPrettyConfig) -> Self {
        Self {
            writer,
            cfg,
            depth: 0,
            just_opened: false,
            text_only: false,
            decl_emitted: false,
        }
    }

    fn maybe_emit_auto_decl(&mut self) -> Result<()> {
        if self.cfg.xml_decl && !self.decl_emitted {
            self.writer
                .write_all(br#"<?xml version="1.0" encoding="UTF-8"?>"#)?;
            self.decl_emitted = true;
        }
        Ok(())
    }

    fn write_indent(&mut self) -> Result<()> {
        if self.cfg.indent == 0 && !self.cfg.tabs {
            return Ok(());
        }
        self.writer.write_all(b"\n")?;
        let unit: &[u8] = if self.cfg.tabs { b"\t" } else { b" " };
        let count = if self.cfg.tabs {
            self.depth
        } else {
            self.cfg.indent as usize * self.depth
        };
        for _ in 0..count {
            self.writer.write_all(unit)?;
        }
        Ok(())
    }
}

impl<W: Write> EventWriter for XmlWriter<W> {
    fn write_event(&mut self, ev: &XmlEvent) -> Result<()> {
        match ev {
            XmlEvent::Decl {
                version,
                encoding,
                standalone,
            } => {
                self.writer.write_all(b"<?xml version=\"")?;
                self.writer.write_all(version.as_bytes())?;
                self.writer.write_all(b"\"")?;
                if let Some(enc) = encoding {
                    self.writer.write_all(b" encoding=\"")?;
                    self.writer.write_all(enc.as_bytes())?;
                    self.writer.write_all(b"\"")?;
                }
                if let Some(sa) = standalone {
                    self.writer.write_all(b" standalone=\"")?;
                    self.writer.write_all(if *sa { b"yes" } else { b"no" })?;
                    self.writer.write_all(b"\"")?;
                }
                self.writer.write_all(b"?>")?;
                self.decl_emitted = true;
            }
            XmlEvent::StartTag { name, attrs } => {
                validate_name(name)?;
                self.maybe_emit_auto_decl()?;
                if self.depth > 0 && !self.text_only {
                    self.write_indent()?;
                }
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
                self.depth += 1;
                self.just_opened = true;
                self.text_only = false;
            }
            XmlEvent::EndTag { name } => {
                self.depth = self.depth.saturating_sub(1);
                if !self.just_opened && !self.text_only {
                    self.write_indent()?;
                }
                self.writer.write_all(b"</")?;
                self.writer.write_all(name.as_bytes())?;
                self.writer.write_all(b">")?;
                self.just_opened = false;
                self.text_only = false;
            }
            XmlEvent::Text(t) => {
                self.text_only = true;
                self.just_opened = false;
                write_text(&mut self.writer, t)?;
            }
            XmlEvent::CData(t) => {
                self.text_only = true;
                self.just_opened = false;
                self.writer.write_all(b"<![CDATA[")?;
                self.writer.write_all(t.as_bytes())?;
                self.writer.write_all(b"]]>")?;
            }
            XmlEvent::Comment(t) => {
                self.maybe_emit_auto_decl()?;
                self.writer.write_all(b"<!--")?;
                self.writer.write_all(t.as_bytes())?;
                self.writer.write_all(b"-->")?;
            }
            XmlEvent::Pi { target, data } => {
                self.maybe_emit_auto_decl()?;
                self.writer.write_all(b"<?")?;
                self.writer.write_all(target.as_bytes())?;
                if !data.is_empty() {
                    self.writer.write_all(b" ")?;
                    self.writer.write_all(data.as_bytes())?;
                }
                self.writer.write_all(b"?>")?;
            }
        }
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

    #[test]
    fn cdata_emits_section() {
        let s = render(&[
            XmlEvent::StartTag {
                name: "a".into(),
                attrs: vec![],
            },
            XmlEvent::CData("raw <stuff>".into()),
            XmlEvent::EndTag { name: "a".into() },
        ]);
        assert_eq!(s, "<a><![CDATA[raw <stuff>]]></a>");
    }

    #[test]
    fn decl_and_comment_and_pi() {
        let s = render(&[
            XmlEvent::Decl {
                version: "1.0".into(),
                encoding: Some("UTF-8".into()),
                standalone: None,
            },
            XmlEvent::Comment(" hello ".into()),
            XmlEvent::Pi {
                target: "stylesheet".into(),
                data: r#"href="a.xsl""#.into(),
            },
            XmlEvent::StartTag {
                name: "a".into(),
                attrs: vec![],
            },
            XmlEvent::EndTag { name: "a".into() },
        ]);
        assert_eq!(
            s,
            r#"<?xml version="1.0" encoding="UTF-8"?><!-- hello --><?stylesheet href="a.xsl"?><a></a>"#
        );
    }

    #[test]
    fn pretty_indent() {
        let mut buf = Vec::new();
        let cfg = XmlPrettyConfig {
            indent: 2,
            tabs: false,
            xml_decl: false,
        };
        let mut w = XmlWriter::with_config(&mut buf, cfg);
        for ev in [
            XmlEvent::StartTag {
                name: "a".into(),
                attrs: vec![],
            },
            XmlEvent::StartTag {
                name: "b".into(),
                attrs: vec![],
            },
            XmlEvent::Text("v".into()),
            XmlEvent::EndTag { name: "b".into() },
            XmlEvent::EndTag { name: "a".into() },
        ] {
            w.write_event(&ev).unwrap();
        }
        w.finish().unwrap();
        assert_eq!(String::from_utf8(buf).unwrap(), "<a>\n  <b>v</b>\n</a>");
    }

    #[test]
    fn auto_xml_decl_when_configured() {
        let mut buf = Vec::new();
        let cfg = XmlPrettyConfig {
            indent: 0,
            tabs: false,
            xml_decl: true,
        };
        let mut w = XmlWriter::with_config(&mut buf, cfg);
        w.write_event(&XmlEvent::StartTag {
            name: "a".into(),
            attrs: vec![],
        })
        .unwrap();
        w.write_event(&XmlEvent::EndTag { name: "a".into() })
            .unwrap();
        w.finish().unwrap();
        assert_eq!(
            String::from_utf8(buf).unwrap(),
            r#"<?xml version="1.0" encoding="UTF-8"?><a></a>"#
        );
    }
}
