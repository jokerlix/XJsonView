use serde::Serialize;
use thiserror::Error;

pub type Result<T> = std::result::Result<T, ViewerError>;

#[derive(Debug, Serialize, Error)]
#[serde(tag = "type", content = "data")]
pub enum ViewerError {
    #[error("file not found: {0}")]
    NotFound(String),

    #[error("session not found")]
    InvalidSession,

    #[error("node out of range")]
    InvalidNode,

    #[error("indexing in progress")]
    NotReady,

    #[error("parse error at byte {pos}: {msg}")]
    Parse { pos: u64, msg: String },

    #[error("invalid query: {0}")]
    InvalidQuery(String),

    #[error("io: {0}")]
    Io(String),

    #[error("file is in use by another session: {0}")]
    FileLocked(String),
}

impl From<std::io::Error> for ViewerError {
    fn from(e: std::io::Error) -> Self {
        ViewerError::Io(e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_messages() {
        assert_eq!(
            ViewerError::NotFound("foo.json".into()).to_string(),
            "file not found: foo.json"
        );
        assert_eq!(ViewerError::InvalidSession.to_string(), "session not found");
        assert_eq!(ViewerError::InvalidNode.to_string(), "node out of range");
        assert_eq!(ViewerError::NotReady.to_string(), "indexing in progress");
        assert_eq!(
            ViewerError::Parse {
                pos: 42,
                msg: "bad".into()
            }
            .to_string(),
            "parse error at byte 42: bad"
        );
        assert_eq!(
            ViewerError::Io("disk full".into()).to_string(),
            "io: disk full"
        );
    }

    #[test]
    fn serializes_to_tagged_json() {
        let err = ViewerError::Parse {
            pos: 7,
            msg: "oops".into(),
        };
        let s = serde_json::to_string(&err).unwrap();
        assert!(s.contains("\"Parse\""), "got {s}");
        assert!(s.contains("\"pos\":7"), "got {s}");
    }

    #[test]
    fn from_io_error() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "x");
        let v: ViewerError = io_err.into();
        assert!(matches!(v, ViewerError::Io(_)));
    }

    #[test]
    fn file_locked_displays() {
        let err = ViewerError::FileLocked("foo.json".into());
        assert_eq!(err.to_string(), "file is in use by another session: foo.json");
        let s = serde_json::to_string(&err).unwrap();
        assert!(s.contains("FileLocked"), "got {s}");
    }

    #[test]
    fn invalid_query_displays_with_message() {
        let err = ViewerError::InvalidQuery("unbalanced ( in pattern".into());
        assert_eq!(err.to_string(), "invalid query: unbalanced ( in pattern");
        let s = serde_json::to_string(&err).unwrap();
        assert!(s.contains("InvalidQuery"), "got {s}");
    }
}
