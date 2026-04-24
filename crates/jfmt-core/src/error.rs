use std::io;
use thiserror::Error;

/// Errors produced by the jfmt-core streaming pipeline.
#[derive(Debug, Error)]
pub enum Error {
    /// Lower-level I/O failure (reader/writer).
    #[error("io error: {0}")]
    Io(#[from] io::Error),

    /// The input bytes are not valid JSON.
    #[error("syntax error at {}: {message}", format_location(.offset, .line, .column))]
    Syntax {
        offset: u64,
        line: Option<u64>,
        column: Option<u64>,
        message: String,
    },

    /// The parser/writer was called in an unexpected state (internal bug).
    #[error("invalid state: {0}")]
    State(String),
}

fn format_location(offset: &u64, line: &Option<u64>, column: &Option<u64>) -> String {
    match (line, column) {
        (Some(l), Some(c)) => format!("line {l} column {c} (byte {offset})"),
        _ => format!("byte {offset}"),
    }
}

/// Convenience alias.
pub type Result<T> = std::result::Result<T, Error>;
