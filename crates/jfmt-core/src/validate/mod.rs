//! Validation and streaming statistics.

pub mod ndjson;
pub mod stats;
pub mod syntax;

pub use ndjson::{validate_ndjson, LineError, NdjsonOptions, NdjsonReport};
pub use stats::{Stats, StatsCollector, StatsConfig, ValueKind};
pub use syntax::validate_syntax;
