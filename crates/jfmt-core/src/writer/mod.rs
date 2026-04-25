//! Event sinks. Writers consume a stream of [`crate::Event`] and produce
//! JSON text on an underlying `Write`.

pub mod minify;
pub mod pretty;

pub use minify::MinifyWriter;
pub use pretty::{PrettyConfig, PrettyWriter};

use crate::event::Event;
use crate::Result;

/// Common interface for JSON event sinks.
pub trait EventWriter {
    /// Consume one event.
    fn write_event(&mut self, event: &Event) -> Result<()>;

    /// Flush underlying buffered state, if any.
    fn finish(&mut self) -> Result<()>;
}

/// Optional capability for tests / introspection: yield the
/// underlying writer, consuming the wrapper.
pub trait IntoInner<T> {
    fn into_inner(self) -> T;
}
