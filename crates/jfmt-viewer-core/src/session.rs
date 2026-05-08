//! Open file → owned index + content buffer; child / value / pointer accessors.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::error::{Result, ViewerError};
use crate::index::{IndexMode, SparseIndex};
use crate::ndjson::is_ndjson_path;
use crate::types::{ChildSummary, ContainerKind, Kind, NodeId};
use jfmt_core::event::{Event, Scalar};
use jfmt_core::parser::EventReader;

const LEAF_PREVIEW_BYTES: usize = 256;
const LEAF_PREVIEW_TRUNC_BYTES: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Format {
    Json,
    Ndjson,
}

#[derive(Debug, Serialize)]
pub struct GetChildrenResp {
    pub items: Vec<ChildSummary>,
    pub total: u32,
}

#[derive(Debug)]
pub struct Session {
    path: PathBuf,
    bytes: Vec<u8>,
    index: SparseIndex,
    format: Format,
}

impl Session {
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let bytes = std::fs::read(&path).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => ViewerError::NotFound(path.display().to_string()),
            _ => ViewerError::Io(e.to_string()),
        })?;

        let format = if is_ndjson_path(&path) {
            Format::Ndjson
        } else {
            Format::Json
        };
        let mode = match format {
            Format::Json => IndexMode::Json,
            Format::Ndjson => IndexMode::Ndjson,
        };
        let index = SparseIndex::build(&bytes, mode)?;

        Ok(Self {
            path,
            bytes,
            index,
            format,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn byte_len(&self) -> u64 {
        self.index.byte_len
    }

    pub fn format(&self) -> Format {
        self.format
    }

    pub fn index(&self) -> &SparseIndex {
        &self.index
    }

    pub fn get_children(&self, parent: NodeId, offset: u32, limit: u32) -> Result<GetChildrenResp> {
        let entry = self
            .index
            .entries
            .get(parent.0 as usize)
            .ok_or(ViewerError::InvalidNode)?;

        if entry.kind == ContainerKind::NdjsonDoc && parent == NodeId::ROOT {
            return self.ndjson_root_children(offset, limit);
        }

        // `file_offset` points to the byte after the previous token (struson's
        // position semantics), NOT necessarily the `{` or `[` opening byte.
        // Scan forward from `file_offset` to find the actual opening bracket.
        let scan_start = entry.file_offset as usize;
        let open_byte = match entry.kind {
            ContainerKind::Object => b'{',
            ContainerKind::Array => b'[',
            ContainerKind::NdjsonDoc => b'{', // handled above
        };
        let actual_start = self.bytes[scan_start..]
            .iter()
            .position(|&b| b == open_byte)
            .map(|p| scan_start + p)
            .unwrap_or(scan_start);

        // Re-walk this container's byte range.
        let slice = &self.bytes[actual_start..entry.byte_end as usize];
        let mut reader = EventReader::new_unlimited(slice);
        let mut items: Vec<ChildSummary> = Vec::new();
        let mut depth = 0u32;
        let mut next_index = 0u32;
        let mut pending_key: Option<String> = None;

        loop {
            let local_pos = reader.byte_offset();
            let ev = match reader.next_event() {
                Ok(Some(e)) => e,
                Ok(None) => break,
                Err(e) => {
                    return Err(ViewerError::Parse {
                        pos: actual_start as u64 + local_pos,
                        msg: e.to_string(),
                    });
                }
            };
            match ev {
                Event::StartObject | Event::StartArray => {
                    if depth == 1 {
                        let key = self.consume_key(entry.kind, &mut pending_key, &mut next_index);
                        let child_offset = actual_start as u64 + local_pos;
                        let id = self.find_container_child(parent, child_offset);
                        let child_kind = if matches!(ev, Event::StartObject) {
                            Kind::Object
                        } else {
                            Kind::Array
                        };
                        let count = id
                            .map(|id| self.index.entries[id.0 as usize].child_count)
                            .unwrap_or(0);
                        items.push(ChildSummary {
                            id,
                            key,
                            kind: child_kind,
                            child_count: count,
                            preview: None,
                        });
                    }
                    depth += 1;
                }
                Event::EndObject | Event::EndArray => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        break;
                    }
                }
                Event::Name(k) => {
                    if depth == 1 {
                        pending_key = Some(k);
                    }
                }
                Event::Value(scalar) => {
                    if depth == 1 {
                        let key = self.consume_key(entry.kind, &mut pending_key, &mut next_index);
                        let (kind, preview) = leaf_preview(&scalar);
                        items.push(ChildSummary {
                            id: None,
                            key,
                            kind,
                            child_count: 0,
                            preview: Some(preview),
                        });
                    }
                }
            }
        }

        let total = items.len() as u32;
        let start = offset.min(total) as usize;
        let end = (offset.saturating_add(limit)).min(total) as usize;
        let window = items[start..end].to_vec();
        Ok(GetChildrenResp {
            items: window,
            total,
        })
    }

    fn consume_key(
        &self,
        parent_kind: ContainerKind,
        pending_key: &mut Option<String>,
        next_index: &mut u32,
    ) -> String {
        match parent_kind {
            ContainerKind::Object => pending_key.take().unwrap_or_default(),
            ContainerKind::Array | ContainerKind::NdjsonDoc => {
                let s = next_index.to_string();
                *next_index += 1;
                s
            }
        }
    }

    fn find_container_child(&self, parent: NodeId, child_offset: u64) -> Option<NodeId> {
        // Children of `parent` are NOT guaranteed to be contiguous in the entries
        // array — nested containers of earlier children interleave. Scan all entries
        // that have the given parent and match the file offset.
        for (i, e) in self.index.entries.iter().enumerate() {
            if e.parent == Some(parent) && e.file_offset == child_offset {
                return Some(NodeId(i as u64));
            }
        }
        None
    }

    fn ndjson_root_children(&self, offset: u32, limit: u32) -> Result<GetChildrenResp> {
        let mut items = Vec::new();
        for (i, e) in self.index.entries.iter().enumerate().skip(1) {
            if e.parent == Some(NodeId::ROOT) {
                items.push(ChildSummary {
                    id: Some(NodeId(i as u64)),
                    key: e.key_or_index.as_str().to_string(),
                    kind: Kind::from(e.kind),
                    child_count: e.child_count,
                    preview: None,
                });
            }
        }
        let total = items.len() as u32;
        let start = offset.min(total) as usize;
        let end = (offset.saturating_add(limit)).min(total) as usize;
        let window = items[start..end].to_vec();
        Ok(GetChildrenResp {
            items: window,
            total,
        })
    }
}

fn leaf_preview(scalar: &Scalar) -> (Kind, String) {
    let (kind, raw) = match scalar {
        Scalar::String(s) => (Kind::String, format!("{s:?}")),
        Scalar::Number(n) => (Kind::Number, n.clone()),
        Scalar::Bool(true) => (Kind::Bool, "true".into()),
        Scalar::Bool(false) => (Kind::Bool, "false".into()),
        Scalar::Null => (Kind::Null, "null".into()),
    };
    let preview = if raw.len() <= LEAF_PREVIEW_BYTES {
        raw
    } else {
        // Char-boundary safe truncate.
        let mut take = LEAF_PREVIEW_TRUNC_BYTES.min(raw.len());
        while take > 0 && !raw.is_char_boundary(take) {
            take -= 1;
        }
        let mut s = raw[..take].to_string();
        s.push('…');
        s
    };
    (kind, preview)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Kind;

    fn small_session() -> Session {
        let path = format!("{}/tests/fixtures/small.json", env!("CARGO_MANIFEST_DIR"));
        Session::open(path).unwrap()
    }

    #[test]
    fn open_reports_format_and_size() {
        let s = small_session();
        assert_eq!(s.format(), Format::Json);
        assert!(s.byte_len() > 0);
    }

    #[test]
    fn root_children_are_users_and_meta() {
        let s = small_session();
        let resp = s.get_children(NodeId::ROOT, 0, 100).unwrap();
        assert_eq!(resp.total, 2);
        assert_eq!(resp.items.len(), 2);
        assert_eq!(resp.items[0].key, "users");
        assert_eq!(resp.items[0].kind, Kind::Array);
        assert_eq!(resp.items[1].key, "meta");
        assert_eq!(resp.items[1].kind, Kind::Object);
        assert!(resp.items[0].preview.is_none());
    }

    #[test]
    fn leaves_carry_inline_preview() {
        let s = small_session();
        let meta = s
            .get_children(NodeId::ROOT, 0, 100)
            .unwrap()
            .items
            .into_iter()
            .find(|c| c.key == "meta")
            .unwrap();
        let meta_id = meta.id.unwrap();
        let leaves = s.get_children(meta_id, 0, 100).unwrap();
        let version = leaves.items.iter().find(|c| c.key == "version").unwrap();
        assert_eq!(version.kind, Kind::String);
        assert_eq!(version.preview.as_deref(), Some("\"2.1\""));
    }

    #[test]
    fn pagination_windows() {
        let s = small_session();
        let users = s
            .get_children(NodeId::ROOT, 0, 100)
            .unwrap()
            .items
            .into_iter()
            .find(|c| c.key == "users")
            .unwrap();
        let users_id = users.id.unwrap();

        let page1 = s.get_children(users_id, 0, 1).unwrap();
        assert_eq!(page1.total, 2);
        assert_eq!(page1.items.len(), 1);

        let page2 = s.get_children(users_id, 1, 1).unwrap();
        assert_eq!(page2.items.len(), 1);

        let beyond = s.get_children(users_id, 5, 10).unwrap();
        assert_eq!(beyond.items.len(), 0);
    }

    #[test]
    fn open_missing_file_errors() {
        let err = Session::open("/no/such/file.json").unwrap_err();
        assert!(matches!(
            err,
            crate::ViewerError::NotFound(_) | crate::ViewerError::Io(_)
        ));
    }
}
