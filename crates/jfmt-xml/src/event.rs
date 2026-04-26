//! XML event model for jfmt-xml.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum XmlEvent {
    Decl {
        version: String,
        encoding: Option<String>,
        standalone: Option<bool>,
    },
    StartTag {
        name: String,
        attrs: Vec<(String, String)>,
    },
    EndTag {
        name: String,
    },
    Text(String),
    CData(String),
    Comment(String),
    Pi {
        target: String,
        data: String,
    },
}
