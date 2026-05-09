# jfmt M10 — viewer scalability for large JSON arrays — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `jfmt-viewer` open 1 GB JSON files with up to 10 M containers and return any 200-element child window in < 500 ms.

**Architecture:** Add a CSR (compressed-sparse-row) `parent → children` index alongside `SparseIndex.entries`, rewrite `Session::get_children` to skip past out-of-window child subtrees instead of materializing them, and back the file content with `memmap2::Mmap` + `fd-lock` exclusive lock to keep RAM bounded and prevent external mutation.

**Tech Stack:** Rust 1.85, struson 0.6 (existing `EventReader`), memmap2 0.9 (new), fs4 0.9 (new — provides `FileExt::try_lock_exclusive`).

**Spec:** `docs/superpowers/specs/2026-05-09-jfmt-m10-viewer-perf-design.md`

---

## File map

| File | Change | Responsibility |
|---|---|---|
| `crates/jfmt-viewer-core/src/index.rs` | modify | Add `child_offsets` / `child_ids` fields, `compute_csr()` post-pass, `children_of()` helper, CSR unit tests |
| `crates/jfmt-viewer-core/src/session.rs` | modify | `find_container_child` uses CSR; `get_children` becomes paginated with subtree skip; `Session` holds `Mmap` + `_lock`; `open_with_progress` acquires lock |
| `crates/jfmt-viewer-core/src/error.rs` | modify | Add `FileLocked` variant |
| `crates/jfmt-core/src/parser.rs` | modify | Add `EventReader::from_slice_at` |
| `crates/jfmt-viewer-core/Cargo.toml` | modify | Add `memmap2`, `fd-lock` deps; add `big-tests` feature |
| `Cargo.toml` (workspace) | modify | Add `memmap2` / `fd-lock` to `[workspace.dependencies]` |
| `crates/jfmt-viewer-core/tests/big_array.rs` | create | `big-tests`-gated 50 MB perf regression |

`apps/jfmt-viewer/src-tauri/src/commands.rs` and the React frontend need **no changes** — the IPC contract is unchanged.

---

## Task 1: Add CSR fields and `compute_csr()` post-pass

**Files:**
- Modify: `crates/jfmt-viewer-core/src/index.rs`

**Approach note:** rather than threading per-frame accumulators through both
`build_json` (in `index.rs`) and `build_ndjson` (in `ndjson.rs`), build CSR
in a single O(N) post-pass over the finalized `entries` vector. Two passes
(count, then fill) using a temporary cursor vector. This adds < 5% to index
build time and keeps each builder simple.

- [ ] **Step 1: Add failing test for CSR shape**

Append to `crates/jfmt-viewer-core/src/index.rs` (in the existing `#[cfg(test)] mod tests`):

```rust
    #[test]
    fn csr_child_index_for_small_json() {
        let bytes = fixture("small.json");
        let idx = SparseIndex::build(&bytes, IndexMode::Json).unwrap();

        // child_offsets has entries.len() + 1 entries (CSR sentinel).
        assert_eq!(idx.child_offsets.len(), idx.entries.len() + 1);
        // It is monotonically non-decreasing.
        for w in idx.child_offsets.windows(2) {
            assert!(w[0] <= w[1], "child_offsets not monotonic: {:?}", idx.child_offsets);
        }
        // The total length equals the count of entries that have a parent.
        let with_parent = idx.entries.iter().filter(|e| e.parent.is_some()).count();
        assert_eq!(idx.child_ids.len(), with_parent);

        // Sanity: child_ids of root contain every entry whose parent == ROOT.
        let lo = idx.child_offsets[0] as usize;
        let hi = idx.child_offsets[1] as usize;
        let root_kids: Vec<NodeId> = idx.child_ids[lo..hi].to_vec();
        let expected: Vec<NodeId> = idx.entries.iter().enumerate()
            .filter(|(_, e)| e.parent == Some(NodeId::ROOT))
            .map(|(i, _)| NodeId(i as u64))
            .collect();
        assert_eq!(root_kids, expected);

        // Sanity: child_ids slice for any container is sorted by file_offset.
        for parent_idx in 0..idx.entries.len() {
            let lo = idx.child_offsets[parent_idx] as usize;
            let hi = idx.child_offsets[parent_idx + 1] as usize;
            let kids = &idx.child_ids[lo..hi];
            for w in kids.windows(2) {
                let a = idx.entries[w[0].0 as usize].file_offset;
                let b = idx.entries[w[1].0 as usize].file_offset;
                assert!(a < b, "kids not sorted by file_offset for parent {parent_idx}");
            }
        }
    }

    #[test]
    fn csr_child_index_for_ndjson() {
        let bytes = fixture("ndjson_basic.ndjson");
        let idx = SparseIndex::build(&bytes, IndexMode::Ndjson).unwrap();
        // Synthetic root has one child per non-empty line.
        let lo = idx.child_offsets[0] as usize;
        let hi = idx.child_offsets[1] as usize;
        assert_eq!(hi - lo, idx.entries[0].child_count as usize);
    }
```

- [ ] **Step 2: Run; expect FAIL**

Run: `cargo test -p jfmt-viewer-core csr_child_index`
Expected: compile error — `child_offsets` / `child_ids` don't exist on `SparseIndex` yet.

- [ ] **Step 3: Add CSR fields to `SparseIndex`**

In `crates/jfmt-viewer-core/src/index.rs`, replace the `SparseIndex` struct (around lines 14-20):

```rust
#[derive(Debug)]
pub struct SparseIndex {
    pub entries: Vec<ContainerEntry>,
    /// CSR row pointers: `child_offsets[i] .. child_offsets[i+1]` is the
    /// slice of `child_ids` for parent NodeId(i). Length = entries.len() + 1.
    pub child_offsets: Vec<u32>,
    /// CSR data: NodeIds of every container child, grouped by parent and
    /// sorted by source-file offset within each group.
    pub child_ids: Vec<NodeId>,
    pub root_kind: Option<ContainerKind>,
    pub byte_len: u64,
}
```

- [ ] **Step 4: Add `compute_csr()` helper**

In `crates/jfmt-viewer-core/src/index.rs`, add this free function near the top (after the `IndexMode` enum):

```rust
/// Build CSR `parent → child container ids` arrays from finalized entries.
///
/// Children are emitted in source order (the order they appear in
/// `entries`, which mirrors the order they were encountered during
/// indexing). Returns `(child_offsets, child_ids)`.
fn compute_csr(entries: &[ContainerEntry]) -> (Vec<u32>, Vec<NodeId>) {
    let n = entries.len();
    let mut counts = vec![0u32; n];
    for e in entries {
        if let Some(p) = e.parent {
            counts[p.0 as usize] = counts[p.0 as usize].saturating_add(1);
        }
    }
    let mut child_offsets = Vec::with_capacity(n + 1);
    let mut acc: u32 = 0;
    child_offsets.push(0);
    for c in &counts {
        acc = acc.saturating_add(*c);
        child_offsets.push(acc);
    }
    let mut child_ids = vec![NodeId(0); acc as usize];
    let mut cursor = vec![0u32; n];
    for (i, e) in entries.iter().enumerate() {
        if let Some(p) = e.parent {
            let p_idx = p.0 as usize;
            let pos = child_offsets[p_idx] + cursor[p_idx];
            child_ids[pos as usize] = NodeId(i as u64);
            cursor[p_idx] += 1;
        }
    }
    (child_offsets, child_ids)
}
```

- [ ] **Step 5: Wire `compute_csr` into `build_json`**

In `crates/jfmt-viewer-core/src/index.rs`, replace the final return of `build_json` (around lines 165-169):

```rust
    let (child_offsets, child_ids) = compute_csr(&entries);
    Ok(SparseIndex {
        entries,
        child_offsets,
        child_ids,
        root_kind,
        byte_len: input.len() as u64,
    })
```

- [ ] **Step 6: Wire `compute_csr` into `build_ndjson`**

In `crates/jfmt-viewer-core/src/ndjson.rs`, replace the final return of `build_ndjson` (around lines 72-76):

```rust
    let (child_offsets, child_ids) = crate::index::compute_csr(&entries);
    Ok(SparseIndex {
        entries,
        child_offsets,
        child_ids,
        root_kind: Some(ContainerKind::NdjsonDoc),
        byte_len: input.len() as u64,
    })
```

`compute_csr` is currently `fn` (private). Make it `pub(crate)`:

In `index.rs`, change `fn compute_csr(...)` to `pub(crate) fn compute_csr(...)`.

- [ ] **Step 7: Verify ndjson fixture exists**

Run: `ls crates/jfmt-viewer-core/tests/fixtures/`
Expected: `ndjson_basic.ndjson` is present (used by existing ndjson tests). If not, the `csr_child_index_for_ndjson` test must use an existing ndjson fixture name — check `crates/jfmt-viewer-core/src/ndjson.rs` `#[cfg(test)] mod tests` for what's loaded and substitute.

- [ ] **Step 8: Run; expect PASS**

Run: `cargo test -p jfmt-viewer-core`
Expected: all tests pass, including the two new CSR tests.

- [ ] **Step 9: Run clippy**

Run: `cargo clippy -p jfmt-viewer-core --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 10: Commit**

```bash
git add crates/jfmt-viewer-core/src/index.rs crates/jfmt-viewer-core/src/ndjson.rs
git commit -m "$(cat <<'EOF'
feat(viewer-core): CSR parent→children index in SparseIndex

Adds child_offsets / child_ids built in a single O(N) post-pass over
the finalized entries vector, used in M10 to make find_container_child
O(log K) and to power paginated get_children.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 2: `find_container_child` via CSR binary search

**Files:**
- Modify: `crates/jfmt-viewer-core/src/session.rs`
- Modify: `crates/jfmt-viewer-core/src/index.rs` (add `children_of` helper)

- [ ] **Step 1: Add failing test for `children_of` helper**

Append to `crates/jfmt-viewer-core/src/index.rs` `#[cfg(test)] mod tests`:

```rust
    #[test]
    fn children_of_returns_csr_slice() {
        let bytes = fixture("small.json");
        let idx = SparseIndex::build(&bytes, IndexMode::Json).unwrap();
        let root_kids = idx.children_of(NodeId::ROOT);
        let lo = idx.child_offsets[0] as usize;
        let hi = idx.child_offsets[1] as usize;
        assert_eq!(root_kids, &idx.child_ids[lo..hi]);
    }
```

- [ ] **Step 2: Run; expect FAIL**

Run: `cargo test -p jfmt-viewer-core children_of_returns_csr_slice`
Expected: compile error — method doesn't exist.

- [ ] **Step 3: Add `children_of` on `SparseIndex`**

In `crates/jfmt-viewer-core/src/index.rs`, add an `impl SparseIndex` block (or extend the existing one) below the struct definition:

```rust
impl SparseIndex {
    pub fn children_of(&self, parent: NodeId) -> &[NodeId] {
        let i = parent.0 as usize;
        if i + 1 >= self.child_offsets.len() {
            return &[];
        }
        let lo = self.child_offsets[i] as usize;
        let hi = self.child_offsets[i + 1] as usize;
        &self.child_ids[lo..hi]
    }
}
```

- [ ] **Step 4: Run; expect PASS for new test**

Run: `cargo test -p jfmt-viewer-core children_of_returns_csr_slice`
Expected: PASS.

- [ ] **Step 5: Replace `find_container_child` with CSR binary search**

In `crates/jfmt-viewer-core/src/session.rs` (around lines 282-292), replace:

```rust
    fn find_container_child(&self, parent: NodeId, child_offset: u64) -> Option<NodeId> {
        let kids = self.index.children_of(parent);
        kids.binary_search_by(|id| {
            self.index.entries[id.0 as usize]
                .file_offset
                .cmp(&child_offset)
        })
        .ok()
        .map(|i| kids[i])
    }
```

- [ ] **Step 6: Run full viewer-core test suite**

Run: `cargo test -p jfmt-viewer-core`
Expected: all tests pass. The `find_container_child` change is behavior-preserving; if any test breaks the CSR shape is wrong — go back to Task 1.

- [ ] **Step 7: Run clippy**

Run: `cargo clippy -p jfmt-viewer-core --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 8: Commit**

```bash
git add crates/jfmt-viewer-core/src/index.rs crates/jfmt-viewer-core/src/session.rs
git commit -m "$(cat <<'EOF'
perf(viewer-core): find_container_child via CSR binary search

Replaces the O(N) full-table scan with O(log K) binary search inside
the parent's CSR child slice. Behavior identical; the speedup matters
once get_children stops re-materializing the whole array in M10 task 3.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 3: `EventReader::from_slice_at` + paginated `get_children`

**Files:**
- Modify: `crates/jfmt-core/src/parser.rs`
- Modify: `crates/jfmt-viewer-core/src/session.rs`

- [ ] **Step 1: Add failing test for `from_slice_at`**

Append to `crates/jfmt-core/src/parser.rs` `#[cfg(test)] mod tests` (find the existing test module; if none exists at the bottom of the file, add one):

```rust
    #[test]
    fn from_slice_at_starts_at_offset() {
        let input = br#"[1, 2, {"k": 3}, 4]"#;
        // Position 8 is the '{' of the inner object.
        assert_eq!(input[8], b'{');
        let mut r = EventReader::from_slice_at(input, 8);
        let ev = r.next_event().unwrap();
        assert!(matches!(ev, Some(crate::event::Event::StartObject)));
    }
```

If no `#[cfg(test)] mod tests` exists in `parser.rs`, add at file bottom:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_slice_at_starts_at_offset() {
        let input = br#"[1, 2, {"k": 3}, 4]"#;
        assert_eq!(input[8], b'{');
        let mut r = EventReader::from_slice_at(input, 8);
        let ev = r.next_event().unwrap();
        assert!(matches!(ev, Some(crate::event::Event::StartObject)));
    }
}
```

- [ ] **Step 2: Run; expect FAIL**

Run: `cargo test -p jfmt-core from_slice_at_starts_at_offset`
Expected: compile error — `from_slice_at` does not exist.

- [ ] **Step 3: Implement `from_slice_at`**

In `crates/jfmt-core/src/parser.rs`, after `pub fn new_unlimited` (line 41-52), add a free constructor on the `&[u8]` impl. Since `EventReader` is generic over `R: Read`, we need a separate impl block for byte slices. Add at the end of the existing `impl<R: Read> EventReader<R>` block, OR add a new impl:

```rust
impl<'a> EventReader<&'a [u8]> {
    /// Construct an unlimited-depth reader that begins at byte offset
    /// `start` within `input`. The caller must guarantee `start` is on a
    /// valid token boundary (a `[` / `{` / `]` / `}` / `,` byte, or the
    /// first byte of a value).
    ///
    /// `byte_offset()` on the returned reader reports positions relative
    /// to the sub-slice (`&input[start..]`), matching the existing
    /// `new_unlimited` contract. Callers that need an absolute file
    /// position add `start` themselves.
    pub fn from_slice_at(input: &'a [u8], start: usize) -> Self {
        Self::new_unlimited(&input[start..])
    }
}
```

- [ ] **Step 4: Run parser test; expect PASS**

Run: `cargo test -p jfmt-core from_slice_at_starts_at_offset`
Expected: PASS.

- [ ] **Step 5: Add failing pagination tests in session.rs**

Append to `crates/jfmt-viewer-core/src/session.rs` `#[cfg(test)] mod tests`:

```rust
    fn write_array_fixture(n: usize) -> tempfile::NamedTempFile {
        use std::io::Write;
        let f = tempfile::Builder::new().suffix(".json").tempfile().unwrap();
        let mut w = std::io::BufWriter::new(f.reopen().unwrap());
        write!(w, "[").unwrap();
        for i in 0..n {
            if i > 0 { write!(w, ",").unwrap(); }
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
        assert_eq!(resp.items.len(), 50, "tail clipped to remaining items");
        assert_eq!(resp.items[0].key, "950");
        assert_eq!(resp.items[49].key, "999");
    }

    #[test]
    fn get_children_offset_past_end_empty() {
        let f = write_array_fixture(10);
        let s = Session::open(f.path()).unwrap();
        let resp = s.get_children(NodeId::ROOT, 100, 100).unwrap();
        assert_eq!(resp.total, 10);
        assert_eq!(resp.items.len(), 0);
    }
```

- [ ] **Step 6: Run; expect PASS or FAIL — record which**

Run: `cargo test -p jfmt-viewer-core get_children_paginates`
Expected: head/middle/tail PASS on the old impl (it's correct, just slow); offset-past-end may already PASS too. If all four pass, this test set serves as a **regression harness** for the rewrite. If any fail, fix the test (e.g. fixture path issue) before proceeding.

- [ ] **Step 7: Rewrite `get_children`**

In `crates/jfmt-viewer-core/src/session.rs`, replace the body of `pub fn get_children` (lines 99-204) with the paginated version:

```rust
    pub fn get_children(&self, parent: NodeId, offset: u32, limit: u32) -> Result<GetChildrenResp> {
        let entry = self
            .index
            .entries
            .get(parent.0 as usize)
            .ok_or(ViewerError::InvalidNode)?;

        if entry.kind == ContainerKind::NdjsonDoc && parent == NodeId::ROOT {
            return self.ndjson_root_children(offset, limit);
        }

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

        let csr = self.index.children_of(parent);
        let mut csr_cursor = 0usize;
        let mut child_idx: u32 = 0;
        let mut pending_key: Option<String> = None;
        let mut next_array_index = 0u32;
        let mut items: Vec<ChildSummary> = Vec::with_capacity(limit as usize);

        // Reader starts on the opening bracket; consume it.
        let mut reader = EventReader::from_slice_at(&self.bytes, actual_start);
        let _open = reader.next_event().map_err(|e| ViewerError::Parse {
            pos: actual_start as u64,
            msg: e.to_string(),
        })?;

        let stop_at = offset.saturating_add(limit);

        loop {
            if items.len() == limit as usize {
                break;
            }
            let pos_before = reader.byte_offset();
            let ev = match reader.next_event() {
                Ok(Some(e)) => e,
                Ok(None) => break,
                Err(e) => {
                    return Err(ViewerError::Parse {
                        pos: actual_start as u64 + pos_before,
                        msg: e.to_string(),
                    });
                }
            };
            match ev {
                Event::EndObject | Event::EndArray => break,
                Event::Name(k) => {
                    pending_key = Some(k);
                }
                Event::StartObject | Event::StartArray => {
                    // CSR cursor moves on every container child, in source order.
                    let id = csr[csr_cursor];
                    csr_cursor += 1;
                    let child_entry = &self.index.entries[id.0 as usize];

                    if child_idx >= offset && child_idx < stop_at {
                        let key = self.consume_key(entry.kind, &mut pending_key, &mut next_array_index);
                        let child_kind = if matches!(ev, Event::StartObject) {
                            Kind::Object
                        } else {
                            Kind::Array
                        };
                        items.push(ChildSummary {
                            id: Some(id),
                            key,
                            kind: child_kind,
                            child_count: child_entry.child_count,
                            preview: None,
                        });
                    } else {
                        // Still need to consume the slot so subsequent
                        // array indices / object keys advance correctly.
                        let _ = self.consume_key(entry.kind, &mut pending_key, &mut next_array_index);
                    }

                    // Skip the subtree by re-anchoring the reader past byte_end.
                    let resume_at = child_entry.byte_end as usize;
                    reader = EventReader::from_slice_at(&self.bytes, resume_at);

                    child_idx += 1;
                }
                Event::Value(scalar) => {
                    if child_idx >= offset && child_idx < stop_at {
                        let key = self.consume_key(entry.kind, &mut pending_key, &mut next_array_index);
                        let (kind, preview) = leaf_preview(&scalar);
                        items.push(ChildSummary {
                            id: None,
                            key,
                            kind,
                            child_count: 0,
                            preview: Some(preview),
                        });
                    } else {
                        let _ = self.consume_key(entry.kind, &mut pending_key, &mut next_array_index);
                    }
                    child_idx += 1;
                }
            }
        }

        Ok(GetChildrenResp {
            items,
            total: entry.child_count,
        })
    }
```

Note: this drops the `find_container_child` call site inside `get_children`. The function is still used by other code paths (search-result NodeId resolution etc.) — leave it in place.

- [ ] **Step 8: Run pagination tests; expect PASS**

Run: `cargo test -p jfmt-viewer-core get_children_paginates`
Expected: all four PASS.

- [ ] **Step 9: Run full viewer-core suite**

Run: `cargo test -p jfmt-viewer-core`
Expected: all tests pass. If any existing test breaks:
- A test that asserts `total = items.len()` may break if the test fixture has more children than `limit`. The new `total` is the **true total**, not `items.len()`. Fix the test to assert against the true total.
- A test that asserts a specific child's `child_count` may break if the new code reads from `child_entry.child_count` rather than computing it inline. The values should match — investigate if not.

- [ ] **Step 10: Run clippy**

Run: `cargo clippy -p jfmt-viewer-core --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 11: Commit**

```bash
git add crates/jfmt-core/src/parser.rs crates/jfmt-viewer-core/src/session.rs
git commit -m "$(cat <<'EOF'
perf(viewer-core): paginated get_children with subtree skip

Out-of-window child containers are skipped by re-anchoring the
EventReader at their byte_end instead of parsing the subtree.
Window-internal container ids resolve via CSR (O(1)). The 'total'
field is read from the parent entry's pre-computed child_count.
Adds EventReader::from_slice_at to jfmt-core for the re-anchor step.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 4: Mmap + fd-lock

**Files:**
- Modify: `Cargo.toml` (workspace root)
- Modify: `crates/jfmt-viewer-core/Cargo.toml`
- Modify: `crates/jfmt-viewer-core/src/error.rs`
- Modify: `crates/jfmt-viewer-core/src/session.rs`

- [ ] **Step 1: Add failing test for `FileLocked`**

Append to `crates/jfmt-viewer-core/src/session.rs` `#[cfg(test)] mod tests`:

```rust
    #[test]
    fn second_open_returns_file_locked() {
        let f = write_array_fixture(10);
        let _first = Session::open(f.path()).unwrap();
        let err = Session::open(f.path()).unwrap_err();
        assert!(
            matches!(err, ViewerError::FileLocked(_)),
            "expected FileLocked, got {err:?}"
        );
    }
```

- [ ] **Step 2: Run; expect FAIL**

Run: `cargo test -p jfmt-viewer-core second_open_returns_file_locked`
Expected: compile error — `ViewerError::FileLocked` does not exist.

- [ ] **Step 3: Add `FileLocked` variant**

In `crates/jfmt-viewer-core/src/error.rs`, add to the `ViewerError` enum (after `Io`):

```rust
    #[error("file is in use by another session: {0}")]
    FileLocked(String),
```

Add a unit test for the new variant in the same file's `#[cfg(test)] mod tests`:

```rust
    #[test]
    fn file_locked_displays() {
        let err = ViewerError::FileLocked("foo.json".into());
        assert_eq!(err.to_string(), "file is in use by another session: foo.json");
        let s = serde_json::to_string(&err).unwrap();
        assert!(s.contains("FileLocked"), "got {s}");
    }
```

- [ ] **Step 4: Add workspace deps**

In `Cargo.toml` (workspace root), append to `[workspace.dependencies]`:

```toml
# M10 — viewer scalability for large JSON arrays.
memmap2 = "0.9"
fs4 = "0.9"
```

In `crates/jfmt-viewer-core/Cargo.toml`, append to `[dependencies]`:

```toml
memmap2.workspace = true
fs4.workspace = true
```

- [ ] **Step 5: Switch `Session.bytes` to `Mmap` + locked `File`**

In `crates/jfmt-viewer-core/src/session.rs`, change the `Session` struct (around lines 43-49):

```rust
#[derive(Debug)]
pub struct Session {
    path: PathBuf,
    /// Order matters: `bytes` (the Mmap) drops before `_file`, so the
    /// underlying handle is still alive when the mapping is torn down.
    /// Dropping `_file` releases both the file handle and the OS lock
    /// acquired via `fs4::FileExt::try_lock_exclusive`.
    bytes: memmap2::Mmap,
    _file: std::fs::File,
    index: SparseIndex,
    format: Format,
}
```

Note: in Rust, struct fields drop in declaration order. `bytes` first, then `_file` — correct.

- [ ] **Step 6: Rewrite `Session::open_with_progress`**

In `crates/jfmt-viewer-core/src/session.rs`, replace the body of `pub fn open_with_progress` (lines 56-81):

```rust
    pub fn open_with_progress<P: AsRef<Path>, F: FnMut(u64, u64)>(
        path: P,
        on_progress: F,
    ) -> Result<Self> {
        use fs4::fs_std::FileExt;
        let path = path.as_ref().to_path_buf();
        let file = std::fs::OpenOptions::new()
            .read(true)
            .open(&path)
            .map_err(|e| match e.kind() {
                std::io::ErrorKind::NotFound => ViewerError::NotFound(path.display().to_string()),
                _ => ViewerError::Io(e.to_string()),
            })?;
        // Non-blocking exclusive lock; immediately fails if another holder.
        file.try_lock_exclusive()
            .map_err(|_| ViewerError::FileLocked(path.display().to_string()))?;
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
```

The `fs4::fs_std::FileExt` import path is for fs4 v0.9+; if the import resolves to something different, check `cargo doc -p fs4 --open` or the crate README — older versions used `fs4::FileExt` at the crate root. Adjust the `use` line accordingly.

**Lock release:** `try_lock_exclusive` does NOT auto-release on `File` drop in all platforms. fs4 documents that closing the file descriptor releases the lock on both Unix and Windows. Since `_file` is dropped when `Session` drops, the lock is released — verified by the `second_open_returns_file_locked` test running after the first session is dropped (it isn't dropped in that test, so the second open should fail).

- [ ] **Step 7: Update all `&self.bytes` call sites**

`memmap2::Mmap` derefs to `&[u8]`, so most existing slice arithmetic is unchanged. However, places that do `&self.bytes[..]` to coerce to `&[u8]` work as-is. Verify by compiling:

Run: `cargo build -p jfmt-viewer-core`
Expected: compiles. If a borrow-check error appears, the most likely fix is replacing `&self.bytes` with `&self.bytes[..]` at that site to force the deref to a `&[u8]`.

- [ ] **Step 8: Run viewer-core tests**

Run: `cargo test -p jfmt-viewer-core`
Expected: all tests pass, including the new `second_open_returns_file_locked` and `file_locked_displays`.

- [ ] **Step 9: Surface `FileLocked` from the Tauri command**

In `apps/jfmt-viewer/src-tauri/src/commands.rs::open_file`, the existing match (`session_result`) only special-cases `NotFound`. `FileLocked` will pass through as a `ViewerError` already (the error enum is serialized via `tag`/`content`), so no code change is strictly required — verify by reading the existing pattern:

Run: `grep -n "FileLocked\|InvalidSession\|NotFound" apps/jfmt-viewer/src-tauri/src/commands.rs`
Expected: see how other variants flow through. If the IPC error type round-trips via the existing `Result<_, ViewerError>` path with `#[serde(tag = ...)]`, no change needed. If `commands.rs` does any custom matching, add an arm for `FileLocked` that just propagates the message.

- [ ] **Step 10: Build the workspace**

Run: `cargo build --workspace`
Expected: compiles (this catches mismatched signatures across viewer-core consumers).

- [ ] **Step 11: Run clippy**

Run: `cargo clippy --workspace --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 12: Commit**

```bash
git add Cargo.toml crates/jfmt-viewer-core/Cargo.toml crates/jfmt-viewer-core/src/error.rs crates/jfmt-viewer-core/src/session.rs
git commit -m "$(cat <<'EOF'
feat(viewer-core): mmap-backed Session with exclusive file lock

Session now memory-maps the input file (memmap2) and holds an
exclusive advisory lock (fd-lock). Same-process double-open returns
ViewerError::FileLocked. Avoids buffering large files in user-space
RAM and prevents external mutation from corrupting the open view.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 5: `big-tests`-gated 50 MB perf regression

**Files:**
- Modify: `crates/jfmt-viewer-core/Cargo.toml`
- Create: `crates/jfmt-viewer-core/tests/big_array.rs`

- [ ] **Step 1: Add `big-tests` feature**

In `crates/jfmt-viewer-core/Cargo.toml`, append:

```toml
[features]
big-tests = []
```

- [ ] **Step 2: Create the test file**

Create `crates/jfmt-viewer-core/tests/big_array.rs`:

```rust
//! Performance regression for the M10 viewer-core rewrite. Gated behind
//! `--features big-tests` so default `cargo test` stays fast.
//!
//! Run:  cargo test -p jfmt-viewer-core --features big-tests --test big_array

#![cfg(feature = "big-tests")]

use std::io::Write;
use std::time::Instant;

use jfmt_viewer_core::{NodeId, Session};

const N: usize = 200_000; // ~50 MB of small object records

fn write_big_array() -> tempfile::NamedTempFile {
    let f = tempfile::Builder::new()
        .suffix(".json")
        .tempfile()
        .unwrap();
    let mut w = std::io::BufWriter::new(f.reopen().unwrap());
    write!(w, "[").unwrap();
    for i in 0..N {
        if i > 0 {
            write!(w, ",").unwrap();
        }
        write!(
            w,
            r#"{{"i":{i},"name":"row-{i}","tags":["a","b","c"],"nested":{{"x":{i},"y":{}}}}}"#,
            i * 2
        )
        .unwrap();
    }
    write!(w, "]").unwrap();
    w.flush().unwrap();
    drop(w);
    f
}

#[test]
fn open_and_paginate_under_500ms_per_call() {
    let f = write_big_array();
    let s = Session::open(f.path()).expect("open");

    let t0 = Instant::now();
    let head = s.get_children(NodeId::ROOT, 0, 200).expect("head page");
    let head_ms = t0.elapsed().as_millis();
    assert_eq!(head.total as usize, N);
    assert_eq!(head.items.len(), 200);
    assert!(
        head_ms < 500,
        "head pagination too slow: {head_ms} ms (target < 500)"
    );

    let t1 = Instant::now();
    let tail = s
        .get_children(NodeId::ROOT, (N as u32) - 100, 50)
        .expect("tail page");
    let tail_ms = t1.elapsed().as_millis();
    assert_eq!(tail.items.len(), 50);
    assert_eq!(tail.items[0].key, format!("{}", N - 100));
    assert_eq!(tail.items[49].key, format!("{}", N - 51));
    assert!(
        tail_ms < 500,
        "tail pagination too slow: {tail_ms} ms (target < 500)"
    );
}
```

- [ ] **Step 3: Run with feature on; expect PASS**

Run: `cargo test -p jfmt-viewer-core --features big-tests --test big_array --release`
Expected: PASS. Run with `--release` because perf-sensitive; debug builds are 5–10× slower and may trip the 500 ms budget unfairly.

If it fails the timing assertion: check that Tasks 1–4 are all merged. If it fails on correctness: the rewrite has a key-emission bug — look at `consume_key` ordering in the new `get_children`.

- [ ] **Step 4: Run default test suite (feature off) to confirm gating**

Run: `cargo test -p jfmt-viewer-core`
Expected: PASS, and the `big_array` test is **not run** (no log line for it).

- [ ] **Step 5: Run clippy with the feature on**

Run: `cargo clippy -p jfmt-viewer-core --features big-tests --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add crates/jfmt-viewer-core/Cargo.toml crates/jfmt-viewer-core/tests/big_array.rs
git commit -m "$(cat <<'EOF'
test(viewer-core): big-tests-gated 50MB pagination perf regression

Asserts get_children head + tail pagination on a 200k-element array
each return in < 500 ms (release build). Gated behind the new
big-tests feature so default cargo test stays fast.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Post-milestone: version bump

Once all 5 tasks land and `cargo test --workspace` is green:

- [ ] **Bump workspace version to 0.5.0**

In `Cargo.toml`, change `version = "0.4.0"` to `version = "0.5.0"` under `[workspace.package]`.

In each crate's `Cargo.toml` that pins its own version (e.g. `apps/jfmt-viewer/src-tauri/Cargo.toml`, `crates/jfmt-viewer-core/Cargo.toml`, `apps/jfmt-viewer/src-tauri/tauri.conf.json`, `apps/jfmt-viewer/package.json`), update to `0.5.0`. Use `grep -rn '0\.4\.0' --include='*.toml' --include='*.json'` to find every site.

- [ ] **Tag and commit**

```bash
git add -A
git commit -m "$(cat <<'EOF'
chore: bump version to 0.5.0 (M10 — viewer perf for large arrays)

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
git tag v0.5.0
```

Do NOT push the tag without explicit instruction.
