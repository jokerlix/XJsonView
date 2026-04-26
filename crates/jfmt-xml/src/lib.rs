//! Streaming XML SAX-style reader and writer for jfmt.
//!
//! Built on `quick-xml`. Mirrors the shape of `jfmt-core`:
//! [`EventReader`] / [`EventWriter`] / [`XmlWriter`].

pub mod error;
pub mod event;
pub mod reader;
pub mod writer;

pub use error::{Result, XmlError};
pub use event::XmlEvent;
pub use reader::EventReader;
pub use writer::{EventWriter, XmlPrettyConfig, XmlWriter};
