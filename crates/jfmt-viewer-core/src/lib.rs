//! jfmt-viewer-core: streaming index + session APIs for the jfmt GUI viewer.
//!
//! No UI dependencies. Reused by `apps/jfmt-viewer/src-tauri` via `#[tauri::command]`
//! wrappers.

pub mod error;
pub mod index;
pub mod ndjson;
pub mod pointer;
pub mod types;

pub mod session;

pub use error::{Result, ViewerError};
pub use index::{IndexMode, SparseIndex};
pub use ndjson::is_ndjson_path;
pub use session::{Format, GetChildrenResp, Session};
pub use types::{ChildSummary, ContainerEntry, ContainerKind, KeyRef, Kind, NodeId};
