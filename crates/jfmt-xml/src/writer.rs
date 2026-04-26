use crate::{Result, XmlEvent};
use std::io::Write;

pub struct XmlPrettyConfig {
    pub indent: u8,
    pub tabs: bool,
    pub xml_decl: bool,
}

impl Default for XmlPrettyConfig {
    fn default() -> Self {
        Self {
            indent: 2,
            tabs: false,
            xml_decl: false,
        }
    }
}

pub trait EventWriter {
    fn write_event(&mut self, ev: &XmlEvent) -> Result<()>;
    fn finish(&mut self) -> Result<()>;
}

pub struct XmlWriter<W: Write> {
    _writer: W,
}

impl<W: Write> XmlWriter<W> {
    pub fn new(writer: W) -> Self {
        Self { _writer: writer }
    }

    pub fn with_config(writer: W, _cfg: XmlPrettyConfig) -> Self {
        Self { _writer: writer }
    }
}
