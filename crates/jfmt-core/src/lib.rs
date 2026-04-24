//! Streaming JSON parser and writer.
//!
//! Zero I/O assumptions — all entry points accept `impl Read` / `impl Write`.
//! Memory usage is O(nesting depth), not O(file size).

pub mod error;
pub mod escape;
pub mod event;
pub mod ndjson;
pub mod parser;
pub mod transcode;
pub mod validate;
pub mod writer;

pub use error::{Error, Result};
pub use event::{Event, Scalar};
pub use ndjson::{run_ndjson_pipeline, LineError, NdjsonPipelineOptions, PipelineReport};
pub use parser::EventReader;
pub use transcode::transcode;
pub use validate::{validate_syntax, Stats, StatsCollector, StatsConfig};
pub use writer::{EventWriter, MinifyWriter, PrettyConfig, PrettyWriter};
