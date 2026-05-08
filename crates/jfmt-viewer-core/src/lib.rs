//! jfmt-viewer-core: streaming index + session APIs for the jfmt GUI viewer.
//!
//! No UI dependencies. Reused by `apps/jfmt-viewer/src-tauri` via `#[tauri::command]`
//! wrappers.

pub mod error;

pub use error::{Result, ViewerError};
