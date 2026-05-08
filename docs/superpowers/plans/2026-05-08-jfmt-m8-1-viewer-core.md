# jfmt M8.1 — Viewer Core + Minimal UI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the streaming, in-memory sparse index and the seven Tauri IPC commands that power the GUI viewer; ship a minimum Tauri shell that proves the IPC contract end-to-end on small files. No virtual scroll, no preview pane, no search UI in M8.1 — those land in M8.2.

**Architecture:** New `jfmt-viewer-core` Rust crate (no UI deps) owns the file → index → child / value / pointer / search pipeline. New `apps/jfmt-viewer/` directory holds the Tauri 2 + React + Vite app; its `src-tauri/` crate is a workspace member that wraps `jfmt-viewer-core` in `#[tauri::command]` functions. Search backend is implemented with cancelation but is not surfaced in the UI yet. CLI gets a `jfmt view <file>` placeholder subcommand whose binary discovery + spawn logic lands in M8.2.

**Tech Stack:** Rust 1.75+, `jfmt-core` `EventReader` for streaming JSON parse, `memchr` for substring search, `smallvec` for inline keys, `serde_json` for pretty-print, `proptest` + `criterion` for testing/bench. Tauri 2.x, React 18, TypeScript 5, Vite 5, pnpm 9.

**Spec:** `docs/superpowers/specs/2026-05-08-jfmt-m8-viewer-skeleton-design.md`

**Predecessor:** M7 shipped at v0.2.0 (commit `ac05171`).

**Out of scope for M8.1 (lands in M8.2):** virtual scroll, preview pane, JSON pointer copy UI, NDJSON top-level virtualized list, search UI (toolbar + hit list), `jfmt view` real binary discovery, Tauri build/distribution, E2E suite, v0.3.0 release.

---

## Task 1: Workspace scaffolding & dependency freeze

**Files:**
- Modify: `Cargo.toml` (root)
- Create: `crates/jfmt-viewer-core/Cargo.toml`
- Create: `crates/jfmt-viewer-core/src/lib.rs`
- Create: `apps/.gitkeep`

- [ ] **Step 1: Add `jfmt-viewer-core` to workspace members and pin new deps**

Edit root `Cargo.toml`. In `[workspace] members` add `crates/jfmt-viewer-core` (alphabetical order with existing entries). In `[workspace.dependencies]` add:

```toml
memchr = "2"
smallvec = { version = "1", features = ["serde"] }
```

(`serde_json`, `thiserror`, `tokio`, `proptest` are already workspace deps from earlier milestones — reuse without adding.)

- [ ] **Step 2: Create the new crate's manifest**

Create `crates/jfmt-viewer-core/Cargo.toml`:

```toml
[package]
name = "jfmt-viewer-core"
version = "0.0.1"
edition = "2021"
license = "MIT OR Apache-2.0"
description = "Streaming index + IPC data model for the jfmt GUI viewer."
repository.workspace = true

[dependencies]
jfmt-core = { path = "../jfmt-core" }
serde = { workspace = true, features = ["derive"] }
serde_json = { workspace = true }
thiserror = { workspace = true }
memchr.workspace = true
smallvec.workspace = true

[dev-dependencies]
proptest = { workspace = true }
tempfile = { workspace = true }

[lints]
workspace = true
```

- [ ] **Step 3: Create the lib root**

Create `crates/jfmt-viewer-core/src/lib.rs`:

```rust
//! jfmt-viewer-core: streaming index + session APIs for the jfmt GUI viewer.
//!
//! No UI dependencies. Reused by `apps/jfmt-viewer/src-tauri` via `#[tauri::command]`
//! wrappers.

pub mod error;

pub use error::{Result, ViewerError};
```

- [ ] **Step 4: Hold a placeholder for the apps directory**

Create empty file `apps/.gitkeep`. (Cargo workspace has no opinion about this directory until M8.1 Task 12 adds the Tauri crate; the placeholder keeps the dir tracked.)

- [ ] **Step 5: Verify the workspace builds**

Run: `cargo build --workspace 2>&1 | tail -5`
Expected: `Compiling jfmt-viewer-core v0.0.1 ...` followed by `Finished dev`. The lib only re-exports `error` (next task creates it). Build will fail because `error` module does not exist yet — **this is the failing-test step for the scaffold; proceed to Task 2 to make it compile.**

If build error is anything other than `unresolved module 'error'`, stop and diagnose.

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml crates/jfmt-viewer-core apps/.gitkeep
git commit -m "$(cat <<'EOF'
chore(viewer): scaffold jfmt-viewer-core crate + apps/ root

M8.1 starts. New crate is empty pending error module + types in
following tasks. Adds memchr and smallvec to workspace deps.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 2: `ViewerError` + `Result` alias

**Files:**
- Create: `crates/jfmt-viewer-core/src/error.rs`
- Test: same file (`#[cfg(test)] mod tests`)

- [ ] **Step 1: Write failing test for error display + serde**

Create `crates/jfmt-viewer-core/src/error.rs` with only the test module first:

```rust
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
            ViewerError::Parse { pos: 42, msg: "bad".into() }.to_string(),
            "parse error at byte 42: bad"
        );
        assert_eq!(ViewerError::Io("disk full".into()).to_string(), "io: disk full");
    }

    #[test]
    fn serializes_to_tagged_json() {
        let err = ViewerError::Parse { pos: 7, msg: "oops".into() };
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
}
```

- [ ] **Step 2: Run tests; expect compile failure**

Run: `cargo test -p jfmt-viewer-core 2>&1 | tail -10`
Expected: `error[E0432]: unresolved import 'super::*'` or `cannot find ... in this scope`.

- [ ] **Step 3: Implement `ViewerError`**

Prepend to `crates/jfmt-viewer-core/src/error.rs` (above the test module):

```rust
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

    #[error("io: {0}")]
    Io(String),
}

impl From<std::io::Error> for ViewerError {
    fn from(e: std::io::Error) -> Self {
        ViewerError::Io(e.to_string())
    }
}
```

- [ ] **Step 4: Run tests; expect PASS**

Run: `cargo test -p jfmt-viewer-core 2>&1 | tail -8`
Expected: `test result: ok. 3 passed`.

- [ ] **Step 5: Run clippy**

Run: `cargo clippy -p jfmt-viewer-core --all-targets -- -D warnings 2>&1 | tail -3`
Expected: `Finished dev`.

- [ ] **Step 6: Commit**

```bash
git add crates/jfmt-viewer-core/src/error.rs
git commit -m "$(cat <<'EOF'
feat(viewer-core): ViewerError enum + Result alias

Six variants matching the IPC contract from spec §4.3. Tagged-enum
serde representation so the frontend can pattern-match `error.type`.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 3: JSON Pointer encoding (RFC 6901)

**Files:**
- Create: `crates/jfmt-viewer-core/src/pointer.rs`
- Modify: `crates/jfmt-viewer-core/src/lib.rs` (add `pub mod pointer;`)

- [ ] **Step 1: Write failing tests**

Create `crates/jfmt-viewer-core/src/pointer.rs`:

```rust
//! RFC 6901 JSON Pointer encode helpers.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_path_is_root() {
        assert_eq!(encode_pointer(&[]), "");
    }

    #[test]
    fn single_segment() {
        assert_eq!(encode_pointer(&["foo"]), "/foo");
    }

    #[test]
    fn nested_segments() {
        assert_eq!(encode_pointer(&["users", "3", "name"]), "/users/3/name");
    }

    #[test]
    fn escapes_tilde_then_slash() {
        // RFC 6901 §3: `~` MUST be encoded as `~0`, `/` as `~1`.
        // Order matters: `~` is encoded first so a literal `~1` survives intact.
        assert_eq!(encode_pointer(&["a~b"]), "/a~0b");
        assert_eq!(encode_pointer(&["a/b"]), "/a~1b");
        assert_eq!(encode_pointer(&["a~/b"]), "/a~0~1b");
        assert_eq!(encode_pointer(&["~1foo"]), "/~01foo");
    }

    #[test]
    fn empty_segment_is_distinct() {
        // Empty key `""` is legal and yields `/` for a single-segment path.
        assert_eq!(encode_pointer(&[""]), "/");
        assert_eq!(encode_pointer(&["", ""]), "//");
    }

    #[test]
    fn unicode_passthrough() {
        assert_eq!(encode_pointer(&["café", "日本"]), "/café/日本");
    }
}
```

- [ ] **Step 2: Add module declaration**

Edit `crates/jfmt-viewer-core/src/lib.rs`. Insert after `pub mod error;`:

```rust
pub mod pointer;
```

- [ ] **Step 3: Run tests; expect compile failure**

Run: `cargo test -p jfmt-viewer-core pointer 2>&1 | tail -5`
Expected: `cannot find function 'encode_pointer' in this scope`.

- [ ] **Step 4: Implement `encode_pointer`**

Prepend to `crates/jfmt-viewer-core/src/pointer.rs` (above tests):

```rust
/// Encode a list of unescaped path segments into an RFC 6901 JSON Pointer.
///
/// Empty input yields `""` (root). Each segment is escaped: `~` becomes `~0`,
/// `/` becomes `~1`. Tilde substitution runs first so a literal `~1` in the
/// input segment becomes `~01`, not `/`.
pub fn encode_pointer(segments: &[&str]) -> String {
    if segments.is_empty() {
        return String::new();
    }
    // Pre-size: 1 byte per `/` separator + segment length + small escape budget.
    let mut out = String::with_capacity(
        segments.len() + segments.iter().map(|s| s.len()).sum::<usize>(),
    );
    for seg in segments {
        out.push('/');
        for c in seg.chars() {
            match c {
                '~' => out.push_str("~0"),
                '/' => out.push_str("~1"),
                _ => out.push(c),
            }
        }
    }
    out
}
```

- [ ] **Step 5: Run tests; expect PASS**

Run: `cargo test -p jfmt-viewer-core pointer 2>&1 | tail -10`
Expected: `test result: ok. 6 passed`.

- [ ] **Step 6: Run clippy**

Run: `cargo clippy -p jfmt-viewer-core --all-targets -- -D warnings 2>&1 | tail -3`
Expected: clean.

- [ ] **Step 7: Commit**

```bash
git add crates/jfmt-viewer-core/src/pointer.rs crates/jfmt-viewer-core/src/lib.rs
git commit -m "$(cat <<'EOF'
feat(viewer-core): RFC 6901 JSON Pointer encoder

Tilde-first escape ordering so a literal `~1` round-trips correctly.
Empty-segment and Unicode passthrough covered.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 4: Public types module

**Files:**
- Create: `crates/jfmt-viewer-core/src/types.rs`
- Modify: `crates/jfmt-viewer-core/src/lib.rs`

- [ ] **Step 1: Write failing tests**

Create `crates/jfmt-viewer-core/src/types.rs`:

```rust
//! Public IPC and index types. Field shapes match spec §4.1 and §3.3.

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
```

- [ ] **Step 2: Add module declaration**

Edit `crates/jfmt-viewer-core/src/lib.rs`. Append:

```rust
pub mod types;

pub use types::{ChildSummary, ContainerEntry, ContainerKind, KeyRef, Kind, NodeId};
```

- [ ] **Step 3: Run tests; expect compile failure**

Run: `cargo test -p jfmt-viewer-core types 2>&1 | tail -5`
Expected: unresolved imports.

- [ ] **Step 4: Implement types**

Prepend to `crates/jfmt-viewer-core/src/types.rs` (above tests):

```rust
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
    pub fn from_str(s: &str) -> Self {
        Self(SmallVec::from_slice(s.as_bytes()))
    }

    pub fn as_str(&self) -> &str {
        // Safety: only constructed from `&str`.
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
```

- [ ] **Step 5: Run tests; expect PASS**

Run: `cargo test -p jfmt-viewer-core types 2>&1 | tail -10`
Expected: `test result: ok. 4 passed`.

- [ ] **Step 6: Run clippy**

Run: `cargo clippy -p jfmt-viewer-core --all-targets -- -D warnings 2>&1 | tail -3`
Expected: clean.

- [ ] **Step 7: Commit**

```bash
git add crates/jfmt-viewer-core/src/types.rs crates/jfmt-viewer-core/src/lib.rs
git commit -m "$(cat <<'EOF'
feat(viewer-core): public IPC + index types

NodeId, Kind, ContainerKind, KeyRef (inline-16 SmallVec), ContainerEntry,
ChildSummary. snake_case rename rule on Kind matches the TypeScript
union in spec §4.1.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 5: Sparse index — JSON containers

**Files:**
- Create: `crates/jfmt-viewer-core/src/index.rs`
- Modify: `crates/jfmt-viewer-core/src/lib.rs`
- Create: `crates/jfmt-viewer-core/tests/fixtures/small.json`
- Create: `crates/jfmt-viewer-core/tests/fixtures/deep.json`

- [ ] **Step 1: Create test fixtures**

Create `crates/jfmt-viewer-core/tests/fixtures/small.json` (literal contents — keep formatting):

```json
{
  "users": [
    {"id": 1, "name": "Alice"},
    {"id": 2, "name": "Bob"}
  ],
  "meta": {
    "version": "2.1",
    "tags": ["prod", "v2"]
  }
}
```

Create `crates/jfmt-viewer-core/tests/fixtures/deep.json` programmatically — emit a Python one-liner into the file:

```bash
python -c "import sys; sys.stdout.write('[' * 500 + '1' + ']' * 500)" > crates/jfmt-viewer-core/tests/fixtures/deep.json
```

(If python is unavailable, use any equivalent: 500 opening `[`, then `1`, then 500 closing `]`.)

- [ ] **Step 2: Write failing tests**

Create `crates/jfmt-viewer-core/src/index.rs`:

```rust
//! Single-pass sparse index over a JSON file. Records every container
//! (object / array) once; leaves are not indexed.

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> Vec<u8> {
        let path = format!(
            "{}/tests/fixtures/{}",
            env!("CARGO_MANIFEST_DIR"),
            name
        );
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
        assert_eq!(root.child_count, 2); // "users" + "meta"
        assert_eq!(root.first_child, Some(NodeId(1))); // "users" array

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
        // 500 nested arrays + the inner integer leaf (not indexed) → 500 containers.
        assert_eq!(idx.entries.len(), 500);
        assert_eq!(idx.entries[0].parent, None);
        // Each subsequent entry's parent is the previous one.
        for i in 1..500 {
            assert_eq!(idx.entries[i].parent, Some(NodeId(i as u64 - 1)));
        }
    }

    #[test]
    fn root_scalar_yields_synthetic_root() {
        // A file that's just `42` has no container; the index produces an empty
        // entries list and `root_kind = None` so callers can detect this.
        let idx = SparseIndex::build(b"42", IndexMode::Json).unwrap();
        assert!(idx.entries.is_empty());
        assert!(idx.root_kind.is_none());
    }
}
```

- [ ] **Step 3: Add module declaration**

Edit `crates/jfmt-viewer-core/src/lib.rs`. Append:

```rust
pub mod index;

pub use index::{IndexMode, SparseIndex};
```

- [ ] **Step 4: Run tests; expect compile failure**

Run: `cargo test -p jfmt-viewer-core index 2>&1 | tail -5`
Expected: `cannot find type 'SparseIndex'`.

- [ ] **Step 5: Implement the index builder**

Prepend to `crates/jfmt-viewer-core/src/index.rs`:

```rust
use crate::error::{Result, ViewerError};
use crate::types::{ContainerEntry, ContainerKind, KeyRef, NodeId};
use jfmt_core::reader::{Event as JsonEvent, EventReader};

/// Selects between JSON (one root value) and NDJSON (one document per line).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndexMode {
    Json,
    Ndjson,
}

/// Densely-packed sparse index over a single file.
#[derive(Debug)]
pub struct SparseIndex {
    pub entries: Vec<ContainerEntry>,
    /// `None` if the file's root value is a scalar.
    pub root_kind: Option<ContainerKind>,
    pub byte_len: u64,
}

impl SparseIndex {
    pub fn build(input: &[u8], mode: IndexMode) -> Result<Self> {
        match mode {
            IndexMode::Json => build_json(input),
            IndexMode::Ndjson => crate::ndjson::build_ndjson(input),
        }
    }
}

/// Frame on the open-container stack while we walk events.
struct Frame {
    node: NodeId,
    /// Next array index to assign (Object frames ignore this).
    next_array_index: u32,
    /// Pending object key, if we just consumed a `Key` event.
    pending_key: Option<String>,
    kind: ContainerKind,
}

fn build_json(input: &[u8]) -> Result<SparseIndex> {
    let mut reader = EventReader::new(input);
    let mut entries: Vec<ContainerEntry> = Vec::new();
    let mut stack: Vec<Frame> = Vec::new();
    let mut root_kind: Option<ContainerKind> = None;

    loop {
        let pos = reader.byte_offset();
        let ev = match reader.next_event() {
            Ok(Some(e)) => e,
            Ok(None) => break,
            Err(e) => {
                return Err(ViewerError::Parse {
                    pos,
                    msg: e.to_string(),
                });
            }
        };

        match ev {
            JsonEvent::StartObject | JsonEvent::StartArray => {
                let kind = match ev {
                    JsonEvent::StartObject => ContainerKind::Object,
                    _ => ContainerKind::Array,
                };

                let (parent, key_or_index) = if let Some(top) = stack.last_mut() {
                    let key = match top.kind {
                        ContainerKind::Object => {
                            top.pending_key.take().ok_or(ViewerError::Parse {
                                pos,
                                msg: "object child without preceding key".into(),
                            })?
                        }
                        ContainerKind::Array => {
                            let i = top.next_array_index;
                            top.next_array_index += 1;
                            i.to_string()
                        }
                        ContainerKind::NdjsonDoc => unreachable!("ndjson handled in build_ndjson"),
                    };
                    (Some(top.node), KeyRef::from_str(&key))
                } else {
                    if root_kind.is_some() {
                        return Err(ViewerError::Parse {
                            pos,
                            msg: "second top-level value (use NDJSON mode)".into(),
                        });
                    }
                    root_kind = Some(kind);
                    (None, KeyRef::from_str(""))
                };

                let id = NodeId(entries.len() as u64);
                let entry = ContainerEntry {
                    file_offset: pos,
                    byte_end: 0, // patched on close
                    parent,
                    key_or_index,
                    kind,
                    child_count: 0,
                    first_child: None,
                };

                // Patch parent's first_child / child_count for this container child.
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

            JsonEvent::EndObject | JsonEvent::EndArray => {
                let frame = stack.pop().ok_or(ViewerError::Parse {
                    pos,
                    msg: "unmatched close".into(),
                })?;
                let end = reader.byte_offset();
                entries[frame.node.0 as usize].byte_end = end;
            }

            JsonEvent::Key(k) => {
                let top = stack.last_mut().ok_or(ViewerError::Parse {
                    pos,
                    msg: "key outside object".into(),
                })?;
                top.pending_key = Some(k.into_owned());
            }

            // Leaf values: increment parent's child_count; do not index.
            JsonEvent::String(_) | JsonEvent::Number(_) | JsonEvent::Bool(_) | JsonEvent::Null => {
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
    }

    Ok(SparseIndex {
        entries,
        root_kind,
        byte_len: input.len() as u64,
    })
}
```

**Note on `jfmt-core::reader` API:** if the actual `Event` enum or `EventReader` method signatures differ from the names used above (e.g. `next_event` vs `read`), inspect `crates/jfmt-core/src/reader/` and adapt. The plan assumes the M3 streaming API; verify before implementing.

- [ ] **Step 6: Run tests; expect PASS**

Run: `cargo test -p jfmt-viewer-core index 2>&1 | tail -10`
Expected: `test result: ok. 3 passed`.

If the deep nesting test stack-overflows, the issue is the `EventReader` itself (not this code) — file an issue and gate the test on `cargo test --features deep` instead. Walking via events is iterative here.

- [ ] **Step 7: Run clippy**

Run: `cargo clippy -p jfmt-viewer-core --all-targets -- -D warnings 2>&1 | tail -3`
Expected: clean.

- [ ] **Step 8: Commit**

```bash
git add crates/jfmt-viewer-core/src/index.rs crates/jfmt-viewer-core/src/lib.rs crates/jfmt-viewer-core/tests/fixtures
git commit -m "$(cat <<'EOF'
feat(viewer-core): sparse JSON index over containers

Single forward pass through jfmt-core EventReader emits one
ContainerEntry per object/array. Leaves bump child_count without
allocating an index slot. Top-level scalar files yield empty entries
with root_kind=None.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 6: NDJSON indexing mode

**Files:**
- Create: `crates/jfmt-viewer-core/src/ndjson.rs`
- Modify: `crates/jfmt-viewer-core/src/lib.rs`
- Create: `crates/jfmt-viewer-core/tests/fixtures/ndjson.ndjson`

- [ ] **Step 1: Create the fixture**

Create `crates/jfmt-viewer-core/tests/fixtures/ndjson.ndjson` (exactly these 5 lines, LF endings):

```
{"id":1,"name":"Alice"}
{"id":2,"name":"Bob"}
{"id":3,"items":[10,20,30]}

42
```

(Line 4 is intentionally blank to test blank-line skipping. Line 5 is a top-level scalar — also valid NDJSON.)

- [ ] **Step 2: Write failing tests**

Create `crates/jfmt-viewer-core/src/ndjson.rs`:

```rust
//! NDJSON-mode indexing. Each non-blank line becomes one direct child of a
//! synthetic NdjsonDoc root.

#[cfg(test)]
mod tests {
    use super::super::index::{IndexMode, SparseIndex};
    use super::super::types::ContainerKind;

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
        // Doc 5 (top-level `42`) is a leaf, not indexed as a container.
        assert_eq!(idx.entries.len(), 5);
        let root = &idx.entries[0];
        assert_eq!(root.kind, ContainerKind::NdjsonDoc);
        assert_eq!(root.parent, None);
        // 4 non-blank lines: the three objects + the `42` leaf.
        assert_eq!(root.child_count, 4);
    }

    #[test]
    fn detects_ndjson_by_extension() {
        assert!(super::is_ndjson_path("foo.ndjson"));
        assert!(super::is_ndjson_path("foo.jsonl"));
        assert!(super::is_ndjson_path("FOO.NDJSON"));
        assert!(!super::is_ndjson_path("foo.json"));
        assert!(!super::is_ndjson_path("foo"));
    }
}
```

- [ ] **Step 3: Add module declaration**

Edit `crates/jfmt-viewer-core/src/lib.rs`. Append:

```rust
pub mod ndjson;

pub use ndjson::is_ndjson_path;
```

- [ ] **Step 4: Run tests; expect compile failure**

Run: `cargo test -p jfmt-viewer-core ndjson 2>&1 | tail -5`
Expected: unresolved.

- [ ] **Step 5: Implement NDJSON indexing**

Prepend to `crates/jfmt-viewer-core/src/ndjson.rs`:

```rust
use std::path::Path;

use crate::error::{Result, ViewerError};
use crate::index::{IndexMode, SparseIndex};
use crate::types::{ContainerEntry, ContainerKind, KeyRef, NodeId};

/// True when the path's lowercased extension is `ndjson` or `jsonl`.
pub fn is_ndjson_path<P: AsRef<Path>>(p: P) -> bool {
    p.as_ref()
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| s.eq_ignore_ascii_case("ndjson") || s.eq_ignore_ascii_case("jsonl"))
        .unwrap_or(false)
}

pub(crate) fn build_ndjson(input: &[u8]) -> Result<SparseIndex> {
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
        // Find next \n (or EOF).
        let end = memchr::memchr(b'\n', &input[start..])
            .map(|i| start + i)
            .unwrap_or(input.len());
        let line = &input[start..end];
        let trimmed_start = line.iter().take_while(|b| b.is_ascii_whitespace()).count();
        let trimmed = &line[trimmed_start..];
        if !trimmed.is_empty() {
            // Index this line as if it were a single JSON document at offset
            // `start + trimmed_start`. Root containers within the line become
            // children of the NDJSON root; leaves still bump child_count.
            let line_offset = (start + trimmed_start) as u64;
            let before_count = entries.len();
            index_one_line(&input[start + trimmed_start..end], line_offset, line_no, &mut entries)?;
            // If the line produced no container, count it as a leaf doc.
            if entries.len() == before_count {
                entries[0].child_count += 1;
            }
        }
        line_no += 1;
        start = end + 1;
    }

    // first_child is the first index after the synthetic root, if any.
    if entries.len() > 1 {
        entries[0].first_child = Some(NodeId(1));
    }

    Ok(SparseIndex {
        entries,
        root_kind: Some(ContainerKind::NdjsonDoc),
        byte_len: input.len() as u64,
    })
}

/// Index one NDJSON line as if it were the root of a JSON document, attaching
/// its top-level container (if any) as a child of the synthetic NDJSON root.
fn index_one_line(
    line: &[u8],
    line_offset: u64,
    line_no: u32,
    entries: &mut Vec<ContainerEntry>,
) -> Result<()> {
    use jfmt_core::reader::{Event as JsonEvent, EventReader};

    let mut reader = EventReader::new(line);
    let mut depth: u32 = 0;
    let mut top_container: Option<usize> = None;

    loop {
        let local_pos = reader.byte_offset();
        let ev = match reader.next_event() {
            Ok(Some(e)) => e,
            Ok(None) => break,
            Err(e) => {
                return Err(ViewerError::Parse {
                    pos: line_offset + local_pos,
                    msg: format!("ndjson line {line_no}: {e}"),
                });
            }
        };
        match ev {
            JsonEvent::StartObject | JsonEvent::StartArray => {
                if depth == 0 {
                    let kind = if matches!(ev, JsonEvent::StartObject) {
                        ContainerKind::Object
                    } else {
                        ContainerKind::Array
                    };
                    let id = entries.len();
                    entries.push(ContainerEntry {
                        file_offset: line_offset + local_pos,
                        byte_end: 0,
                        parent: Some(NodeId::ROOT),
                        key_or_index: KeyRef::from_str(&line_no.to_string()),
                        kind,
                        child_count: 0,
                        first_child: None,
                    });
                    entries[0].child_count += 1;
                    top_container = Some(id);
                }
                depth += 1;
            }
            JsonEvent::EndObject | JsonEvent::EndArray => {
                depth -= 1;
                if depth == 0 {
                    if let Some(id) = top_container {
                        entries[id].byte_end = line_offset + reader.byte_offset();
                    }
                }
            }
            // Inside the line, we still need to count children of the line's
            // top container — but for M8.1 NDJSON we only need top-level shape.
            // Bump child_count of the top container for direct children.
            JsonEvent::String(_) | JsonEvent::Number(_) | JsonEvent::Bool(_) | JsonEvent::Null => {
                if depth == 1 {
                    if let Some(id) = top_container {
                        entries[id].child_count += 1;
                    }
                }
            }
            JsonEvent::Key(_) => {} // keys announced separately; consumed by next StartX/leaf
        }
    }

    Ok(())
}
```

- [ ] **Step 6: Run tests; expect PASS**

Run: `cargo test -p jfmt-viewer-core ndjson 2>&1 | tail -10`
Expected: `test result: ok. 2 passed`.

- [ ] **Step 7: Run clippy + fmt**

Run: `cargo clippy -p jfmt-viewer-core --all-targets -- -D warnings 2>&1 | tail -3`
Run: `cargo fmt --all`
Expected: clippy clean.

- [ ] **Step 8: Commit**

```bash
git add crates/jfmt-viewer-core/src/ndjson.rs crates/jfmt-viewer-core/src/lib.rs crates/jfmt-viewer-core/tests/fixtures/ndjson.ndjson
git commit -m "$(cat <<'EOF'
feat(viewer-core): NDJSON index mode + path detection

Synthetic root with one child per non-blank line. Each line is
indexed by re-running the JSON event walk in single-document mode;
top-level scalars contribute to child_count without an index entry.
is_ndjson_path() handles .ndjson / .jsonl, case-insensitive.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 7: `Session::open` + `get_children`

**Files:**
- Create: `crates/jfmt-viewer-core/src/session.rs`
- Modify: `crates/jfmt-viewer-core/src/lib.rs`

- [ ] **Step 1: Write failing tests**

Create `crates/jfmt-viewer-core/src/session.rs`:

```rust
//! Open file → owned index + content buffer; child / value / pointer accessors.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Kind;

    fn small_session() -> Session {
        let path = format!(
            "{}/tests/fixtures/small.json",
            env!("CARGO_MANIFEST_DIR")
        );
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
        // both are containers so previews are None
        assert!(resp.items[0].preview.is_none());
    }

    #[test]
    fn leaves_carry_inline_preview() {
        let s = small_session();
        // Drill into meta.version (string leaf).
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
        assert!(matches!(err, crate::ViewerError::NotFound(_) | crate::ViewerError::Io(_)));
    }
}
```

- [ ] **Step 2: Add module declaration**

Edit `crates/jfmt-viewer-core/src/lib.rs`. Append:

```rust
pub mod session;

pub use session::{Format, GetChildrenResp, Session};
```

- [ ] **Step 3: Run tests; expect compile failure**

Run: `cargo test -p jfmt-viewer-core session 2>&1 | tail -5`
Expected: unresolved.

- [ ] **Step 4: Implement `Session::open` + `get_children`**

Prepend to `crates/jfmt-viewer-core/src/session.rs`:

```rust
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::error::{Result, ViewerError};
use crate::index::{IndexMode, SparseIndex};
use crate::ndjson::is_ndjson_path;
use crate::types::{ChildSummary, ContainerKind, Kind, NodeId};
use jfmt_core::reader::{Event as JsonEvent, EventReader};

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
            std::io::ErrorKind::NotFound => {
                ViewerError::NotFound(path.display().to_string())
            }
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

        Ok(Self { path, bytes, index, format })
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

        // Scan the parent's byte range with EventReader to enumerate children.
        let slice = &self.bytes[entry.file_offset as usize..entry.byte_end as usize];
        let mut reader = EventReader::new(slice);
        let mut items: Vec<ChildSummary> = Vec::new();
        let mut depth = 0u32;
        let mut next_index = 0u32;
        let mut pending_key: Option<String> = None;

        // For NDJSON synthetic root, depth tracking is different: each line is
        // a separate document. Handle that path with a dedicated helper.
        if entry.kind == ContainerKind::NdjsonDoc && parent == NodeId::ROOT {
            return self.ndjson_root_children(offset, limit);
        }

        loop {
            let local_pos = reader.byte_offset();
            let ev = match reader.next_event() {
                Ok(Some(e)) => e,
                Ok(None) => break,
                Err(e) => {
                    return Err(ViewerError::Parse {
                        pos: entry.file_offset + local_pos,
                        msg: e.to_string(),
                    });
                }
            };
            match ev {
                JsonEvent::StartObject | JsonEvent::StartArray => {
                    if depth == 1 {
                        let key = self.consume_key(entry.kind, &mut pending_key, &mut next_index);
                        // Find this child's NodeId in the index by matching parent + first uncovered offset.
                        let child_offset = entry.file_offset + local_pos;
                        let id = self.find_container_child(parent, child_offset);
                        let child_kind = if matches!(ev, JsonEvent::StartObject) {
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
                JsonEvent::EndObject | JsonEvent::EndArray => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        break;
                    }
                }
                JsonEvent::Key(k) => {
                    if depth == 1 {
                        pending_key = Some(k.into_owned());
                    }
                }
                ev @ (JsonEvent::String(_)
                | JsonEvent::Number(_)
                | JsonEvent::Bool(_)
                | JsonEvent::Null) => {
                    if depth == 1 {
                        let key = self.consume_key(entry.kind, &mut pending_key, &mut next_index);
                        let (kind, preview) = leaf_preview(&ev);
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
        // Linear from `first_child`; chains through siblings via index order
        // (siblings are contiguous in the index because we record in document order).
        let parent_entry = &self.index.entries[parent.0 as usize];
        let start = parent_entry.first_child?.0 as usize;
        let mut id = start;
        while id < self.index.entries.len() {
            let e = &self.index.entries[id];
            if e.parent != Some(parent) {
                // We walked past this parent's siblings.
                return None;
            }
            if e.file_offset == child_offset {
                return Some(NodeId(id as u64));
            }
            id += 1;
        }
        None
    }

    fn ndjson_root_children(&self, offset: u32, limit: u32) -> Result<GetChildrenResp> {
        // Children of the NDJSON synthetic root are the indexed line containers,
        // one per non-empty/non-leaf line. (Leaf-only lines are counted but not
        // surfaced as children in M8.1 — they appear when M8.2 adds line-leaf
        // entries; for now they're absorbed into child_count.)
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

fn leaf_preview(ev: &JsonEvent) -> (Kind, String) {
    let raw = match ev {
        JsonEvent::String(s) => format!("{:?}", s.as_ref()),
        JsonEvent::Number(n) => n.as_ref().to_string(),
        JsonEvent::Bool(true) => "true".into(),
        JsonEvent::Bool(false) => "false".into(),
        JsonEvent::Null => "null".into(),
        _ => unreachable!("non-leaf passed to leaf_preview"),
    };
    let kind = match ev {
        JsonEvent::String(_) => Kind::String,
        JsonEvent::Number(_) => Kind::Number,
        JsonEvent::Bool(_) => Kind::Bool,
        JsonEvent::Null => Kind::Null,
        _ => unreachable!(),
    };
    let preview = if raw.len() <= LEAF_PREVIEW_BYTES {
        raw
    } else {
        let mut s = raw[..LEAF_PREVIEW_TRUNC_BYTES].to_string();
        s.push('…');
        s
    };
    (kind, preview)
}
```

- [ ] **Step 5: Run tests; expect PASS**

Run: `cargo test -p jfmt-viewer-core session 2>&1 | tail -10`
Expected: `test result: ok. 5 passed`.

- [ ] **Step 6: Run clippy**

Run: `cargo clippy -p jfmt-viewer-core --all-targets -- -D warnings 2>&1 | tail -3`
Expected: clean.

- [ ] **Step 7: Commit**

```bash
git add crates/jfmt-viewer-core/src/session.rs crates/jfmt-viewer-core/src/lib.rs
git commit -m "$(cat <<'EOF'
feat(viewer-core): Session::open + get_children with leaf preview

Reads the file fully into memory (M8.1 limit). get_children re-walks
the parent's byte range to emit ChildSummary entries; container
children link back to their NodeId via parent+offset lookup. Leaves
≤ 256B emit full value; longer values truncate to 200B + ellipsis.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 8: `Session::get_value`

**Files:**
- Modify: `crates/jfmt-viewer-core/src/session.rs`

- [ ] **Step 1: Append failing tests**

Append inside the existing `mod tests` in `crates/jfmt-viewer-core/src/session.rs`:

```rust
    #[test]
    fn get_value_pretty_prints_root() {
        let s = small_session();
        let v = s.get_value(NodeId::ROOT, None).unwrap();
        assert!(!v.truncated);
        // Pretty-printed: starts with `{\n`, contains "users" and indentation.
        assert!(v.json.starts_with("{\n"));
        assert!(v.json.contains("\"users\""));
        assert!(v.json.contains("  \"meta\""));
    }

    #[test]
    fn get_value_truncates_when_over_cap() {
        let s = small_session();
        // Cap below the file size forces truncation.
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
```

- [ ] **Step 2: Run tests; expect FAIL on the new ones**

Run: `cargo test -p jfmt-viewer-core session::tests::get_value 2>&1 | tail -10`
Expected: `cannot find method 'get_value'`.

- [ ] **Step 3: Implement `get_value`**

Add to the `impl Session` block in `crates/jfmt-viewer-core/src/session.rs`:

```rust
    pub fn get_value(&self, node: NodeId, max_bytes: Option<u64>) -> Result<GetValueResp> {
        let cap = max_bytes.unwrap_or(DEFAULT_GET_VALUE_CAP);
        let entry = self
            .index
            .entries
            .get(node.0 as usize)
            .ok_or(ViewerError::InvalidNode)?;

        let slice = &self.bytes[entry.file_offset as usize..entry.byte_end as usize];

        // Parse with serde_json (already a workspace dep) and re-serialize pretty.
        // For M8.1, slice must fit in memory; constant-memory pretty over the
        // streaming reader lands in M8.2 if profiling demands it.
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

        let take = cap as usize;
        // Round down to a char boundary so we don't slice mid-codepoint.
        let mut take = take.min(pretty.len());
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
```

Add the response type and constant (anywhere above `impl Session`, preferably next to `GetChildrenResp`):

```rust
const DEFAULT_GET_VALUE_CAP: u64 = 4 * 1024 * 1024; // 4 MB — spec §4.2

#[derive(Debug, Serialize)]
pub struct GetValueResp {
    pub json: String,
    pub truncated: bool,
}
```

Re-export it. Edit `crates/jfmt-viewer-core/src/lib.rs`:

```rust
pub use session::{Format, GetChildrenResp, GetValueResp, Session};
```

- [ ] **Step 4: Run tests; expect PASS**

Run: `cargo test -p jfmt-viewer-core session 2>&1 | tail -10`
Expected: `test result: ok. 8 passed`.

- [ ] **Step 5: Run clippy + fmt**

Run: `cargo clippy -p jfmt-viewer-core --all-targets -- -D warnings 2>&1 | tail -3`
Run: `cargo fmt --all`
Expected: clippy clean.

- [ ] **Step 6: Commit**

```bash
git add crates/jfmt-viewer-core/src/session.rs crates/jfmt-viewer-core/src/lib.rs
git commit -m "$(cat <<'EOF'
feat(viewer-core): Session::get_value with truncation marker

Re-parses the node's byte range with serde_json and pretty-prints.
Subtrees over the cap (default 4 MB) return prefix + literal trailer
hooked for M9 export_subtree command. Char-boundary aware truncation.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 9: `Session::get_pointer`

**Files:**
- Modify: `crates/jfmt-viewer-core/src/session.rs`

- [ ] **Step 1: Append failing tests**

Append inside the existing `mod tests`:

```rust
    #[test]
    fn root_pointer_is_empty() {
        let s = small_session();
        assert_eq!(s.get_pointer(NodeId::ROOT).unwrap(), "");
    }

    #[test]
    fn nested_pointer_uses_keys_and_indexes() {
        let s = small_session();
        // Find users[1] container.
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
```

- [ ] **Step 2: Run; expect FAIL**

Run: `cargo test -p jfmt-viewer-core session::tests 2>&1 | tail -10`
Expected: cannot find method `get_pointer`.

- [ ] **Step 3: Implement**

Add to `impl Session`:

```rust
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
```

- [ ] **Step 4: Run tests; expect PASS**

Run: `cargo test -p jfmt-viewer-core session 2>&1 | tail -10`
Expected: `test result: ok. 11 passed`.

- [ ] **Step 5: Run clippy**

Run: `cargo clippy -p jfmt-viewer-core --all-targets -- -D warnings 2>&1 | tail -3`
Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add crates/jfmt-viewer-core/src/session.rs
git commit -m "$(cat <<'EOF'
feat(viewer-core): Session::get_pointer (RFC 6901)

Walks parent chain from node to root, reverses, then encodes
each segment with the existing pointer module's escape rules.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 10: Property test — index round-trip

**Files:**
- Create: `crates/jfmt-viewer-core/tests/proptest_roundtrip.rs`

- [ ] **Step 1: Write the proptest**

Create `crates/jfmt-viewer-core/tests/proptest_roundtrip.rs`:

```rust
//! Property test: arbitrary JSON → write to bytes → index → reconstruct via
//! get_value at root → must equal serde_json::to_value of the original.

use jfmt_viewer_core::{NodeId, Session};
use proptest::prelude::*;
use serde_json::Value;
use std::io::Write;

fn arb_value(depth: u32) -> impl Strategy<Value = Value> {
    let leaf = prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::Bool),
        any::<i32>().prop_map(|n| Value::Number(n.into())),
        "[a-z0-9]{0,16}".prop_map(Value::String),
    ];
    leaf.prop_recursive(
        depth,
        32,
        8,
        move |inner| {
            prop_oneof![
                proptest::collection::vec(inner.clone(), 0..6).prop_map(Value::Array),
                proptest::collection::vec(("[a-z]{1,8}", inner), 0..6).prop_map(|kv| {
                    let mut m = serde_json::Map::new();
                    for (k, v) in kv {
                        m.insert(k, v);
                    }
                    Value::Object(m)
                }),
            ]
        },
    )
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn root_get_value_matches_input(v in arb_value(4)) {
        let pretty = serde_json::to_vec(&v).unwrap();
        let mut tmp = tempfile::Builder::new()
            .suffix(".json")
            .tempfile()
            .unwrap();
        tmp.write_all(&pretty).unwrap();
        tmp.flush().unwrap();

        let s = Session::open(tmp.path()).unwrap();
        let resp = s.get_value(NodeId::ROOT, None).unwrap();
        let parsed: Value = serde_json::from_str(&resp.json).unwrap();
        prop_assert_eq!(parsed, v);
    }
}
```

- [ ] **Step 2: Run; expect PASS (proves implementations from Tasks 5–8 are consistent)**

Run: `cargo test -p jfmt-viewer-core --test proptest_roundtrip 2>&1 | tail -10`
Expected: `test result: ok. 1 passed`.

If shrinking produces a counterexample, **stop and report** — investigate the shrunk input by saving it and re-running through `Session::open` directly. Likely culprits: NDJSON path detection (false positive on `.json` files containing newlines), or `find_container_child` mis-keying when a value-leaf precedes a container-leaf at the same depth.

- [ ] **Step 3: Run clippy on tests**

Run: `cargo clippy -p jfmt-viewer-core --all-targets -- -D warnings 2>&1 | tail -3`
Expected: clean.

- [ ] **Step 4: Commit**

```bash
git add crates/jfmt-viewer-core/tests/proptest_roundtrip.rs
git commit -m "$(cat <<'EOF'
test(viewer-core): proptest round-trip Session::get_value at root

64 cases of randomly-generated JSON of depth ≤ 4 and breadth ≤ 6 each
direction. Asserts that reading back via Session matches the original
parsed Value.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 11: Streaming search backend (no UI)

**Files:**
- Create: `crates/jfmt-viewer-core/src/search.rs`
- Modify: `crates/jfmt-viewer-core/src/lib.rs`

- [ ] **Step 1: Write failing tests**

Create `crates/jfmt-viewer-core/src/search.rs`:

```rust
//! Streaming substring search across keys and string-leaf values.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Session;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    fn small_session() -> Session {
        let path = format!(
            "{}/tests/fixtures/small.json",
            env!("CARGO_MANIFEST_DIR")
        );
        Session::open(path).unwrap()
    }

    #[test]
    fn finds_value_match() {
        let s = small_session();
        let cancel = Arc::new(AtomicBool::new(false));
        let mut hits = Vec::new();
        let summary = run_search(
            &s,
            &SearchQuery {
                needle: "Alice".into(),
                case_sensitive: true,
                scope: SearchScope::Both,
            },
            &cancel,
            |hit| hits.push(hit.clone()),
        )
        .unwrap();
        assert_eq!(summary.total_hits, 1);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].matched_in, MatchedIn::Value);
        assert!(hits[0].snippet.contains("**Alice**"));
    }

    #[test]
    fn case_insensitive_finds_mixed_case() {
        let s = small_session();
        let cancel = Arc::new(AtomicBool::new(false));
        let mut hits = Vec::new();
        run_search(
            &s,
            &SearchQuery {
                needle: "alice".into(),
                case_sensitive: false,
                scope: SearchScope::Values,
            },
            &cancel,
            |h| hits.push(h.clone()),
        )
        .unwrap();
        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn key_scope_finds_keys_only() {
        let s = small_session();
        let cancel = Arc::new(AtomicBool::new(false));
        let mut hits = Vec::new();
        run_search(
            &s,
            &SearchQuery {
                needle: "name".into(),
                case_sensitive: true,
                scope: SearchScope::Keys,
            },
            &cancel,
            |h| hits.push(h.clone()),
        )
        .unwrap();
        assert!(hits.iter().all(|h| h.matched_in == MatchedIn::Key));
        assert_eq!(hits.len(), 2); // two `name` keys in users[]
    }

    #[test]
    fn cancel_stops_scan() {
        let s = small_session();
        let cancel = Arc::new(AtomicBool::new(true)); // pre-cancelled
        let mut hits = Vec::new();
        let summary = run_search(
            &s,
            &SearchQuery {
                needle: "x".into(),
                case_sensitive: false,
                scope: SearchScope::Both,
            },
            &cancel,
            |h| hits.push(h.clone()),
        )
        .unwrap();
        assert!(summary.cancelled);
    }
}
```

- [ ] **Step 2: Add module declaration**

Edit `crates/jfmt-viewer-core/src/lib.rs`. Append:

```rust
pub mod search;

pub use search::{run_search, MatchedIn, SearchHit, SearchQuery, SearchScope, SearchSummary};
```

- [ ] **Step 3: Run; expect compile fail**

Run: `cargo test -p jfmt-viewer-core search 2>&1 | tail -5`
Expected: unresolved.

- [ ] **Step 4: Implement search**

Prepend to `crates/jfmt-viewer-core/src/search.rs`:

```rust
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::session::Session;
use crate::types::NodeId;
use jfmt_core::reader::{Event as JsonEvent, EventReader};

#[derive(Debug, Clone, Deserialize)]
pub struct SearchQuery {
    pub needle: String,
    pub case_sensitive: bool,
    pub scope: SearchScope,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SearchScope {
    Both,
    Keys,
    Values,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MatchedIn {
    Key,
    Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct SearchHit {
    pub node: Option<NodeId>,
    pub path: String,
    pub matched_in: MatchedIn,
    pub snippet: String,
}

#[derive(Debug, Serialize)]
pub struct SearchSummary {
    pub total_hits: u32,
    pub cancelled: bool,
}

const SNIPPET_RADIUS: usize = 32;

pub fn run_search(
    session: &Session,
    query: &SearchQuery,
    cancel: &Arc<AtomicBool>,
    mut on_hit: impl FnMut(&SearchHit),
) -> Result<SearchSummary> {
    if cancel.load(Ordering::Relaxed) {
        return Ok(SearchSummary { total_hits: 0, cancelled: true });
    }

    let needle_lc: String;
    let needle_bytes: &[u8] = if query.case_sensitive {
        query.needle.as_bytes()
    } else {
        needle_lc = query.needle.to_ascii_lowercase();
        needle_lc.as_bytes()
    };
    if needle_bytes.is_empty() {
        return Ok(SearchSummary { total_hits: 0, cancelled: false });
    }

    let bytes = std::fs::read(session.path())?;
    let mut reader = EventReader::new(&bytes);
    let mut total: u32 = 0;
    let mut path_segments: Vec<String> = Vec::new();
    let mut next_index: Vec<u32> = vec![0]; // outer

    let do_keys = matches!(query.scope, SearchScope::Both | SearchScope::Keys);
    let do_values = matches!(query.scope, SearchScope::Both | SearchScope::Values);

    loop {
        if cancel.load(Ordering::Relaxed) {
            return Ok(SearchSummary { total_hits: total, cancelled: true });
        }
        let pos = reader.byte_offset();
        let ev = match reader.next_event() {
            Ok(Some(e)) => e,
            Ok(None) => break,
            Err(e) => {
                return Err(crate::ViewerError::Parse { pos, msg: e.to_string() });
            }
        };
        match ev {
            JsonEvent::StartObject | JsonEvent::StartArray => {
                next_index.push(0);
                path_segments.push(String::new()); // placeholder; set by Key/array index
            }
            JsonEvent::EndObject | JsonEvent::EndArray => {
                next_index.pop();
                path_segments.pop();
            }
            JsonEvent::Key(k) => {
                let key = k.as_ref().to_string();
                if let Some(last) = path_segments.last_mut() {
                    *last = key.clone();
                }
                if do_keys && contains_match(&key, needle_bytes, query.case_sensitive) {
                    total += 1;
                    let hit = SearchHit {
                        node: None,
                        path: build_path(&path_segments),
                        matched_in: MatchedIn::Key,
                        snippet: snippet(&key, needle_bytes, query.case_sensitive),
                    };
                    on_hit(&hit);
                }
            }
            JsonEvent::String(s) => {
                handle_array_step(&mut path_segments, &mut next_index);
                if do_values {
                    let val = s.as_ref();
                    if contains_match(val, needle_bytes, query.case_sensitive) {
                        total += 1;
                        let hit = SearchHit {
                            node: None,
                            path: build_path(&path_segments),
                            matched_in: MatchedIn::Value,
                            snippet: snippet(val, needle_bytes, query.case_sensitive),
                        };
                        on_hit(&hit);
                    }
                }
            }
            JsonEvent::Number(_) | JsonEvent::Bool(_) | JsonEvent::Null => {
                handle_array_step(&mut path_segments, &mut next_index);
            }
        }
        let _ = pos;
    }

    Ok(SearchSummary { total_hits: total, cancelled: false })
}

fn handle_array_step(path: &mut [String], next_index: &mut [u32]) {
    if let (Some(last), Some(idx)) = (path.last_mut(), next_index.last_mut()) {
        // If the immediate parent is an array, the placeholder segment becomes
        // the next index. Object children already had their key set on Key.
        if last.is_empty() {
            *last = idx.to_string();
        }
        *idx += 1;
    }
}

fn contains_match(haystack: &str, needle: &[u8], case_sensitive: bool) -> bool {
    if case_sensitive {
        memchr::memmem::find(haystack.as_bytes(), needle).is_some()
    } else if haystack.is_ascii() {
        memchr::memmem::find(&haystack.as_bytes().to_ascii_lowercase(), needle).is_some()
    } else {
        haystack.to_lowercase().contains(std::str::from_utf8(needle).unwrap_or(""))
    }
}

fn snippet(haystack: &str, needle: &[u8], case_sensitive: bool) -> String {
    let bytes = haystack.as_bytes();
    let lower;
    let probe = if case_sensitive {
        bytes
    } else {
        lower = bytes.to_ascii_lowercase();
        &lower
    };
    let idx = memchr::memmem::find(probe, needle).unwrap_or(0);
    let start = idx.saturating_sub(SNIPPET_RADIUS);
    let end = (idx + needle.len() + SNIPPET_RADIUS).min(bytes.len());
    let mut start = start;
    while start > 0 && !haystack.is_char_boundary(start) {
        start -= 1;
    }
    let mut end = end;
    while end < bytes.len() && !haystack.is_char_boundary(end) {
        end += 1;
    }
    let prefix = if start > 0 { "…" } else { "" };
    let suffix = if end < bytes.len() { "…" } else { "" };
    let before_match = &haystack[start..idx];
    let matched = &haystack[idx..idx + needle.len()];
    let after_match = &haystack[idx + needle.len()..end];
    format!("{prefix}{before_match}**{matched}**{after_match}{suffix}")
}

fn build_path(segments: &[String]) -> String {
    if segments.is_empty() {
        return String::new();
    }
    let refs: Vec<&str> = segments.iter().map(|s| s.as_str()).collect();
    crate::pointer::encode_pointer(&refs)
}
```

- [ ] **Step 5: Run; expect PASS**

Run: `cargo test -p jfmt-viewer-core search 2>&1 | tail -10`
Expected: `test result: ok. 4 passed`.

- [ ] **Step 6: Run clippy + fmt**

Run: `cargo clippy -p jfmt-viewer-core --all-targets -- -D warnings 2>&1 | tail -3`
Run: `cargo fmt --all`
Expected: clippy clean.

- [ ] **Step 7: Commit**

```bash
git add crates/jfmt-viewer-core/src/search.rs crates/jfmt-viewer-core/src/lib.rs
git commit -m "$(cat <<'EOF'
feat(viewer-core): streaming substring search backend

Walks the file with EventReader, applies memchr substring on keys
and string-leaf values, emits SearchHit per match via a closure.
ASCII fast path for case-insensitive search; non-ASCII falls back
to to_lowercase. Cancelable via Arc<AtomicBool>.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 12: CLI `jfmt view` placeholder subcommand

**Files:**
- Modify: `crates/jfmt-cli/src/cli.rs`
- Create: `crates/jfmt-cli/src/commands/view.rs`
- Modify: `crates/jfmt-cli/src/commands/mod.rs`
- Modify: `crates/jfmt-cli/src/lib.rs`
- Test: `crates/jfmt-cli/tests/cli_view_placeholder.rs`

- [ ] **Step 1: Write the failing CLI e2e**

Create `crates/jfmt-cli/tests/cli_view_placeholder.rs`:

```rust
use assert_cmd::Command;
use predicates::prelude::*;

#[test]
fn view_subcommand_exists_and_prints_placeholder() {
    let mut cmd = Command::cargo_bin("jfmt").unwrap();
    cmd.args(["view", "some.json"]);
    cmd.assert().failure().stderr(predicate::str::contains(
        "GUI viewer not yet bundled",
    ));
}

#[test]
fn view_help_lists_subcommand() {
    let mut cmd = Command::cargo_bin("jfmt").unwrap();
    cmd.args(["--help"]);
    cmd.assert().success().stdout(predicate::str::contains("view"));
}
```

- [ ] **Step 2: Run; expect FAIL**

Run: `cargo test -p jfmt-cli --test cli_view_placeholder 2>&1 | tail -10`
Expected: clap unknown subcommand `view`.

- [ ] **Step 3: Add `View` subcommand to clap**

Open `crates/jfmt-cli/src/cli.rs`. Locate the `Commands` enum (it has `Pretty`, `Minify`, `Convert`, etc.) and add:

```rust
    /// Launch the GUI viewer for a JSON / NDJSON file.
    View {
        /// File to open.
        file: std::path::PathBuf,
    },
```

(Order alphabetically with siblings; if siblings aren't ordered, append.)

- [ ] **Step 4: Wire the dispatch arm**

Open `crates/jfmt-cli/src/lib.rs` (created in M7 Task 12). In the `match` over `Commands` inside `run_cli`, add:

```rust
        Commands::View { file } => commands::view::run(file)?,
```

- [ ] **Step 5: Create the placeholder command**

Edit `crates/jfmt-cli/src/commands/mod.rs`. Append:

```rust
pub mod view;
```

Create `crates/jfmt-cli/src/commands/view.rs`:

```rust
use std::path::Path;

use anyhow::{anyhow, Result};

/// M8.1 placeholder: errors with a clear message. M8.2 replaces this with
/// real GUI binary discovery + spawn.
pub fn run<P: AsRef<Path>>(_file: P) -> Result<()> {
    Err(anyhow!(
        "GUI viewer not yet bundled — run `apps/jfmt-viewer` directly during M8.1 development. \
         Production `jfmt view` integration ships in M8.2."
    ))
}
```

- [ ] **Step 6: Run; expect PASS**

Run: `cargo test -p jfmt-cli --test cli_view_placeholder 2>&1 | tail -8`
Expected: `test result: ok. 2 passed`.

- [ ] **Step 7: Run clippy + workspace tests**

Run: `cargo clippy -p jfmt-cli --all-targets -- -D warnings 2>&1 | tail -3`
Run: `cargo test --workspace 2>&1 | grep -E "^test result:" | head -20`
Expected: all green.

- [ ] **Step 8: Commit**

```bash
git add crates/jfmt-cli/src/cli.rs crates/jfmt-cli/src/lib.rs crates/jfmt-cli/src/commands/mod.rs crates/jfmt-cli/src/commands/view.rs crates/jfmt-cli/tests/cli_view_placeholder.rs
git commit -m "$(cat <<'EOF'
feat(cli): jfmt view placeholder subcommand

Errors with M8.2-deferred message. Reserves the subcommand name and
help-text slot so the surface area is stable; real binary discovery
and spawn lands in M8.2.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 13: Tauri 2 + React app scaffold

**Files:**
- Create: `apps/jfmt-viewer/package.json`
- Create: `apps/jfmt-viewer/pnpm-workspace.yaml`
- Create: `apps/jfmt-viewer/vite.config.ts`
- Create: `apps/jfmt-viewer/tsconfig.json`
- Create: `apps/jfmt-viewer/index.html`
- Create: `apps/jfmt-viewer/src/main.tsx`
- Create: `apps/jfmt-viewer/src/App.tsx`
- Create: `apps/jfmt-viewer/src/api.ts`
- Create: `apps/jfmt-viewer/src-tauri/Cargo.toml`
- Create: `apps/jfmt-viewer/src-tauri/tauri.conf.json`
- Create: `apps/jfmt-viewer/src-tauri/build.rs`
- Create: `apps/jfmt-viewer/src-tauri/src/main.rs`
- Create: `apps/jfmt-viewer/src-tauri/src/lib.rs`
- Modify: `Cargo.toml` (root) — add `apps/jfmt-viewer/src-tauri` to workspace members
- Modify: `.gitignore` — exclude `node_modules`, `dist`, `target/` under apps

- [ ] **Step 1: Verify Tauri 2 prerequisites**

Run: `cargo install --list | findstr tauri-cli` (Windows) or `cargo install --list | grep tauri-cli`. If `tauri-cli` is not installed, run:

```bash
cargo install tauri-cli@^2 --locked
```

Verify Node/pnpm:

```bash
node --version    # ≥ 20
pnpm --version    # ≥ 9; if missing: npm i -g pnpm@9
```

If any tool is missing or wrong version, **stop and ask the user** before proceeding — system-level installs need their consent.

- [ ] **Step 2: Frontend manifest**

Create `apps/jfmt-viewer/package.json`:

```json
{
  "name": "jfmt-viewer",
  "private": true,
  "version": "0.0.1",
  "type": "module",
  "scripts": {
    "dev": "vite",
    "build": "tsc && vite build",
    "tauri": "tauri"
  },
  "dependencies": {
    "@tauri-apps/api": "^2",
    "@tauri-apps/plugin-dialog": "^2",
    "react": "^18",
    "react-dom": "^18"
  },
  "devDependencies": {
    "@tauri-apps/cli": "^2",
    "@types/react": "^18",
    "@types/react-dom": "^18",
    "@vitejs/plugin-react": "^4",
    "typescript": "^5",
    "vite": "^5"
  }
}
```

Create `apps/jfmt-viewer/pnpm-workspace.yaml`:

```yaml
packages:
  - .
```

Create `apps/jfmt-viewer/vite.config.ts`:

```ts
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: { port: 1420, strictPort: true },
});
```

Create `apps/jfmt-viewer/tsconfig.json`:

```json
{
  "compilerOptions": {
    "target": "ES2022",
    "useDefineForClassFields": true,
    "lib": ["ES2022", "DOM", "DOM.Iterable"],
    "module": "ESNext",
    "skipLibCheck": true,
    "moduleResolution": "bundler",
    "allowImportingTsExtensions": true,
    "resolveJsonModule": true,
    "isolatedModules": true,
    "noEmit": true,
    "jsx": "react-jsx",
    "strict": true,
    "noUnusedLocals": true,
    "noUnusedParameters": true,
    "noFallthroughCasesInSwitch": true
  },
  "include": ["src"]
}
```

Create `apps/jfmt-viewer/index.html`:

```html
<!doctype html>
<html lang="en">
  <head>
    <meta charset="UTF-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1.0" />
    <title>jfmt viewer</title>
  </head>
  <body>
    <div id="root"></div>
    <script type="module" src="/src/main.tsx"></script>
  </body>
</html>
```

Create `apps/jfmt-viewer/src/main.tsx`:

```tsx
import React from "react";
import ReactDOM from "react-dom/client";
import { App } from "./App";

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
```

Create `apps/jfmt-viewer/src/api.ts`:

```ts
import { invoke } from "@tauri-apps/api/core";

export type NodeId = number;

export interface ChildSummary {
  id: NodeId | null;
  key: string;
  kind: "object" | "array" | "string" | "number" | "bool" | "null" | "ndjson_doc";
  child_count: number;
  preview: string | null;
}

export async function ping(): Promise<string> {
  return invoke<string>("ping");
}
```

Create `apps/jfmt-viewer/src/App.tsx`:

```tsx
import { useEffect, useState } from "react";
import { ping } from "./api";

export function App() {
  const [msg, setMsg] = useState<string>("…");
  useEffect(() => {
    ping().then(setMsg).catch((e) => setMsg(`error: ${String(e)}`));
  }, []);
  return (
    <main style={{ fontFamily: "system-ui", padding: 16 }}>
      <h1>jfmt-viewer (M8.1 scaffold)</h1>
      <p>backend says: {msg}</p>
    </main>
  );
}
```

- [ ] **Step 3: Tauri Rust crate**

Create `apps/jfmt-viewer/src-tauri/Cargo.toml`:

```toml
[package]
name = "jfmt-viewer-app"
version = "0.0.1"
edition = "2021"
license = "MIT OR Apache-2.0"
description = "jfmt GUI viewer (Tauri 2)"
default-run = "jfmt-viewer-app"

[dependencies]
jfmt-viewer-core = { path = "../../../crates/jfmt-viewer-core" }
serde.workspace = true
serde_json.workspace = true
tauri = { version = "2", features = [] }
tauri-plugin-dialog = "2"
tokio = { workspace = true, features = ["rt-multi-thread", "sync"] }

[build-dependencies]
tauri-build = { version = "2", features = [] }

[lints]
workspace = true
```

Create `apps/jfmt-viewer/src-tauri/build.rs`:

```rust
fn main() {
    tauri_build::build()
}
```

Create `apps/jfmt-viewer/src-tauri/tauri.conf.json`:

```json
{
  "$schema": "https://schema.tauri.app/config/2",
  "productName": "jfmt-viewer",
  "version": "0.0.1",
  "identifier": "io.jfmt.viewer",
  "build": {
    "beforeDevCommand": "pnpm dev",
    "beforeBuildCommand": "pnpm build",
    "devUrl": "http://localhost:1420",
    "frontendDist": "../dist"
  },
  "app": {
    "windows": [
      {
        "title": "jfmt-viewer (M8.1)",
        "width": 1200,
        "height": 800,
        "resizable": true
      }
    ],
    "security": { "csp": null }
  },
  "bundle": { "active": false }
}
```

Create `apps/jfmt-viewer/src-tauri/src/main.rs`:

```rust
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    jfmt_viewer_app::run();
}
```

Create `apps/jfmt-viewer/src-tauri/src/lib.rs`:

```rust
#[tauri::command]
fn ping() -> String {
    "pong".into()
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![ping])
        .run(tauri::generate_context!())
        .expect("error while running tauri app");
}
```

- [ ] **Step 4: Add the Tauri crate to the workspace**

Edit root `Cargo.toml`. In `[workspace] members`, append:

```toml
"apps/jfmt-viewer/src-tauri",
```

(maintain alphabetical order if your existing list is sorted).

- [ ] **Step 5: Update `.gitignore`**

Append to `.gitignore`:

```
# jfmt-viewer build outputs
apps/*/node_modules/
apps/*/dist/
apps/*/src-tauri/target/
apps/*/.vite/
apps/*/pnpm-debug.log
```

- [ ] **Step 6: Install + verify cargo build of the Tauri crate**

```bash
cd apps/jfmt-viewer && pnpm install --frozen-lockfile=false
cd ../..
cargo build -p jfmt-viewer-app 2>&1 | tail -8
```

The first `pnpm install` writes a fresh `pnpm-lock.yaml` — commit it.

Expected `cargo build` output: `Finished dev [unoptimized + debuginfo] target(s)`. If `tauri-build` complains about missing icons, generate the required icon set with the Tauri CLI's icon-from-PNG helper:

```bash
cd apps/jfmt-viewer
# Use any 32×32+ square PNG. If you don't have one handy, generate a solid-color one:
#   python -c "from PIL import Image; Image.new('RGB',(64,64),(33,114,229)).save('seed.png')"
# Or copy any existing PNG with side ≥ 32 px to apps/jfmt-viewer/seed.png.
pnpm tauri icon seed.png
cd ../..
```

`pnpm tauri icon` writes a full set under `src-tauri/icons/` — that's what `tauri-build` looks for. The `bundle.active=false` setting in `tauri.conf.json` skips installer creation but icons are still required at compile time.

If pnpm or cargo build fails for any other reason, **stop and report**.

- [ ] **Step 7: Smoke run (optional during development)**

`cd apps/jfmt-viewer && pnpm tauri dev` should open a window saying "backend says: pong". Close after verifying. Not required for CI.

- [ ] **Step 8: Run workspace clippy and tests**

Run: `cargo clippy --workspace --all-targets -- -D warnings 2>&1 | tail -3`
Run: `cargo test --workspace 2>&1 | grep -E "^test result: ok|^test result: FAILED" | head -20`
Expected: clippy clean; existing tests still pass; one new crate has no tests yet.

- [ ] **Step 9: Commit**

```bash
git add apps/ Cargo.toml .gitignore
git commit -m "$(cat <<'EOF'
chore(viewer): scaffold Tauri 2 + React + Vite app

apps/jfmt-viewer/ holds the GUI:
- frontend: pnpm + vite + react 18 + ts 5
- src-tauri: workspace member jfmt-viewer-app, depends on
  jfmt-viewer-core, exposes one ping command end-to-end
Window opens with "pong" round-tripped over IPC, proving the toolchain
is wired. Bundle disabled in M8.1; release packaging is M8.2.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 14: Wire the seven IPC commands

**Files:**
- Modify: `apps/jfmt-viewer/src-tauri/Cargo.toml` (add `dashmap`, `uuid`)
- Create: `apps/jfmt-viewer/src-tauri/src/state.rs`
- Create: `apps/jfmt-viewer/src-tauri/src/commands.rs`
- Modify: `apps/jfmt-viewer/src-tauri/src/lib.rs`
- Modify: `apps/jfmt-viewer/src/api.ts`

- [ ] **Step 1: Add deps**

Edit `apps/jfmt-viewer/src-tauri/Cargo.toml`. Append to `[dependencies]`:

```toml
dashmap = "6"
uuid = { version = "1", features = ["v4"] }
```

(both small, well-trodden; pin to current major.)

- [ ] **Step 2: Session state container**

Create `apps/jfmt-viewer/src-tauri/src/state.rs`:

```rust
use std::sync::Arc;

use dashmap::DashMap;
use jfmt_viewer_core::Session;

pub struct ViewerState {
    pub sessions: DashMap<String, Arc<Session>>,
    pub search_cancels: DashMap<String, Arc<std::sync::atomic::AtomicBool>>,
}

impl ViewerState {
    pub fn new() -> Self {
        Self {
            sessions: DashMap::new(),
            search_cancels: DashMap::new(),
        }
    }
}
```

- [ ] **Step 3: Implement the seven commands**

Create `apps/jfmt-viewer/src-tauri/src/commands.rs`:

```rust
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use jfmt_viewer_core::{
    run_search, ChildSummary, Format, NodeId, SearchHit, SearchQuery, SearchScope, Session,
    ViewerError,
};
use serde::Serialize;
use tauri::ipc::Channel;
use tauri::State;

use crate::state::ViewerState;

#[derive(Serialize)]
pub struct OpenFileResp {
    pub session_id: String,
    pub root_id: u64,
    pub format: Format,
    pub total_bytes: u64,
}

#[derive(Serialize, Clone)]
#[serde(tag = "phase", rename_all = "lowercase")]
pub enum IndexProgress {
    Scanning { bytes_done: u64, bytes_total: u64 },
    Ready { build_ms: u64 },
    Error { message: String },
}

#[tauri::command]
pub async fn open_file(
    path: String,
    on_progress: Channel<IndexProgress>,
    state: State<'_, ViewerState>,
) -> Result<OpenFileResp, ViewerError> {
    let path = PathBuf::from(&path);
    if !path.exists() {
        return Err(ViewerError::NotFound(path.display().to_string()));
    }
    // M8.1: synchronous build for simplicity. We still emit the protocol so
    // M8.2 can swap in a real spawn_blocking + progress without changing the
    // frontend.
    let start = Instant::now();
    let session = tokio::task::spawn_blocking(move || Session::open(&path))
        .await
        .map_err(|e| ViewerError::Io(e.to_string()))??;
    let id = uuid::Uuid::new_v4().to_string();
    let total = session.byte_len();
    let format = session.format();
    state.sessions.insert(id.clone(), Arc::new(session));

    // Emit a single progress frame followed by Ready.
    let _ = on_progress.send(IndexProgress::Scanning {
        bytes_done: total,
        bytes_total: total,
    });
    let _ = on_progress.send(IndexProgress::Ready {
        build_ms: start.elapsed().as_millis() as u64,
    });

    Ok(OpenFileResp {
        session_id: id,
        root_id: NodeId::ROOT.0,
        format,
        total_bytes: total,
    })
}

#[tauri::command]
pub async fn close_file(session_id: String, state: State<'_, ViewerState>) -> Result<(), ViewerError> {
    state.sessions.remove(&session_id);
    Ok(())
}

#[derive(Serialize)]
pub struct GetChildrenResp {
    pub items: Vec<ChildSummary>,
    pub total: u32,
}

#[tauri::command]
pub async fn get_children(
    session_id: String,
    parent: u64,
    offset: u32,
    limit: u32,
    state: State<'_, ViewerState>,
) -> Result<GetChildrenResp, ViewerError> {
    let session = state
        .sessions
        .get(&session_id)
        .ok_or(ViewerError::InvalidSession)?
        .clone();
    let resp = session.get_children(NodeId(parent), offset, limit)?;
    Ok(GetChildrenResp {
        items: resp.items,
        total: resp.total,
    })
}

#[derive(Serialize)]
pub struct GetValueResp {
    pub json: String,
    pub truncated: bool,
}

#[tauri::command]
pub async fn get_value(
    session_id: String,
    node: u64,
    max_bytes: Option<u64>,
    state: State<'_, ViewerState>,
) -> Result<GetValueResp, ViewerError> {
    let session = state
        .sessions
        .get(&session_id)
        .ok_or(ViewerError::InvalidSession)?
        .clone();
    let resp = session.get_value(NodeId(node), max_bytes)?;
    Ok(GetValueResp {
        json: resp.json,
        truncated: resp.truncated,
    })
}

#[derive(Serialize)]
pub struct GetPointerResp {
    pub pointer: String,
}

#[tauri::command]
pub async fn get_pointer(
    session_id: String,
    node: u64,
    state: State<'_, ViewerState>,
) -> Result<GetPointerResp, ViewerError> {
    let session = state
        .sessions
        .get(&session_id)
        .ok_or(ViewerError::InvalidSession)?
        .clone();
    let pointer = session.get_pointer(NodeId(node))?;
    Ok(GetPointerResp { pointer })
}

#[derive(Serialize, Clone)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum SearchEvent {
    Hit {
        node: Option<u64>,
        path: String,
        matched_in: jfmt_viewer_core::MatchedIn,
        snippet: String,
    },
    Progress {
        bytes_done: u64,
        bytes_total: u64,
        hits_so_far: u32,
    },
    Done {
        total_hits: u32,
        elapsed_ms: u64,
    },
    Cancelled,
    Error {
        message: String,
    },
}

#[derive(Serialize)]
pub struct SearchHandle {
    pub id: String,
}

#[tauri::command]
pub async fn search(
    session_id: String,
    query: SearchQuery,
    on_event: Channel<SearchEvent>,
    state: State<'_, ViewerState>,
) -> Result<SearchHandle, ViewerError> {
    let session = state
        .sessions
        .get(&session_id)
        .ok_or(ViewerError::InvalidSession)?
        .clone();
    let handle_id = uuid::Uuid::new_v4().to_string();
    let cancel = Arc::new(AtomicBool::new(false));
    state
        .search_cancels
        .insert(handle_id.clone(), cancel.clone());

    let on_event_clone = on_event.clone();
    let cancel_clone = cancel.clone();
    let started = Instant::now();
    tokio::task::spawn_blocking(move || {
        let result = run_search(&session, &query, &cancel_clone, |hit: &SearchHit| {
            let _ = on_event_clone.send(SearchEvent::Hit {
                node: hit.node.map(|n| n.0),
                path: hit.path.clone(),
                matched_in: hit.matched_in,
                snippet: hit.snippet.clone(),
            });
        });
        let elapsed_ms = started.elapsed().as_millis() as u64;
        match result {
            Ok(s) if s.cancelled => {
                let _ = on_event_clone.send(SearchEvent::Cancelled);
            }
            Ok(s) => {
                let _ = on_event_clone.send(SearchEvent::Done {
                    total_hits: s.total_hits,
                    elapsed_ms,
                });
            }
            Err(e) => {
                let _ = on_event_clone.send(SearchEvent::Error {
                    message: e.to_string(),
                });
            }
        }
    });

    Ok(SearchHandle { id: handle_id })
}

#[tauri::command]
pub async fn cancel_search(
    handle: String,
    state: State<'_, ViewerState>,
) -> Result<(), ViewerError> {
    if let Some((_, cancel)) = state.search_cancels.remove(&handle) {
        cancel.store(true, Ordering::Relaxed);
    }
    Ok(())
}
```

- [ ] **Step 4: Register commands and state**

Replace `apps/jfmt-viewer/src-tauri/src/lib.rs`:

```rust
mod commands;
mod state;

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(state::ViewerState::new())
        .invoke_handler(tauri::generate_handler![
            commands::open_file,
            commands::close_file,
            commands::get_children,
            commands::get_value,
            commands::get_pointer,
            commands::search,
            commands::cancel_search,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri app");
}
```

- [ ] **Step 5: Replace `api.ts` with the full IPC surface**

Overwrite `apps/jfmt-viewer/src/api.ts`:

```ts
import { invoke, Channel } from "@tauri-apps/api/core";

export type NodeId = number;
export type Kind =
  | "object"
  | "array"
  | "string"
  | "number"
  | "bool"
  | "null"
  | "ndjson_doc";

export interface ChildSummary {
  id: NodeId | null;
  key: string;
  kind: Kind;
  child_count: number;
  preview: string | null;
}

export interface OpenFileResp {
  session_id: string;
  root_id: NodeId;
  format: "json" | "ndjson";
  total_bytes: number;
}

export type IndexProgress =
  | { phase: "scanning"; bytes_done: number; bytes_total: number }
  | { phase: "ready"; build_ms: number }
  | { phase: "error"; message: string };

export interface GetChildrenResp {
  items: ChildSummary[];
  total: number;
}

export interface SearchQuery {
  needle: string;
  case_sensitive: boolean;
  scope: "both" | "keys" | "values";
}

export type SearchEvent =
  | { kind: "hit"; node: NodeId | null; path: string; matched_in: "key" | "value"; snippet: string }
  | { kind: "progress"; bytes_done: number; bytes_total: number; hits_so_far: number }
  | { kind: "done"; total_hits: number; elapsed_ms: number }
  | { kind: "cancelled" }
  | { kind: "error"; message: string };

export async function openFile(
  path: string,
  onProgress: (p: IndexProgress) => void,
): Promise<OpenFileResp> {
  const channel = new Channel<IndexProgress>();
  channel.onmessage = onProgress;
  return invoke<OpenFileResp>("open_file", { path, onProgress: channel });
}

export async function closeFile(sessionId: string): Promise<void> {
  await invoke("close_file", { sessionId });
}

export async function getChildren(
  sessionId: string,
  parent: NodeId,
  offset: number,
  limit: number,
): Promise<GetChildrenResp> {
  return invoke<GetChildrenResp>("get_children", {
    sessionId,
    parent,
    offset,
    limit,
  });
}

export async function getValue(
  sessionId: string,
  node: NodeId,
  maxBytes?: number,
): Promise<{ json: string; truncated: boolean }> {
  return invoke("get_value", { sessionId, node, maxBytes });
}

export async function getPointer(
  sessionId: string,
  node: NodeId,
): Promise<string> {
  const r = await invoke<{ pointer: string }>("get_pointer", { sessionId, node });
  return r.pointer;
}

export async function search(
  sessionId: string,
  query: SearchQuery,
  onEvent: (e: SearchEvent) => void,
): Promise<{ id: string }> {
  const channel = new Channel<SearchEvent>();
  channel.onmessage = onEvent;
  return invoke<{ id: string }>("search", { sessionId, query, onEvent: channel });
}

export async function cancelSearch(handle: string): Promise<void> {
  await invoke("cancel_search", { handle });
}
```

- [ ] **Step 6: Build the Tauri crate**

Run: `cargo build -p jfmt-viewer-app 2>&1 | tail -8`
Expected: `Finished`. Any unresolved import → diagnose by inspecting compilation output.

- [ ] **Step 7: Run workspace clippy + tests**

Run: `cargo clippy --workspace --all-targets -- -D warnings 2>&1 | tail -5`
Run: `cargo test --workspace 2>&1 | grep -E "^test result: " | head -25`
Expected: clippy clean; all existing tests still pass.

- [ ] **Step 8: Commit**

```bash
git add apps/jfmt-viewer/src-tauri apps/jfmt-viewer/src/api.ts apps/jfmt-viewer/pnpm-lock.yaml 2>/dev/null || true
git add apps/jfmt-viewer/src-tauri apps/jfmt-viewer/src/api.ts
git commit -m "$(cat <<'EOF'
feat(viewer): wire all 7 Tauri IPC commands to viewer-core

open_file / close_file / get_children / get_value / get_pointer
delegate to jfmt-viewer-core::Session. search runs on a blocking pool
and streams Hit/Progress/Done/Cancelled/Error frames over a typed
Channel; cancel_search flips an AtomicBool watched by the scanner.
ViewerState holds DashMap<SessionId, Arc<Session>>.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 15: Minimal React tree UI (no virtual scroll)

**Files:**
- Create: `apps/jfmt-viewer/src/components/Tree.tsx`
- Create: `apps/jfmt-viewer/src/components/TreeRow.tsx`
- Modify: `apps/jfmt-viewer/src/App.tsx`

- [ ] **Step 1: Tree row component**

Create `apps/jfmt-viewer/src/components/TreeRow.tsx`:

```tsx
import { ChildSummary } from "../api";

interface Props {
  child: ChildSummary;
  depth: number;
  expanded: boolean;
  onToggle: () => void;
}

export function TreeRow({ child, depth, expanded, onToggle }: Props) {
  const isContainer = child.id !== null;
  const chevron = isContainer ? (expanded ? "▾" : "▸") : "•";
  const sizeHint = isContainer ? `[${child.child_count}]` : (child.preview ?? "");
  return (
    <div
      style={{
        paddingLeft: depth * 16,
        cursor: isContainer ? "pointer" : "default",
        whiteSpace: "nowrap",
        fontFamily: "ui-monospace, monospace",
        fontSize: 13,
      }}
      onClick={isContainer ? onToggle : undefined}
    >
      <span style={{ width: 14, display: "inline-block" }}>{chevron}</span>
      <span style={{ color: "#888" }}> {child.kind}</span>{" "}
      <strong>{child.key}</strong>{" "}
      <span style={{ color: "#444" }}>{sizeHint}</span>
    </div>
  );
}
```

- [ ] **Step 2: Tree container with lazy expansion**

Create `apps/jfmt-viewer/src/components/Tree.tsx`:

```tsx
import { useEffect, useState } from "react";
import { ChildSummary, getChildren, NodeId } from "../api";
import { TreeRow } from "./TreeRow";

interface Props {
  sessionId: string;
  rootId: NodeId;
}

interface NodeState {
  loaded: ChildSummary[];
  total: number;
  expanded: boolean;
}

export function Tree({ sessionId, rootId }: Props) {
  const [byId, setById] = useState<Map<NodeId, NodeState>>(new Map());

  useEffect(() => {
    let cancelled = false;
    (async () => {
      const r = await getChildren(sessionId, rootId, 0, 200);
      if (cancelled) return;
      const m = new Map<NodeId, NodeState>();
      m.set(rootId, { loaded: r.items, total: r.total, expanded: true });
      setById(m);
    })();
    return () => {
      cancelled = true;
    };
  }, [sessionId, rootId]);

  async function toggle(id: NodeId) {
    const cur = byId.get(id);
    if (cur) {
      const next = new Map(byId);
      next.set(id, { ...cur, expanded: !cur.expanded });
      setById(next);
      return;
    }
    const r = await getChildren(sessionId, id, 0, 200);
    const next = new Map(byId);
    next.set(id, { loaded: r.items, total: r.total, expanded: true });
    setById(next);
  }

  function render(id: NodeId, depth: number): JSX.Element[] {
    const state = byId.get(id);
    if (!state) return [];
    const out: JSX.Element[] = [];
    for (const c of state.loaded) {
      out.push(
        <TreeRow
          key={`${id}-${c.key}-${c.id ?? "leaf"}`}
          child={c}
          depth={depth}
          expanded={c.id !== null && (byId.get(c.id)?.expanded ?? false)}
          onToggle={() => c.id !== null && toggle(c.id)}
        />,
      );
      if (c.id !== null && byId.get(c.id)?.expanded) {
        out.push(...render(c.id, depth + 1));
      }
    }
    if (state.total > state.loaded.length) {
      out.push(
        <div key={`${id}-more`} style={{ paddingLeft: depth * 16, color: "#888" }}>
          (+{state.total - state.loaded.length} more — virtual scroll lands in M8.2)
        </div>,
      );
    }
    return out;
  }

  return <div>{render(rootId, 0)}</div>;
}
```

- [ ] **Step 3: Wire App with file open**

Replace `apps/jfmt-viewer/src/App.tsx`:

```tsx
import { useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { closeFile, openFile } from "./api";
import { Tree } from "./components/Tree";

interface OpenSession {
  sessionId: string;
  rootId: number;
  path: string;
  totalBytes: number;
  format: string;
}

export function App() {
  const [session, setSession] = useState<OpenSession | null>(null);
  const [progress, setProgress] = useState<string>("");

  async function pickFile() {
    const picked = await open({
      multiple: false,
      filters: [{ name: "JSON", extensions: ["json", "ndjson", "jsonl"] }],
    });
    if (!picked || Array.isArray(picked)) return;
    if (session) await closeFile(session.sessionId);
    setProgress("opening…");
    const resp = await openFile(picked, (p) => {
      if (p.phase === "scanning") {
        setProgress(`scanning: ${p.bytes_done}/${p.bytes_total}`);
      } else if (p.phase === "ready") {
        setProgress(`ready (${p.build_ms} ms)`);
      } else if (p.phase === "error") {
        setProgress(`error: ${p.message}`);
      }
    });
    setSession({
      sessionId: resp.session_id,
      rootId: resp.root_id,
      path: picked,
      totalBytes: resp.total_bytes,
      format: resp.format,
    });
  }

  return (
    <main style={{ fontFamily: "system-ui", padding: 16 }}>
      <h2 style={{ margin: "0 0 8px" }}>jfmt-viewer (M8.1)</h2>
      <button onClick={pickFile}>📁 Open</button>{" "}
      <span style={{ color: "#666" }}>{progress}</span>
      {session && (
        <>
          <h3 style={{ marginTop: 16 }}>
            {session.path} · {session.format} · {session.totalBytes} bytes
          </h3>
          <Tree sessionId={session.sessionId} rootId={session.rootId} />
        </>
      )}
    </main>
  );
}
```

- [ ] **Step 4: Build the frontend**

Run: `cd apps/jfmt-viewer && pnpm build && cd ../..`
Expected: `vite v5.x building for production` followed by `dist/` written. tsc errors → fix.

- [ ] **Step 5: Smoke run (manual; not CI gate)**

`cd apps/jfmt-viewer && pnpm tauri dev`. Open the workspace's `crates/jfmt-viewer-core/tests/fixtures/small.json`. Confirm:
- The "📁 Open" button picks the file.
- After "ready (Nms)", the tree shows `users` and `meta` at depth 0.
- Clicking `users` expands to `0` and `1`.
- Clicking `meta` shows `version`, `created` (if present), `tags`.
- (+N more) banner appears for arrays larger than the 200-item batch.

If any of these fails, **stop and report**: typically the cause is `find_container_child` not matching offsets — verify `EventReader::byte_offset()` semantics on `StartObject`/`StartArray` (offset of the brace, not the key).

- [ ] **Step 6: Workspace clippy + tests**

Run: `cargo clippy --workspace --all-targets -- -D warnings 2>&1 | tail -3`
Run: `cargo test --workspace 2>&1 | grep -E "^test result:" | head -25`
Expected: clippy clean; all green.

- [ ] **Step 7: Commit**

```bash
git add apps/jfmt-viewer/src
git commit -m "$(cat <<'EOF'
feat(viewer): minimal React tree UI (M8.1 internal)

Tree component renders children via getChildren and lazy-expands
on click. No virtual scroll yet — caps at 200 children per page
with a "+N more" placeholder for M8.2. App.tsx wires the open-file
dialog and shows index-progress messages from the Tauri channel.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 16: M8.1 internal milestone marker

**Files:**
- Create: `crates/jfmt-viewer-core/CHANGELOG.md`
- Modify: `docs/superpowers/specs/2026-04-23-jfmt-phase1-design.md` (add a one-liner referencing M8 spec — optional; defer if file is locked)

- [ ] **Step 1: Create the crate's CHANGELOG**

Create `crates/jfmt-viewer-core/CHANGELOG.md`:

```markdown
# jfmt-viewer-core changelog

All notable changes to this crate will be documented in this file.
The format follows Keep a Changelog; this crate predates a stable
release line and is versioned `0.0.x` until M8.2 ships v0.3.0.

## [0.0.1] — 2026-05-08 (M8.1 internal)

### Added
- Sparse in-memory index over JSON / NDJSON containers (`SparseIndex`).
- `Session::open` reads file fully into memory and builds the index.
- `Session::get_children(parent, offset, limit)` returns a paginated
  window of children with leaf-value previews.
- `Session::get_value(node, max_bytes)` pretty-prints the subtree at
  `node`, truncating at `max_bytes` (default 4 MB) with a literal
  trailer reserved for M9 export.
- `Session::get_pointer(node)` produces an RFC 6901 JSON Pointer.
- `run_search(query, cancel, on_hit)` streams substring matches across
  keys and string-leaf values; ASCII fast path; cancelable.
- Property-tested round-trip: arbitrary JSON → index → `get_value`
  matches `serde_json::Value`.

### Limits
- Whole-file in-memory load: ~10 GB ceiling depending on host RAM.
- No persistent index sidecar (M9).
- Number / bool / null leaves not searchable (M9).

### Used by
- `apps/jfmt-viewer/src-tauri` (Tauri 2 GUI shell, M8.1 internal).
```

- [ ] **Step 2: Verify the milestone**

Run all of:

```bash
cargo test --workspace 2>&1 | tail -5
cargo clippy --workspace --all-targets -- -D warnings 2>&1 | tail -3
cargo fmt --all -- --check
cd apps/jfmt-viewer && pnpm build && cd ../..
```

Expected: each completes successfully.

- [ ] **Step 3: Final commit (no tag)**

M8.1 is internal; **do not** create a git tag yet. Tagging happens at M8.2 ship time as `v0.3.0`.

```bash
git add crates/jfmt-viewer-core/CHANGELOG.md
git commit -m "$(cat <<'EOF'
docs(viewer-core): CHANGELOG for 0.0.1 (M8.1 internal milestone)

M8.1 — Viewer Core + Minimal UI complete:
- jfmt-viewer-core 0.0.1 (index + session + search)
- apps/jfmt-viewer Tauri 2 + React shell with all 7 IPC commands
- jfmt view CLI placeholder reserves the subcommand surface
M8.2 will add virtual scroll, preview pane, search UI, NDJSON
top-level rendering, real `jfmt view` binary discovery, Tauri
build + cargo-dist coordination, and ship v0.3.0.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

- [ ] **Step 4: Report**

Print to the session a one-screen summary listing every task's commit SHA and any deviations from the plan (e.g. cargo dep version drift, Tauri-build icon workaround, EventReader API mismatches discovered during Task 5). The user reads this before deciding whether to start M8.2.

---

## Plan summary

16 tasks. M8.1 ships nothing publicly — at the end you have:

- A new `jfmt-viewer-core` crate with index, session, pointer, search, and a property-tested round-trip — usable as a library by anyone who wants to build a viewer.
- A new `apps/jfmt-viewer` Tauri 2 + React + Vite app whose seven IPC commands are end-to-end wired, and whose minimal tree UI verifies the contract on small fixtures.
- A `jfmt view` CLI subcommand that prints a placeholder error, reserving the surface area.
- All workspace tests green, clippy `-D warnings` clean, `cargo fmt --all` applied.
- No git tag, no version bump on jfmt or jfmt-cli, no Tauri bundle.

M8.2 will: add virtual scroll, preview pane, search UI, real `jfmt view` binary discovery + spawn, NDJSON top-level rendering, E2E tests, Tauri bundling, cargo-dist coordination, and ship v0.3.0.
