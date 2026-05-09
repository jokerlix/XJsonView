# jfmt M10 — viewer scalability for large JSON arrays

**Status:** approved
**Date:** 2026-05-09
**Owner:** lizhongwei
**Predecessors:** M8 (viewer-core), M9 (search/scope/export)

## Problem

Opening a 300 MB single-array JSON (~800 000 object children at the top level)
in `jfmt-viewer` leaves the left tree pane permanently blank — the first
`get_children(ROOT, 0, 200)` call never returns. The viewer-core has two
quadratic bottlenecks that together produce ~10¹² operations on this input:

1. **`Session::get_children` re-parses the entire container range and
   materializes every direct child into a `Vec<ChildSummary>` before slicing
   the requested window** (`crates/jfmt-viewer-core/src/session.rs:99-203`).
   `offset / limit` saves no work; an array of N children always costs O(N).
2. **`Session::find_container_child` is a linear scan over the whole
   `entries` table for every child container** (`session.rs:282-292`). For an
   array of K container children indexed by N total entries the call sites
   together cost O(K·N) — for our test file ~800 000 × ~3 000 000.

NDJSON files use a different code path (`ndjson_root_children`) and do not
suffer from this. Only the JSON-array case is broken at scale.

## Goals

Targeting **1 GB JSON files with up to ~10 M containers**:

- `get_children(ROOT, 0, 200)` returns in **< 500 ms**.
- `get_children(ROOT, 800_000, 50)` returns in **< 500 ms** (offset-driven
  paging is not slower than head paging).
- Process RSS stays within reach of file size — no buffering an extra full
  copy of the file content in user-space.
- All existing viewer-core / Tauri / E2E tests stay green.

Out of scope:

- Streaming or on-disk indexes for files > 4 GB.
- Compressing `ContainerEntry` to shrink the in-RAM index further.
- New CLI features.
- Frontend changes (the React tree already paginates correctly via
  `getChildren`; no UI work is needed once the backend returns quickly).

## Solution overview

Three coordinated changes in `jfmt-viewer-core`, plus a memory-mapped buffer
to keep RAM bounded as files grow:

1. **CSR parent→children index** built in the same single pass that builds
   `entries`. Replaces the linear `find_container_child` scan with a binary
   search inside the parent's child slice.
2. **True paginated `get_children`**, using a new `EventReader::from_slice_at`
   constructor to skip past any direct child container's byte range without
   parsing it. Window-internal lookups consult CSR for O(1) ID resolution.
3. **`Session.bytes` becomes `memmap2::Mmap`**, with the underlying file
   exclusively locked via `fd-lock` so external mutation cannot corrupt the
   open view.

## Detailed design

### 1. SparseIndex changes

```rust
pub struct SparseIndex {
    pub entries: Vec<ContainerEntry>,
    pub child_offsets: Vec<u32>,   // len = entries.len() + 1, CSR row pointers
    pub child_ids: Vec<NodeId>,    // total child-container count across all parents
    pub root_kind: Option<ContainerKind>,
    pub byte_len: u64,
}
```

`ContainerEntry` gains one field:

```rust
pub direct_child_count: u32,   // direct children including scalars
```

Building both arrays during the existing single-pass scan:

- Each `Frame` keeps a `Vec<NodeId> children` accumulator.
- On `StartObject` / `StartArray`, push the new child's `NodeId` onto the
  parent frame's `children`.
- On `Value` at the parent level, just bump `direct_child_count`.
- On `EndObject` / `EndArray`, set
  `child_offsets[entry.0] = child_ids.len() as u32`,
  extend `child_ids` from the frame's `children`,
  and finalize `entry.direct_child_count` from the running counter.
- After the loop, push a sentinel `child_offsets.push(child_ids.len() as u32)`
  so `child_offsets[i+1]` is always valid.

Children of a parent are appended to `child_ids` in source order, so they are
naturally sorted by `file_offset`. Lookups can use `slice::binary_search_by`
on `entries[id.0].file_offset`.

Memory budget at 10 M containers (file_offset capped at 4 GB by `u32` row
pointers, which is fine since target is 1 GB):

| Structure | Size |
|---|---|
| `entries` (10M × ~64 B) | ~640 MB |
| `child_offsets` (10M × 4 B) | 40 MB |
| `child_ids` (≤10M × 8 B) | ≤80 MB |
| **Total index RAM** | **≲ 760 MB** |

Plus the mmapped file content (not counted against process RSS in the same
way). Acceptable for the 1 GB target. Compressing `ContainerEntry` is left
to a follow-up if needed.

### 2. find_container_child via CSR

```rust
fn find_container_child(&self, parent: NodeId, child_offset: u64) -> Option<NodeId> {
    let lo = self.index.child_offsets[parent.0 as usize] as usize;
    let hi = self.index.child_offsets[parent.0 as usize + 1] as usize;
    let slice = &self.index.child_ids[lo..hi];
    slice
        .binary_search_by(|id| {
            self.index.entries[id.0 as usize]
                .file_offset
                .cmp(&child_offset)
        })
        .ok()
        .map(|i| slice[i])
}
```

O(log K) per lookup, O(1) memory. Behavior is identical to the current
implementation on every existing test input; the change is internal.

### 3. Paginated get_children

The rewritten loop walks the parent's byte range with `EventReader`, but
**every direct child container is fast-skipped** by recreating the reader at
the container's `byte_end` instead of parsing into it. Window-internal
container children resolve their `NodeId` via CSR (no scanning).

```rust
pub fn get_children(&self, parent: NodeId, offset: u32, limit: u32)
    -> Result<GetChildrenResp>
{
    let entry = self.entries.get(parent.0 as usize).ok_or(InvalidNode)?;
    if entry.kind == NdjsonDoc && parent == NodeId::ROOT {
        return self.ndjson_root_children(offset, limit);
    }

    let actual_start = scan_to_open_bracket(&self.bytes, entry);
    let container_end = entry.byte_end as usize;

    let mut reader = EventReader::from_slice_at(&self.bytes, actual_start);
    let csr = self.children_of(parent);   // &[NodeId]
    let mut csr_cursor = 0usize;
    let mut child_idx: u32 = 0;
    let mut pending_key: Option<String> = None;
    let mut next_array_index = 0u32;
    let mut items = Vec::with_capacity(limit as usize);

    // Consume opening bracket
    let _ = reader.next_event()?;

    while items.len() < limit as usize {
        let ev = reader.next_event()?;
        match ev {
            Some(Event::EndObject | Event::EndArray) => break,
            Some(Event::Name(k)) => { pending_key = Some(k); }
            Some(Event::StartObject | Event::StartArray) => {
                let id = csr[csr_cursor]; csr_cursor += 1;
                let child_entry = &self.entries[id.0 as usize];
                if child_idx >= offset {
                    let key = consume_key(parent.kind, &mut pending_key, &mut next_array_index);
                    items.push(child_summary_for(id, key, child_entry));
                } else {
                    consume_key(parent.kind, &mut pending_key, &mut next_array_index);
                }
                // Skip subtree
                reader = EventReader::from_slice_at(&self.bytes, child_entry.byte_end as usize);
                child_idx += 1;
            }
            Some(Event::Value(scalar)) => {
                if child_idx >= offset {
                    let key = consume_key(parent.kind, &mut pending_key, &mut next_array_index);
                    items.push(scalar_child_summary(key, scalar));
                } else {
                    consume_key(parent.kind, &mut pending_key, &mut next_array_index);
                }
                child_idx += 1;
            }
            None => break,
        }
    }

    Ok(GetChildrenResp {
        items,
        total: entry.direct_child_count,
    })
}
```

Cost analysis on the 800 000-element root array:

- `getChildren(0, 200)`: 200 deep walks (none — they're all single objects, just
  one StartObject + skip) + 0 fast-skips before. ~O(200) events.
- `getChildren(800_000, 50)`: 800 000 fast-skip iterations + 50 captured.
  Each fast-skip is one EventReader event + one Mmap byte-range jump.
  ~O(800k) cheap iterations, well under 500 ms.

Both well within target.

### 4. New EventReader API

`crates/jfmt-core/src/parser.rs` (or wherever `EventReader::new_unlimited`
lives) gains:

```rust
impl<'a> EventReader<'a> {
    /// Construct a reader that begins at `start` bytes into `input`.
    /// Caller must guarantee `start` lies on a valid token boundary
    /// (a `[` / `{` / `]` / `}` / `,` byte, or the first byte of a value).
    ///
    /// `byte_offset()` on the returned reader still reports a position
    /// **relative to the sub-slice** (`&input[start..]`), to match the
    /// existing `EventReader::new_unlimited` contract. Callers that need an
    /// absolute file position add `start` themselves. (No new state is
    /// stashed inside the reader; this constructor is just a shorthand
    /// for `EventReader::new_unlimited(&input[start..])` at the API level,
    /// kept as a separate name to document intent.)
    pub fn from_slice_at(input: &'a [u8], start: usize) -> Self {
        EventReader::new_unlimited(&input[start..])
    }
}
```

Note: the existing get_children code already restarts EventReader on a
sub-slice (`session.rs:127`). The new method just makes that pattern
ergonomic and absolute-offset-aware.

### 5. Mmap + file lock

Add workspace dependencies:

```toml
memmap2 = "0.9"
fd-lock = "4"
```

`Session::open_with_progress` becomes:

```rust
let file = std::fs::OpenOptions::new().read(true).open(&path)
    .map_err(/* NotFound or Io */)?;
let mut lock = fd_lock::RwLock::new(file);
let guard = lock.try_write()
    .map_err(|_| ViewerError::FileLocked(path.display().to_string()))?;
let mmap = unsafe { memmap2::Mmap::map(&*guard)? };
// Hold both `lock` and `mmap` for the Session lifetime.
```

Storage:

```rust
pub struct Session {
    path: PathBuf,
    _lock: fd_lock::RwLock<std::fs::File>,
    bytes: memmap2::Mmap,
    index: SparseIndex,
    format: Format,
}
```

`bytes` derefs to `&[u8]` so all existing slice arithmetic (`&self.bytes[..]`,
`memchr` calls, `serde_json::from_slice`) is unchanged.

`SparseIndex::build_with_progress` keeps its `&[u8]` signature; it gets
called as `SparseIndex::build_with_progress(&mmap, mode, on_progress)`.

### 6. New error variant

```rust
#[derive(Debug, Error)]
pub enum ViewerError {
    /* existing variants */
    #[error("file is in use by another process: {0}")]
    FileLocked(String),
}
```

Mapped to a Tauri command error in `commands.rs::open_file` so the frontend
can show a useful message.

## Testing

Per CLAUDE.md conventions: unit + e2e + big-tests gating.

### Unit (default `cargo test`)

- `index.rs`: build a 3-level fixture (object containing array containing
  object), assert `child_offsets` monotonically non-decreasing,
  `child_ids.len() == sum of all container children`, and that each parent's
  slice contains the right NodeIds in source order.
- `session.rs`: existing get_children tests stay; add one for paginated
  reads on a synthetic 1 000-element array — verify
  `get_children(ROOT, 0, 100)`, `get_children(ROOT, 500, 100)`, and
  `get_children(ROOT, 950, 100)` return correct keys and the total field.
- `session.rs`: opening the same file twice in one process returns
  `FileLocked` on the second `Session::open`.

### Big-tests (gated `--features big-tests`)

- `crates/jfmt-viewer-core/tests/big_array.rs`: synthesize a ~50 MB array
  of small objects in a temp file; assert
  - `Session::open` succeeds
  - `get_children(ROOT, 0, 200)` returns `total = N` and 200 items in
    < 500 ms
  - `get_children(ROOT, N - 100, 50)` returns the last 50 items in < 500 ms

50 MB is enough to expose the O(N²) behavior (the current code takes tens
of seconds at this scale). Local 300 MB / 1 GB testing remains a manual
exercise driven by `scripts/gen_big_json.py`.

### Existing test surface

All 100+ existing viewer-core unit tests, the Tauri command tests, and the
WebdriverIO E2E suite must stay green without modification. The CSR
addition is a pure refactor at the API level.

## Risk and mitigation

| Risk | Mitigation |
|---|---|
| `EventReader::from_slice_at` produces wrong byte offsets | Round-trip property test: parse a fixture once normally, recover every container's `(file_offset, byte_end)`; restart reader at `file_offset` and verify next event is the matching open bracket and `byte_offset()` matches. |
| Mmap on Windows opens with too-permissive sharing | `fd-lock` uses `LOCKFILE_EXCLUSIVE_LOCK` semantics; same-process double open is the test gate. Cross-process is documented but not automatically tested. |
| Frontend tries to load same file twice (multi-window future) | Out of scope for M10. `FileLocked` is surfaced as a normal error. |
| `direct_child_count` overflow on > 4 B-element containers | Hard-cap: if a container's direct children exceed `u32::MAX`, indexing fails with `ViewerError::Parse` and a clear message. The 1 GB target makes this impossible (a 4 B-byte JSON array of 4-byte values is far above target). |
| `fd-lock` interacts badly with antivirus scanners on Windows | Documented limitation; if it bites, fall back to advisory-only opening (no lock). Decision deferred until observed. |

## Milestone breakdown

5 tasks, one commit each, executed under the `jfmt-iterate` skill.

| # | Task | Tests added |
|---|---|---|
| 1 | Add `child_offsets` / `child_ids` / `direct_child_count`; populate in single-pass build | CSR unit tests in `index.rs` |
| 2 | Switch `find_container_child` to CSR binary search | full regression must stay green |
| 3 | Add `EventReader::from_slice_at`; rewrite `get_children` for true pagination | offset-paginated unit tests in `session.rs` |
| 4 | `Vec<u8>` → `Mmap` + `fd-lock`; `FileLocked` error variant | double-open unit test |
| 5 | `big-tests`-gated 50 MB e2e timing test | one new test file |

Tag `v0.5.0` after Task 5 ships.

## Non-goals (deferred to later milestones)

- Compressing `ContainerEntry` to shrink in-RAM index.
- Persisting the index to disk for re-open speedup.
- Streaming index construction (currently still O(file_size) RAM during
  build because `entries` lives in memory).
- Multi-window / multi-session support against the same file.
- Files > 4 GB.
