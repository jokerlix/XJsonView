//! Validation and streaming statistics.

pub mod schema;
pub mod stats;
pub mod syntax;

pub use schema::{SchemaError, SchemaValidator, Violation};
pub use stats::{Stats, StatsCollector, StatsConfig, ValueKind};
pub use syntax::validate_syntax;
