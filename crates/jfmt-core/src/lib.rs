//! Streaming JSON parser and writer.
//!
//! Zero I/O assumptions — all entry points accept `impl Read` / `impl Write`.
//! Memory usage is O(nesting depth), not O(file size).

pub mod error;
pub mod escape;
pub mod event;
pub mod parser;

pub use error::{Error, Result};
pub use event::{Event, Scalar};
pub use parser::EventReader;
