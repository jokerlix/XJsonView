//! Validation and streaming statistics.

pub mod stats;
pub mod syntax;

pub use stats::{Stats, StatsCollector, StatsConfig, ValueKind};
pub use syntax::validate_syntax;
