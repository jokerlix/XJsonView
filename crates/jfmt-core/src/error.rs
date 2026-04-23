use std::io;
use thiserror::Error;

/// Errors produced by the jfmt-core streaming pipeline.
#[derive(Debug, Error)]
pub enum Error {
    /// Lower-level I/O failure (reader/writer).
    #[error("io error: {0}")]
    Io(#[from] io::Error),

    /// The input bytes are not valid JSON.
    #[error("syntax error at byte {offset}: {message}")]
    Syntax { offset: u64, message: String },

    /// The parser/writer was called in an unexpected state (internal bug).
    #[error("invalid state: {0}")]
    State(String),
}

/// Convenience alias.
pub type Result<T> = std::result::Result<T, Error>;
