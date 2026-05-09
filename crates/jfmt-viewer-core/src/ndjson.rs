//! NDJSON-mode indexing. Each non-blank line becomes one direct child of a
//! synthetic NdjsonDoc root.

use std::path::Path;

use crate::error::{Result, ViewerError};
use crate::index::SparseIndex;
use crate::types::{ContainerEntry, ContainerKind, KeyRef, NodeId};
use jfmt_core::event::Event;
use jfmt_core::parser::EventReader;

/// True when the path's lowercased extension is `ndjson` or `jsonl`.
pub fn is_ndjson_path<P: AsRef<Path>>(p: P) -> bool {
    p.as_ref()
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| s.eq_ignore_ascii_case("ndjson") || s.eq_ignore_ascii_case("jsonl"))
        .unwrap_or(false)
}

pub(crate) fn build_ndjson<F: FnMut(u64, u64)>(
    input: &[u8],
    mut on_progress: F,
) -> Result<SparseIndex> {
    // Synthetic root entry at index 0.
    let mut entries = vec![ContainerEntry {
        file_offset: 0,
        byte_end: input.len() as u64,
        parent: None,
        key_or_index: KeyRef::from_str(""),
        kind: ContainerKind::NdjsonDoc,
        child_count: 0,
        first_child: None,
    }];

    let mut line_no: u32 = 0;
    let mut start: usize = 0;
    while start < input.len() {
        let end = memchr::memchr(b'\n', &input[start..])
            .map(|i| start + i)
            .unwrap_or(input.len());
        let line = &input[start..end];
        let trimmed_start = line.iter().take_while(|b| b.is_ascii_whitespace()).count();
        let trimmed = &line[trimmed_start..];
        if !trimmed.is_empty() {
            let line_offset = (start + trimmed_start) as u64;
            let before_count = entries.len();
            index_one_line(
                &input[start + trimmed_start..end],
                line_offset,
                line_no,
                &mut entries,
            )?;
            // If the line produced no container, count it as a leaf doc.
            if entries.len() == before_count {
                entries[0].child_count += 1;
            }
        }
        line_no += 1;
        start = end + 1;
        if line_no % 256 == 0 {
            on_progress(start as u64, input.len() as u64);
        }
    }

    if entries.len() > 1 {
        entries[0].first_child = Some(NodeId(1));
    }

    on_progress(input.len() as u64, input.len() as u64);

    let (child_offsets, child_ids) = crate::index::compute_csr(&entries);
    Ok(SparseIndex {
        entries,
        child_offsets,
        child_ids,
        root_kind: Some(ContainerKind::NdjsonDoc),
        byte_len: input.len() as u64,
    })
}

/// Frame mirrors `index::Frame` but tracked locally per line so that nested
/// containers within an NDJSON line are indexed too.
struct Frame {
    node: NodeId,
    next_array_index: u32,
    pending_key: Option<String>,
    kind: ContainerKind,
}

/// Index one NDJSON line, attaching its top-level container (if any) as a
/// child of the synthetic NDJSON root and indexing all nested containers
/// the same way `build_json` does for JSON files.
fn index_one_line(
    line: &[u8],
    line_offset: u64,
    line_no: u32,
    entries: &mut Vec<ContainerEntry>,
) -> Result<()> {
    let mut reader = EventReader::new_unlimited(line);
    let mut stack: Vec<Frame> = Vec::new();

    loop {
        let pos_before = reader.byte_offset();
        let ev = match reader.next_event() {
            Ok(Some(e)) => e,
            Ok(None) => break,
            Err(e) => {
                return Err(ViewerError::Parse {
                    pos: line_offset + pos_before,
                    msg: format!("ndjson line {line_no}: {e}"),
                });
            }
        };

        match ev {
            Event::StartObject | Event::StartArray => {
                let kind = if matches!(ev, Event::StartObject) {
                    ContainerKind::Object
                } else {
                    ContainerKind::Array
                };

                let (parent, key) = if let Some(top) = stack.last_mut() {
                    let key = match top.kind {
                        ContainerKind::Object => {
                            top.pending_key.take().ok_or(ViewerError::Parse {
                                pos: line_offset + pos_before,
                                msg: format!(
                                    "ndjson line {line_no}: object child without preceding key"
                                ),
                            })?
                        }
                        ContainerKind::Array => {
                            let i = top.next_array_index;
                            top.next_array_index += 1;
                            i.to_string()
                        }
                        ContainerKind::NdjsonDoc => unreachable!(),
                    };
                    (Some(top.node), KeyRef::from_str(&key))
                } else {
                    // Top of the line — parent is the synthetic NDJSON root.
                    (Some(NodeId::ROOT), KeyRef::from_str(&line_no.to_string()))
                };

                let id = NodeId(entries.len() as u64);
                entries.push(ContainerEntry {
                    file_offset: line_offset + pos_before,
                    byte_end: 0,
                    parent,
                    key_or_index: key,
                    kind,
                    child_count: 0,
                    first_child: None,
                });

                if let Some(p) = parent {
                    let parent_entry = &mut entries[p.0 as usize];
                    if parent_entry.first_child.is_none() {
                        parent_entry.first_child = Some(id);
                    }
                    parent_entry.child_count += 1;
                }

                stack.push(Frame {
                    node: id,
                    next_array_index: 0,
                    pending_key: None,
                    kind,
                });
            }

            Event::EndObject | Event::EndArray => {
                let frame = stack.pop().ok_or(ViewerError::Parse {
                    pos: line_offset + pos_before,
                    msg: format!("ndjson line {line_no}: unmatched close"),
                })?;
                let end = line_offset + reader.byte_offset();
                entries[frame.node.0 as usize].byte_end = end;
            }

            Event::Name(k) => {
                let top = stack.last_mut().ok_or(ViewerError::Parse {
                    pos: line_offset + pos_before,
                    msg: format!("ndjson line {line_no}: name outside object"),
                })?;
                top.pending_key = Some(k);
            }

            Event::Value(_) => {
                if let Some(top) = stack.last_mut() {
                    if matches!(top.kind, ContainerKind::Array) {
                        top.next_array_index += 1;
                    } else {
                        top.pending_key = None;
                    }
                    entries[top.node.0 as usize].child_count += 1;
                }
                // Top-level scalar in a line: build_ndjson handles that case.
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::{IndexMode, SparseIndex};

    fn fixture() -> Vec<u8> {
        let path = format!(
            "{}/tests/fixtures/ndjson.ndjson",
            env!("CARGO_MANIFEST_DIR")
        );
        std::fs::read(&path).expect(&path)
    }

    #[test]
    fn synthetic_root_with_per_line_children() {
        let bytes = fixture();
        let idx = SparseIndex::build(&bytes, IndexMode::Ndjson).unwrap();

        // Root + 3 object docs + 1 array-of-3 inside doc 3 = 5 containers.
        assert_eq!(idx.entries.len(), 5);
        let root = &idx.entries[0];
        assert_eq!(root.kind, ContainerKind::NdjsonDoc);
        assert_eq!(root.parent, None);
        // 4 non-blank lines: three objects + the `42` leaf.
        assert_eq!(root.child_count, 4);
    }

    #[test]
    fn detects_ndjson_by_extension() {
        assert!(is_ndjson_path("foo.ndjson"));
        assert!(is_ndjson_path("foo.jsonl"));
        assert!(is_ndjson_path("FOO.NDJSON"));
        assert!(!is_ndjson_path("foo.json"));
        assert!(!is_ndjson_path("foo"));
    }

    #[test]
    fn fifty_k_lines_index_perf() {
        let path = format!(
            "{}/tests/fixtures/large-ndjson.ndjson",
            env!("CARGO_MANIFEST_DIR")
        );
        let bytes = std::fs::read(&path).expect(&path);
        let start = std::time::Instant::now();
        let idx = SparseIndex::build(&bytes, IndexMode::Ndjson).unwrap();
        let elapsed = start.elapsed();
        assert_eq!(idx.entries[0].child_count, 50_000);
        assert!(
            elapsed < std::time::Duration::from_secs(5),
            "indexed in {elapsed:?}"
        );
    }
}
