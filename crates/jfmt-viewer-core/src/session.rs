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

const DEFAULT_GET_VALUE_CAP: u64 = 4 * 1024 * 1024; // 4 MB — spec §4.2

#[derive(Debug, Serialize)]
pub struct GetValueResp {
    pub json: String,
    pub truncated: bool,
}

#[derive(Debug, Clone, Copy, serde::Deserialize)]
pub struct ExportOptions {
    pub pretty: bool,
}

#[derive(Debug)]
pub struct Session {
    path: PathBuf,
    /// Order matters: `bytes` (the Mmap) drops before `_file`, so the
    /// underlying handle is still alive when the mapping is torn down.
    /// Dropping `_file` releases the OS lock acquired via fs4.
    bytes: memmap2::Mmap,
    _file: std::fs::File,
    index: SparseIndex,
    format: Format,
}

impl Session {
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        Self::open_with_progress(path, |_, _| {})
    }

    pub fn open_with_progress<P: AsRef<Path>, F: FnMut(u64, u64)>(
        path: P,
        on_progress: F,
    ) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let file = std::fs::OpenOptions::new()
            .read(true)
            .open(&path)
            .map_err(|e| match e.kind() {
                std::io::ErrorKind::NotFound => ViewerError::NotFound(path.display().to_string()),
                _ => ViewerError::Io(e.to_string()),
            })?;
        // Shared lock: any number of Session readers may coexist, but
        // external writers (which need exclusive) are blocked. Exclusive
        // lock + mmap together break on Windows (mapped reads hit the
        // locked region with ERROR_LOCK_VIOLATION); shared lock avoids
        // that while still preventing the file from being clobbered.
        // Fully-qualified to disambiguate from the unstable std method.
        fs4::fs_std::FileExt::try_lock_shared(&file)
            .map_err(|_| ViewerError::FileLocked(path.display().to_string()))?;
        // SAFETY: file is held in `Self::_file` for the Session lifetime,
        // so the mapping stays valid. External mutation of the file is
        // prevented by the exclusive lock above.
        let bytes = unsafe {
            memmap2::Mmap::map(&file).map_err(|e| ViewerError::Io(e.to_string()))?
        };

        let format = if is_ndjson_path(&path) {
            Format::Ndjson
        } else {
            Format::Json
        };
        let mode = match format {
            Format::Json => IndexMode::Json,
            Format::Ndjson => IndexMode::Ndjson,
        };
        let index = SparseIndex::build_with_progress(&bytes[..], mode, on_progress)?;
        Ok(Self {
            path,
            bytes,
            _file: file,
            index,
            format,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Memory-mapped file content. Cheap; no I/O.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes[..]
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

        // Walk the parent's byte range with a depth-tracking EventReader.
        // Direct children (depth == 1) are paginated against [offset, stop_at);
        // their CSR slice provides O(1) NodeId lookup for container children.
        let slice = &self.bytes[actual_start..entry.byte_end as usize];
        let mut reader = EventReader::new_unlimited(slice);
        let csr = self.index.children_of(parent);
        let mut csr_cursor = 0usize;
        let mut child_idx: u32 = 0;
        let mut depth = 0u32;
        let mut pending_key: Option<String> = None;
        let mut next_index = 0u32;
        let mut items: Vec<ChildSummary> = Vec::with_capacity(limit.min(64) as usize);
        let stop_at = offset.saturating_add(limit);

        loop {
            if items.len() == limit as usize {
                break;
            }
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
                        let id = csr[csr_cursor];
                        csr_cursor += 1;
                        if child_idx >= offset && child_idx < stop_at {
                            let key = self.consume_key(
                                entry.kind,
                                &mut pending_key,
                                &mut next_index,
                            );
                            let child_kind = if matches!(ev, Event::StartObject) {
                                Kind::Object
                            } else {
                                Kind::Array
                            };
                            items.push(ChildSummary {
                                id: Some(id),
                                key,
                                kind: child_kind,
                                child_count: self.index.entries[id.0 as usize].child_count,
                                preview: None,
                            });
                        } else {
                            let _ = self.consume_key(
                                entry.kind,
                                &mut pending_key,
                                &mut next_index,
                            );
                        }
                        child_idx += 1;
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
                        if child_idx >= offset && child_idx < stop_at {
                            let key = self.consume_key(
                                entry.kind,
                                &mut pending_key,
                                &mut next_index,
                            );
                            let (kind, preview) = leaf_preview(&scalar);
                            items.push(ChildSummary {
                                id: None,
                                key,
                                kind,
                                child_count: 0,
                                preview: Some(preview),
                            });
                        } else {
                            let _ = self.consume_key(
                                entry.kind,
                                &mut pending_key,
                                &mut next_index,
                            );
                        }
                        child_idx += 1;
                    }
                }
            }
        }

        Ok(GetChildrenResp {
            items,
            total: entry.child_count,
        })
    }

    pub fn get_value(&self, node: NodeId, max_bytes: Option<u64>) -> Result<GetValueResp> {
        let cap = max_bytes.unwrap_or(DEFAULT_GET_VALUE_CAP);
        let entry = self
            .index
            .entries
            .get(node.0 as usize)
            .ok_or(ViewerError::InvalidNode)?;

        // Like get_children, `file_offset` may land just past the preceding
        // key/colon rather than on the opening brace. Scan forward to the actual
        // opening container byte before slicing.
        // (For NDJSON synthetic root, file_offset is 0 which is fine.)
        let raw_start = entry.file_offset as usize;
        let open_byte = match entry.kind {
            ContainerKind::Object | ContainerKind::NdjsonDoc => b'{',
            ContainerKind::Array => b'[',
        };
        let actual_start = self.bytes[raw_start..]
            .iter()
            .position(|&b| b == open_byte)
            .map(|p| raw_start + p)
            .unwrap_or(raw_start);

        let raw_end = entry.byte_end as usize;
        let slice = &self.bytes[actual_start..raw_end];

        // Parse with serde_json and re-serialize pretty.
        let value: serde_json::Value =
            serde_json::from_slice(slice).map_err(|e| ViewerError::Parse {
                pos: entry.file_offset,
                msg: e.to_string(),
            })?;
        let pretty = serde_json::to_string_pretty(&value).map_err(|e| ViewerError::Parse {
            pos: entry.file_offset,
            msg: e.to_string(),
        })?;

        if (pretty.len() as u64) <= cap {
            return Ok(GetValueResp {
                json: pretty,
                truncated: false,
            });
        }

        // Char-boundary aware truncation.
        let mut take = (cap as usize).min(pretty.len());
        while take > 0 && !pretty.is_char_boundary(take) {
            take -= 1;
        }
        let prefix = &pretty[..take];
        let total_mb = (pretty.len() as f64) / (1024.0 * 1024.0);
        let trailer = format!(
            "\n... (truncated, {total_mb:.1} MB total — export full subtree feature lands in M9)\n"
        );
        Ok(GetValueResp {
            json: format!("{prefix}{trailer}"),
            truncated: true,
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
}

/// Map an array index to the CSR slot for that element's container.
/// `Ok(Some(slot))` if element at `idx` is a container, `Ok(None)` if it's
/// a scalar leaf or the index is out of range.
fn idx_to_csr_slot(session: &Session, parent: NodeId, idx: usize) -> Result<Option<usize>> {
    let entry = &session.index.entries[parent.0 as usize];
    let csr_len = session.index.children_of(parent).len();
    // Fast path: every direct child is a container (csr == direct child
    // count). Skip the byte-range walk entirely.
    if entry.child_count as usize == csr_len {
        return Ok(if idx < csr_len { Some(idx) } else { None });
    }
    // Slow path: scan events to count scalar gaps before idx.
    let scan_start = entry.file_offset as usize;
    let actual_start = session.bytes[scan_start..]
        .iter()
        .position(|&b| b == b'[')
        .map(|p| scan_start + p)
        .unwrap_or(scan_start);
    let slice = &session.bytes[actual_start..entry.byte_end as usize];
    let mut reader = EventReader::new_unlimited(slice);
    let mut depth = 0u32;
    let mut child_idx = 0usize;
    let mut csr_cursor = 0usize;
    loop {
        let ev = match reader.next_event() {
            Ok(Some(e)) => e,
            _ => break,
        };
        match ev {
            Event::StartObject | Event::StartArray => {
                if depth == 1 {
                    if child_idx == idx {
                        return Ok(Some(csr_cursor));
                    }
                    csr_cursor += 1;
                    child_idx += 1;
                }
                depth += 1;
            }
            Event::EndObject | Event::EndArray => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    break;
                }
            }
            Event::Value(_) => {
                if depth == 1 {
                    if child_idx == idx {
                        return Ok(None); // scalar — no NodeId
                    }
                    child_idx += 1;
                }
            }
            Event::Name(_) => {}
        }
    }
    Ok(None)
}

impl Session {
    /// Resolve a single JSON-pointer-style segment relative to `parent` to
    /// the corresponding container child's NodeId. Returns `None` if the
    /// segment refers to a scalar leaf (which has no NodeId) or the index
    /// is out of range.
    ///
    /// O(1) for arrays (CSR lookup); O(direct_children) for objects (must
    /// re-walk the parent's byte range to map keys to child positions).
    pub fn child_for_segment(&self, parent: NodeId, segment: &str) -> Result<Option<NodeId>> {
        let entry = self
            .index
            .entries
            .get(parent.0 as usize)
            .ok_or(ViewerError::InvalidNode)?;
        let csr = self.index.children_of(parent);
        match entry.kind {
            ContainerKind::Array => {
                let idx: usize = match segment.parse() {
                    Ok(i) => i,
                    Err(_) => return Ok(None),
                };
                // Array element idx might be a scalar (no NodeId) or a
                // container. CSR only holds containers, but they are
                // appended in source order, so we can't just index csr
                // by idx — there may be scalar gaps. Re-walk the array
                // counting children to find which CSR slot corresponds.
                let pos = idx_to_csr_slot(self, parent, idx)?;
                Ok(pos.map(|p| csr[p]))
            }
            ContainerKind::Object | ContainerKind::NdjsonDoc => {
                // Linear scan over container children; objects rarely have
                // huge direct child counts.
                for &id in csr {
                    let e = &self.index.entries[id.0 as usize];
                    if e.key_or_index.as_str() == segment {
                        return Ok(Some(id));
                    }
                }
                Ok(None)
            }
        }
    }

    pub fn get_pointer(&self, node: NodeId) -> Result<String> {
        if node == NodeId::ROOT {
            return Ok(String::new());
        }
        let mut segments: Vec<String> = Vec::new();
        let mut cur = node;
        loop {
            let entry = self
                .index
                .entries
                .get(cur.0 as usize)
                .ok_or(ViewerError::InvalidNode)?;
            match entry.parent {
                Some(p) => {
                    segments.push(entry.key_or_index.as_str().to_string());
                    cur = p;
                }
                None => break,
            }
        }
        segments.reverse();
        let refs: Vec<&str> = segments.iter().map(|s| s.as_str()).collect();
        Ok(crate::pointer::encode_pointer(&refs))
    }

    pub fn export_subtree(
        &self,
        node: NodeId,
        target: &std::path::Path,
        options: ExportOptions,
    ) -> Result<u64> {
        let entry = self
            .index
            .entries
            .get(node.0 as usize)
            .ok_or(ViewerError::InvalidNode)?;

        // Forward-scan past the recorded file_offset to find the actual
        // opening `{` or `[`. (Same workaround as get_children / get_value:
        // EventReader's byte_offset returns the position after the token
        // it just consumed, so file_offset can land on a colon/whitespace.)
        let raw_start = entry.file_offset as usize;
        let raw_end = entry.byte_end as usize;
        let bytes = &self.bytes[raw_start..raw_end];
        let skip = bytes
            .iter()
            .position(|&b| b == b'{' || b == b'[')
            .unwrap_or(0);
        let slice = &bytes[skip..];

        let value: serde_json::Value =
            serde_json::from_slice(slice).map_err(|e| ViewerError::Parse {
                pos: entry.file_offset,
                msg: e.to_string(),
            })?;
        let serialized = if options.pretty {
            serde_json::to_vec_pretty(&value)
        } else {
            serde_json::to_vec(&value)
        }
        .map_err(|e| ViewerError::Parse {
            pos: entry.file_offset,
            msg: e.to_string(),
        })?;

        std::fs::write(target, &serialized).map_err(|e| ViewerError::Io(e.to_string()))?;
        Ok(serialized.len() as u64)
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

    /// Copy the shared `small.json` fixture into a unique tempfile and open a
    /// Session on it. The exclusive lock on the source fixture would otherwise
    /// serialize all parallel tests; per-test tempfiles avoid that.
    fn small_session() -> Session {
        let src = format!("{}/tests/fixtures/small.json", env!("CARGO_MANIFEST_DIR"));
        let bytes = std::fs::read(&src).expect(&src);
        let tmp = tempfile::Builder::new()
            .suffix(".json")
            .tempfile()
            .unwrap();
        std::fs::write(tmp.path(), &bytes).unwrap();
        let (_file, path) = tmp.keep().unwrap();
        Session::open(path).unwrap()
    }

    fn write_array_fixture(n: usize) -> tempfile::NamedTempFile {
        use std::io::Write;
        let f = tempfile::Builder::new().suffix(".json").tempfile().unwrap();
        let mut w = std::io::BufWriter::new(f.reopen().unwrap());
        write!(w, "[").unwrap();
        for i in 0..n {
            if i > 0 {
                write!(w, ",").unwrap();
            }
            write!(w, "{{\"i\":{i}}}").unwrap();
        }
        write!(w, "]").unwrap();
        w.flush().unwrap();
        drop(w);
        f
    }

    #[test]
    fn get_children_paginates_head() {
        let f = write_array_fixture(1000);
        let s = Session::open(f.path()).unwrap();
        let resp = s.get_children(NodeId::ROOT, 0, 100).unwrap();
        assert_eq!(resp.total, 1000);
        assert_eq!(resp.items.len(), 100);
        assert_eq!(resp.items[0].key, "0");
        assert_eq!(resp.items[99].key, "99");
    }

    #[test]
    fn get_children_paginates_middle() {
        let f = write_array_fixture(1000);
        let s = Session::open(f.path()).unwrap();
        let resp = s.get_children(NodeId::ROOT, 500, 100).unwrap();
        assert_eq!(resp.total, 1000);
        assert_eq!(resp.items.len(), 100);
        assert_eq!(resp.items[0].key, "500");
        assert_eq!(resp.items[99].key, "599");
    }

    #[test]
    fn get_children_paginates_tail() {
        let f = write_array_fixture(1000);
        let s = Session::open(f.path()).unwrap();
        let resp = s.get_children(NodeId::ROOT, 950, 100).unwrap();
        assert_eq!(resp.total, 1000);
        assert_eq!(resp.items.len(), 50);
        assert_eq!(resp.items[0].key, "950");
        assert_eq!(resp.items[49].key, "999");
    }

    #[test]
    fn multiple_readers_coexist() {
        // Shared lock allows any number of Session readers on the same file.
        let f = write_array_fixture(10);
        let _a = Session::open(f.path()).unwrap();
        let _b = Session::open(f.path()).unwrap();
        let _c = Session::open(f.path()).unwrap();
    }

    #[test]
    fn get_children_offset_past_end_empty() {
        let f = write_array_fixture(10);
        let s = Session::open(f.path()).unwrap();
        let resp = s.get_children(NodeId::ROOT, 100, 100).unwrap();
        assert_eq!(resp.total, 10);
        assert_eq!(resp.items.len(), 0);
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

    #[test]
    fn get_value_pretty_prints_root() {
        let s = small_session();
        let v = s.get_value(NodeId::ROOT, None).unwrap();
        assert!(!v.truncated);
        assert!(v.json.starts_with("{\n"));
        assert!(v.json.contains("\"users\""));
        assert!(v.json.contains("  \"meta\""));
    }

    #[test]
    fn get_value_truncates_when_over_cap() {
        let s = small_session();
        let v = s.get_value(NodeId::ROOT, Some(40)).unwrap();
        assert!(v.truncated);
        assert!(v.json.contains("(truncated"));
    }

    #[test]
    fn get_value_invalid_node() {
        let s = small_session();
        let err = s.get_value(NodeId(9999), None).unwrap_err();
        assert!(matches!(err, crate::ViewerError::InvalidNode));
    }

    #[test]
    fn root_pointer_is_empty() {
        let s = small_session();
        assert_eq!(s.get_pointer(NodeId::ROOT).unwrap(), "");
    }

    #[test]
    fn nested_pointer_uses_keys_and_indexes() {
        let s = small_session();
        let users = s
            .get_children(NodeId::ROOT, 0, 100)
            .unwrap()
            .items
            .into_iter()
            .find(|c| c.key == "users")
            .unwrap();
        let users_id = users.id.unwrap();
        let user1 = s
            .get_children(users_id, 0, 100)
            .unwrap()
            .items
            .into_iter()
            .find(|c| c.key == "1")
            .unwrap();
        let user1_id = user1.id.unwrap();
        assert_eq!(s.get_pointer(user1_id).unwrap(), "/users/1");
    }

    #[test]
    fn pointer_invalid_node() {
        let s = small_session();
        let err = s.get_pointer(NodeId(9999)).unwrap_err();
        assert!(matches!(err, crate::ViewerError::InvalidNode));
    }

    #[test]
    fn export_subtree_root_pretty_matches_get_value() {
        let s = small_session();
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let bytes = s
            .export_subtree(NodeId::ROOT, tmp.path(), ExportOptions { pretty: true })
            .unwrap();
        assert!(bytes > 0);
        let written = std::fs::read_to_string(tmp.path()).unwrap();
        let via_get_value = s.get_value(NodeId::ROOT, None).unwrap().json;
        assert_eq!(written, via_get_value);
    }

    #[test]
    fn export_subtree_compact_is_smaller_than_pretty() {
        let s = small_session();
        let pretty_tmp = tempfile::NamedTempFile::new().unwrap();
        let compact_tmp = tempfile::NamedTempFile::new().unwrap();
        let pretty_bytes = s
            .export_subtree(
                NodeId::ROOT,
                pretty_tmp.path(),
                ExportOptions { pretty: true },
            )
            .unwrap();
        let compact_bytes = s
            .export_subtree(
                NodeId::ROOT,
                compact_tmp.path(),
                ExportOptions { pretty: false },
            )
            .unwrap();
        assert!(compact_bytes < pretty_bytes);
    }

    #[test]
    fn export_subtree_invalid_node() {
        let s = small_session();
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let err = s
            .export_subtree(NodeId(9999), tmp.path(), ExportOptions { pretty: true })
            .unwrap_err();
        assert!(matches!(err, crate::ViewerError::InvalidNode));
    }
}
