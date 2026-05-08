//! Single-pass sparse index over a JSON file. Records every container
//! (object / array) once; leaves are not indexed.

use crate::error::{Result, ViewerError};
use crate::types::{ContainerEntry, ContainerKind, KeyRef, NodeId};
use jfmt_core::event::Event;
use jfmt_core::parser::EventReader;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndexMode {
    Json,
    Ndjson,
}

#[derive(Debug)]
pub struct SparseIndex {
    pub entries: Vec<ContainerEntry>,
    pub root_kind: Option<ContainerKind>,
    pub byte_len: u64,
}

impl SparseIndex {
    pub fn build(input: &[u8], mode: IndexMode) -> Result<Self> {
        Self::build_with_progress(input, mode, |_, _| {})
    }

    pub fn build_with_progress<F: FnMut(u64, u64)>(
        input: &[u8],
        mode: IndexMode,
        on_progress: F,
    ) -> Result<Self> {
        match mode {
            IndexMode::Json => build_json(input, on_progress),
            IndexMode::Ndjson => crate::ndjson::build_ndjson(input, on_progress),
        }
    }
}

struct Frame {
    node: NodeId,
    next_array_index: u32,
    pending_key: Option<String>,
    kind: ContainerKind,
}

fn build_json<F: FnMut(u64, u64)>(input: &[u8], mut on_progress: F) -> Result<SparseIndex> {
    let mut reader = EventReader::new_unlimited(input);
    let mut entries: Vec<ContainerEntry> = Vec::new();
    let mut stack: Vec<Frame> = Vec::new();
    let mut root_kind: Option<ContainerKind> = None;

    loop {
        let pos_before = reader.byte_offset();
        let ev = match reader.next_event() {
            Ok(Some(e)) => e,
            Ok(None) => break,
            Err(e) => {
                return Err(ViewerError::Parse {
                    pos: pos_before,
                    msg: e.to_string(),
                });
            }
        };

        match ev {
            Event::StartObject | Event::StartArray => {
                let kind = match ev {
                    Event::StartObject => ContainerKind::Object,
                    _ => ContainerKind::Array,
                };

                let (parent, key_or_index) = if let Some(top) = stack.last_mut() {
                    let key = match top.kind {
                        ContainerKind::Object => {
                            top.pending_key.take().ok_or(ViewerError::Parse {
                                pos: pos_before,
                                msg: "object child without preceding key".into(),
                            })?
                        }
                        ContainerKind::Array => {
                            let i = top.next_array_index;
                            top.next_array_index += 1;
                            i.to_string()
                        }
                        ContainerKind::NdjsonDoc => {
                            unreachable!("ndjson handled in build_ndjson")
                        }
                    };
                    (Some(top.node), KeyRef::from_str(&key))
                } else {
                    if root_kind.is_some() {
                        return Err(ViewerError::Parse {
                            pos: pos_before,
                            msg: "second top-level value (use NDJSON mode)".into(),
                        });
                    }
                    root_kind = Some(kind);
                    (None, KeyRef::from_str(""))
                };

                let id = NodeId(entries.len() as u64);
                let entry = ContainerEntry {
                    file_offset: pos_before,
                    byte_end: 0, // patched on close
                    parent,
                    key_or_index,
                    kind,
                    child_count: 0,
                    first_child: None,
                };

                if let Some(p) = parent {
                    let parent_entry = &mut entries[p.0 as usize];
                    if parent_entry.first_child.is_none() {
                        parent_entry.first_child = Some(id);
                    }
                    parent_entry.child_count += 1;
                }

                entries.push(entry);
                stack.push(Frame {
                    node: id,
                    next_array_index: 0,
                    pending_key: None,
                    kind,
                });
            }

            Event::EndObject | Event::EndArray => {
                let frame = stack.pop().ok_or(ViewerError::Parse {
                    pos: pos_before,
                    msg: "unmatched close".into(),
                })?;
                let end = reader.byte_offset();
                entries[frame.node.0 as usize].byte_end = end;
            }

            Event::Name(k) => {
                let top = stack.last_mut().ok_or(ViewerError::Parse {
                    pos: pos_before,
                    msg: "name outside object".into(),
                })?;
                top.pending_key = Some(k);
            }

            Event::Value(_scalar) => {
                if let Some(top) = stack.last_mut() {
                    if matches!(top.kind, ContainerKind::Array) {
                        top.next_array_index += 1;
                    } else {
                        top.pending_key = None;
                    }
                    entries[top.node.0 as usize].child_count += 1;
                }
                // top-level scalar: root_kind stays None
            }
        }
        if entries.len() % 1024 == 0 {
            on_progress(reader.byte_offset(), input.len() as u64);
        }
    }

    on_progress(input.len() as u64, input.len() as u64);

    Ok(SparseIndex {
        entries,
        root_kind,
        byte_len: input.len() as u64,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> Vec<u8> {
        let path = format!("{}/tests/fixtures/{}", env!("CARGO_MANIFEST_DIR"), name);
        std::fs::read(&path).expect(&path)
    }

    #[test]
    fn small_json_index_shape() {
        let bytes = fixture("small.json");
        let idx = SparseIndex::build(&bytes, IndexMode::Json).unwrap();

        // Containers: root object, "users" array, users[0] obj, users[1] obj,
        // "meta" object, "meta.tags" array. = 6 containers.
        assert_eq!(idx.entries.len(), 6, "got {idx:#?}");

        let root = &idx.entries[0];
        assert_eq!(root.kind, ContainerKind::Object);
        assert_eq!(root.parent, None);
        assert_eq!(root.child_count, 2);
        assert_eq!(root.first_child, Some(NodeId(1)));

        let users_arr = idx
            .entries
            .iter()
            .find(|e| e.parent == Some(NodeId::ROOT) && e.key_or_index.as_str() == "users")
            .expect("users container");
        assert_eq!(users_arr.kind, ContainerKind::Array);
        assert_eq!(users_arr.child_count, 2);
    }

    #[test]
    fn deep_json_no_stack_overflow() {
        let bytes = fixture("deep.json");
        let idx = SparseIndex::build(&bytes, IndexMode::Json).unwrap();
        assert_eq!(idx.entries.len(), 500);
        assert_eq!(idx.entries[0].parent, None);
        for i in 1..500 {
            assert_eq!(idx.entries[i].parent, Some(NodeId(i as u64 - 1)));
        }
    }

    #[test]
    fn root_scalar_yields_synthetic_root() {
        let idx = SparseIndex::build(b"42", IndexMode::Json).unwrap();
        assert!(idx.entries.is_empty());
        assert!(idx.root_kind.is_none());
    }

    #[test]
    fn build_with_progress_invokes_callback() {
        let bytes = fixture("small.json");
        let mut calls: Vec<(u64, u64)> = Vec::new();
        let _ = SparseIndex::build_with_progress(&bytes, IndexMode::Json, |done, total| {
            calls.push((done, total));
        })
        .unwrap();
        assert!(!calls.is_empty(), "expected at least one progress call");
        let last = calls.last().unwrap();
        assert_eq!(last.1, bytes.len() as u64, "total should be input length");
        assert!(last.0 <= last.1);
    }
}
