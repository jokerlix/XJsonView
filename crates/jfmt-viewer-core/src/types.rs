//! Public IPC and index types. Field shapes match spec §4.1 and §3.3.

use serde::{Deserialize, Serialize};
use smallvec::SmallVec;

/// Index into the session's container array. `0` is always the root.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct NodeId(pub u64);

impl NodeId {
    pub const ROOT: NodeId = NodeId(0);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Object,
    Array,
    String,
    Number,
    Bool,
    Null,
    NdjsonDoc,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContainerKind {
    Object,
    Array,
    NdjsonDoc,
}

impl From<ContainerKind> for Kind {
    fn from(c: ContainerKind) -> Self {
        match c {
            ContainerKind::Object => Kind::Object,
            ContainerKind::Array => Kind::Array,
            ContainerKind::NdjsonDoc => Kind::NdjsonDoc,
        }
    }
}

/// Inline-stored key bytes. Keys ≤ 16 bytes live on the stack.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyRef(SmallVec<[u8; 16]>);

impl KeyRef {
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Self {
        Self(SmallVec::from_slice(s.as_bytes()))
    }

    pub fn as_str(&self) -> &str {
        std::str::from_utf8(&self.0).expect("KeyRef invariant: UTF-8")
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

/// One entry per container (object / array / NDJSON synthetic doc).
/// Leaves are NOT indexed; they are re-parsed on demand from the parent's
/// `file_offset`.
#[derive(Debug, Clone)]
pub struct ContainerEntry {
    pub file_offset: u64,
    pub byte_end: u64,
    pub parent: Option<NodeId>,
    pub key_or_index: KeyRef,
    pub kind: ContainerKind,
    pub child_count: u32,
    pub first_child: Option<NodeId>,
}

/// IPC payload per child of a container.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChildSummary {
    /// Container child → its `NodeId`; leaf → `None`.
    pub id: Option<NodeId>,
    pub key: String,
    pub kind: Kind,
    pub child_count: u32,
    /// Only set for leaves. Full value if ≤ 256 bytes; else first 200 bytes + "…".
    pub preview: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_serializes_to_lowercase() {
        let s = serde_json::to_string(&Kind::Object).unwrap();
        assert_eq!(s, "\"object\"");
        let s = serde_json::to_string(&Kind::NdjsonDoc).unwrap();
        assert_eq!(s, "\"ndjson_doc\"");
    }

    #[test]
    fn child_summary_round_trip() {
        let c = ChildSummary {
            id: Some(NodeId(7)),
            key: "name".into(),
            kind: Kind::String,
            child_count: 0,
            preview: Some("\"Alice\"".into()),
        };
        let s = serde_json::to_string(&c).unwrap();
        let back: ChildSummary = serde_json::from_str(&s).unwrap();
        assert_eq!(back.key, "name");
        assert_eq!(back.kind, Kind::String);
        assert_eq!(back.preview.as_deref(), Some("\"Alice\""));
    }

    #[test]
    fn key_ref_inline_capacity() {
        let small = KeyRef::from_str("hi");
        assert_eq!(small.as_str(), "hi");
        let big = KeyRef::from_str("a-very-long-key-that-exceeds-inline-buffer");
        assert_eq!(big.as_str(), "a-very-long-key-that-exceeds-inline-buffer");
    }

    #[test]
    fn node_id_root() {
        assert_eq!(NodeId::ROOT, NodeId(0));
    }
}
