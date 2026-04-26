use std::io;
use thiserror::Error;

pub type Result<T> = std::result::Result<T, XmlError>;

#[derive(Debug, Error)]
pub enum XmlError {
    #[error("XML I/O error: {0}")]
    Io(#[from] io::Error),

    #[error("XML parse error at line {line}, column {column}: {message}")]
    Parse {
        line: u64,
        column: u64,
        message: String,
    },

    #[error("unexpected end of XML input")]
    UnexpectedEof,

    #[error("XML encoding error: {0}")]
    Encoding(String),

    #[error("invalid XML name: {0}")]
    InvalidName(String),
}
