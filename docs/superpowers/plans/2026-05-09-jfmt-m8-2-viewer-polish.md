# jfmt M8.2 — Viewer Production Polish & v0.3.0 Release Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Take the M8.1 internal viewer (backend + minimal UI) to a public release: virtual scrolling for ≥ 10 GB files, preview pane, JSON Pointer copy, NDJSON top-level virtualization, full search UI with streaming hits, real `jfmt view` CLI integration, E2E test suite, and dual-pipeline release artifacts. Ship as v0.3.0.

**Architecture:** Frontend gets `@tanstack/react-virtual` for the tree and the search hit list, a flexbox split-pane layout with a draggable divider, debounced search input wired to the existing `search` IPC command via the `Channel<SearchEvent>` API, and clipboard-write through `tauri-plugin-clipboard-manager`. Backend extends `SparseIndex::build` with a progress callback so the new `IndexProgress::Scanning { bytes_done, bytes_total }` frames carry real progress; `run_search` emits `Progress` and `Error` variants now too. CLI gets a real `jfmt view` that discovers `jfmt-viewer` (or platform-specific `.app`) and spawns it with the file path. Release wires `tauri build` into the existing GitHub Actions matrix alongside cargo-dist; both attach to the same v0.3.0 GitHub Release.

**Tech Stack (additions vs M8.1):** `@tanstack/react-virtual` 3, `tauri-plugin-clipboard-manager` 2, `tauri-driver` + WebdriverIO for E2E, `cargo-dist` (already configured) + `tauri-action` GitHub Action for installer builds.

**Spec:** `docs/superpowers/specs/2026-05-08-jfmt-m8-viewer-skeleton-design.md`
**Predecessor:** M8.1 internal milestone at commit `ce9b4f2` (rustc 1.85.1, Tauri 2 wired end-to-end with all 7 IPC commands, minimal non-virtualized tree UI).

**Out of scope (defers to M9):** disk-persistent index sidecar, search regex / number leaves / fuzzy, jq filter view, code-signing on Windows / macOS, multi-tab UI, i18n, edit/save, theme switcher UI, cancel-during-load (Esc interrupts get_value), XML viewing.

---

## Task 1: Tauri capabilities + clipboard plugin

**Files:**
- Create: `apps/jfmt-viewer/src-tauri/capabilities/default.json`
- Modify: `apps/jfmt-viewer/src-tauri/Cargo.toml` (add `tauri-plugin-clipboard-manager`)
- Modify: `apps/jfmt-viewer/src-tauri/src/lib.rs` (register clipboard plugin)
- Modify: `apps/jfmt-viewer/package.json` (add `@tauri-apps/plugin-clipboard-manager`)

Tauri 2 enforces explicit capability grants. Without these, the dialog plugin will refuse to open file pickers in production builds.

- [ ] **Step 1: Create capabilities manifest**

Create `apps/jfmt-viewer/src-tauri/capabilities/default.json`:

```json
{
  "$schema": "../gen/schemas/desktop-schema.json",
  "identifier": "default",
  "description": "M8.2 capability set for jfmt-viewer",
  "windows": ["main"],
  "permissions": [
    "core:default",
    "dialog:allow-open",
    "clipboard-manager:allow-write-text"
  ]
}
```

- [ ] **Step 2: Add clipboard-manager dep**

Edit `apps/jfmt-viewer/src-tauri/Cargo.toml`. Append to `[dependencies]`:

```toml
tauri-plugin-clipboard-manager = "2"
```

Edit `apps/jfmt-viewer/package.json`. Add to `dependencies`:

```json
"@tauri-apps/plugin-clipboard-manager": "^2",
```

Run `cd apps/jfmt-viewer && pnpm install` to update the lockfile.

- [ ] **Step 3: Register the plugin in lib.rs**

Edit `apps/jfmt-viewer/src-tauri/src/lib.rs`. Replace the existing builder with:

```rust
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
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

- [ ] **Step 4: Verify build**

```bash
cargo build -p jfmt-viewer-app 2>&1 | tail -3
cd apps/jfmt-viewer && pnpm build && cd ../..
```

Expected: both clean.

- [ ] **Step 5: Commit**

```bash
git add apps/jfmt-viewer/src-tauri/Cargo.toml apps/jfmt-viewer/src-tauri/src/lib.rs apps/jfmt-viewer/src-tauri/capabilities apps/jfmt-viewer/package.json apps/jfmt-viewer/pnpm-lock.yaml
git commit -m "$(cat <<'EOF'
feat(viewer): add capabilities manifest + clipboard plugin

M8.2 starts. Capabilities/default.json grants core, dialog open,
and clipboard write — needed for production bundles where Tauri 2
denies all permissions by default. Clipboard plugin powers Task 4's
JSON Pointer copy.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 2: SparseIndex progress callback

**Files:**
- Modify: `crates/jfmt-viewer-core/src/index.rs`
- Modify: `crates/jfmt-viewer-core/src/ndjson.rs`
- Modify: `crates/jfmt-viewer-core/src/lib.rs`

Backend hook so `open_file` can report real indexing progress instead of one Scanning frame at completion.

- [ ] **Step 1: Append failing test to `index.rs`**

Append inside `mod tests` in `crates/jfmt-viewer-core/src/index.rs`:

```rust
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
```

- [ ] **Step 2: Run; expect FAIL** — `cannot find function 'build_with_progress'`.

- [ ] **Step 3: Add the API**

In `crates/jfmt-viewer-core/src/index.rs`, replace the existing `impl SparseIndex { ... }` block with:

```rust
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
```

In `build_json` add a `mut on_progress: F` parameter (with the same `F: FnMut(u64, u64)` bound) and at the bottom of the event loop, after each event, call:

```rust
            if entries.len() % 1024 == 0 {
                on_progress(reader.byte_offset(), input.len() as u64);
            }
```

(Sample every 1024 containers so the callback isn't a hot path.)

After the `loop {}` exits, just before returning `Ok(SparseIndex { ... })`, call `on_progress(input.len() as u64, input.len() as u64);` so the final 100 % frame is guaranteed.

Update the function signature:

```rust
fn build_json<F: FnMut(u64, u64)>(input: &[u8], mut on_progress: F) -> Result<SparseIndex> {
```

- [ ] **Step 4: Mirror in `ndjson.rs`**

Edit `crates/jfmt-viewer-core/src/ndjson.rs`. Change `build_ndjson` signature to:

```rust
pub(crate) fn build_ndjson<F: FnMut(u64, u64)>(
    input: &[u8],
    mut on_progress: F,
) -> Result<SparseIndex> {
```

Inside the `while start < input.len()` loop, after each iteration's `start = end + 1`, call:

```rust
        if line_no % 256 == 0 {
            on_progress(start as u64, input.len() as u64);
        }
```

Before returning, call the final `on_progress(input.len() as u64, input.len() as u64);`.

- [ ] **Step 5: Update callers**

`Session::open` in `crates/jfmt-viewer-core/src/session.rs` calls `SparseIndex::build` directly — that path stays valid (it routes through `build_with_progress` with a no-op closure). No change needed.

- [ ] **Step 6: Run tests; expect PASS**

```bash
cargo test -p jfmt-viewer-core 2>&1 | tail -5
```
Expected: `test result: ok. 19 passed`. (18 existing + 1 new.)

- [ ] **Step 7: Run clippy** — must be clean.

```bash
cargo clippy -p jfmt-viewer-core --all-targets -- -D warnings 2>&1 | tail -3
```

- [ ] **Step 8: Commit**

```bash
git add crates/jfmt-viewer-core/src/index.rs crates/jfmt-viewer-core/src/ndjson.rs
git commit -m "$(cat <<'EOF'
feat(viewer-core): SparseIndex::build_with_progress callback

JSON path samples every 1024 containers; NDJSON every 256 lines.
Final progress frame at completion is guaranteed. Existing
SparseIndex::build delegates to this with a no-op closure for
back-compat.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 3: Wire incremental progress + search Progress events

**Files:**
- Modify: `apps/jfmt-viewer/src-tauri/src/commands.rs`
- Modify: `crates/jfmt-viewer-core/src/search.rs`

- [ ] **Step 1: Add a search Progress callback to `run_search`**

Edit `crates/jfmt-viewer-core/src/search.rs`. Change `run_search` signature to:

```rust
pub fn run_search<F: FnMut(u64, u64, u32)>(
    session: &Session,
    query: &SearchQuery,
    cancel: &Arc<AtomicBool>,
    mut on_hit: impl FnMut(&SearchHit),
    mut on_progress: F,
) -> Result<SearchSummary> {
```

Inside the event loop, immediately after the `cancel.load(...)` check, add:

```rust
        let now_pos = reader.byte_offset();
        // Sample every ~1MB to avoid IPC flooding.
        if now_pos.saturating_sub(last_progress_at) >= 1_048_576 {
            on_progress(now_pos, total_bytes_len, total);
            last_progress_at = now_pos;
        }
```

Add `let total_bytes_len = bytes.len() as u64; let mut last_progress_at = 0u64;` just before the loop.

After the loop, before the `Ok(SearchSummary { ... })`, add:

```rust
    on_progress(total_bytes_len, total_bytes_len, total);
```

- [ ] **Step 2: Update existing search tests for the new signature**

Edit the four tests in `mod tests` in `crates/jfmt-viewer-core/src/search.rs` — every `run_search(...)` call needs a sixth argument: `|_, _, _| {}` no-op.

Example for `finds_value_match`:

```rust
        let summary = run_search(
            &s,
            &SearchQuery { /* ... */ },
            &cancel,
            |hit| hits.push(hit.clone()),
            |_, _, _| {},
        )
        .unwrap();
```

- [ ] **Step 3: Update Tauri commands.rs to wire the callbacks**

Edit `apps/jfmt-viewer/src-tauri/src/commands.rs`. In `open_file`, replace the post-spawn-blocking lines with:

```rust
    let session = tokio::task::spawn_blocking({
        let path = path.clone();
        let on_progress = on_progress.clone();
        move || {
            jfmt_viewer_core::SparseIndex::build_with_progress(
                &std::fs::read(&path).unwrap_or_default(),
                if jfmt_viewer_core::is_ndjson_path(&path) {
                    jfmt_viewer_core::IndexMode::Ndjson
                } else {
                    jfmt_viewer_core::IndexMode::Json
                },
                |done, total| {
                    let _ = on_progress.send(IndexProgress::Scanning {
                        bytes_done: done,
                        bytes_total: total,
                    });
                },
            )?;
            jfmt_viewer_core::Session::open(&path)
        }
    })
    .await
    .map_err(|e| ViewerError::Io(e.to_string()))??;
```

Wait — that double-builds the index. Simpler: have `Session::open` itself accept a progress callback. Add `Session::open_with_progress` to viewer-core in this same task.

Edit `crates/jfmt-viewer-core/src/session.rs`. Replace `Session::open` body with a delegate:

```rust
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        Self::open_with_progress(path, |_, _| {})
    }

    pub fn open_with_progress<P: AsRef<Path>, F: FnMut(u64, u64)>(
        path: P,
        on_progress: F,
    ) -> Result<Self> {
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
        let index = SparseIndex::build_with_progress(&bytes, mode, on_progress)?;
        Ok(Self {
            path,
            bytes,
            index,
            format,
        })
    }
```

- [ ] **Step 4: Re-do the commands.rs `open_file` with the cleaner API**

```rust
    let on_progress_for_open = on_progress.clone();
    let session = tokio::task::spawn_blocking(move || {
        jfmt_viewer_core::Session::open_with_progress(&path, |done, total| {
            let _ = on_progress_for_open.send(IndexProgress::Scanning {
                bytes_done: done,
                bytes_total: total,
            });
        })
    })
    .await
    .map_err(|e| ViewerError::Io(e.to_string()))??;
```

In `commands::search`, the `run_search` call gets a sixth arg threading the progress event:

```rust
    tokio::task::spawn_blocking(move || {
        let on_event_inner = on_event_clone.clone();
        let result = run_search(
            &session,
            &query,
            &cancel_clone,
            |hit: &SearchHit| {
                let _ = on_event_clone.send(SearchEvent::Hit {
                    node: hit.node.map(|n| n.0),
                    path: hit.path.clone(),
                    matched_in: hit.matched_in,
                    snippet: hit.snippet.clone(),
                });
            },
            |bytes_done, bytes_total, hits_so_far| {
                let _ = on_event_inner.send(SearchEvent::Progress {
                    bytes_done,
                    bytes_total,
                    hits_so_far,
                });
            },
        );
        // ... existing Done/Cancelled/Error handling
    });
```

Remove the `#[allow(dead_code)]` attributes on `IndexProgress` and `SearchEvent` — both `Error` and `Progress` variants are now constructed (Error in IndexProgress on `Ok(Err(_))` path of `spawn_blocking`'s inner Result; Progress in run_search closure).

Actually `IndexProgress::Error` still isn't constructed in the happy path. Wire it:

```rust
    let session = match tokio::task::spawn_blocking(/* ... */).await {
        Ok(Ok(s)) => s,
        Ok(Err(e)) => {
            let _ = on_progress.send(IndexProgress::Error {
                message: e.to_string(),
            });
            return Err(e);
        }
        Err(e) => {
            let msg = e.to_string();
            let _ = on_progress.send(IndexProgress::Error {
                message: msg.clone(),
            });
            return Err(ViewerError::Io(msg));
        }
    };
```

Then drop the `#[allow(dead_code)]` on `IndexProgress`. Drop the one on `SearchEvent` after Progress is constructed in Step 3.

- [ ] **Step 5: Build + test**

```bash
cargo test -p jfmt-viewer-core 2>&1 | tail -5
cargo clippy --workspace --all-targets -- -D warnings 2>&1 | tail -3
cargo build -p jfmt-viewer-app 2>&1 | tail -3
```
Expected: 19 viewer-core tests pass, clippy clean, Tauri builds.

- [ ] **Step 6: Commit**

```bash
git add crates/jfmt-viewer-core apps/jfmt-viewer/src-tauri
git commit -m "$(cat <<'EOF'
feat(viewer): wire incremental progress + search Progress events

Session::open_with_progress threads index progress through the
existing Channel<IndexProgress>. run_search gains an on_progress
callback that emits SearchEvent::Progress every ~1 MB scanned plus
hit count. IndexProgress::Error fires when the spawn_blocking task
returns Err. Removes the M8.1 #[allow(dead_code)] markers that
masked these unused variants.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 4: Virtual scroll for the tree

**Files:**
- Modify: `apps/jfmt-viewer/package.json` (add `@tanstack/react-virtual`)
- Replace: `apps/jfmt-viewer/src/components/Tree.tsx`

- [ ] **Step 1: Add the dep**

Edit `apps/jfmt-viewer/package.json`. Add to `dependencies`:

```json
"@tanstack/react-virtual": "^3"
```

Run `cd apps/jfmt-viewer && pnpm install`.

- [ ] **Step 2: Replace `Tree.tsx`**

Replace `apps/jfmt-viewer/src/components/Tree.tsx`:

```tsx
import { useEffect, useRef, useState } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { ChildSummary, getChildren, NodeId } from "../api";
import { TreeRow } from "./TreeRow";

interface Props {
  sessionId: string;
  rootId: NodeId;
  onSelect?: (node: NodeId | null) => void;
  selectedId?: NodeId | null;
}

interface NodeState {
  loaded: ChildSummary[];
  total: number;
  expanded: boolean;
}

interface FlatRow {
  child: ChildSummary;
  depth: number;
  parentId: NodeId;
}

const PAGE_LIMIT = 200;
const ROW_HEIGHT = 22;

export function Tree({ sessionId, rootId, onSelect, selectedId }: Props) {
  const [byId, setById] = useState<Map<NodeId, NodeState>>(new Map());
  const containerRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      const r = await getChildren(sessionId, rootId, 0, PAGE_LIMIT);
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
    const r = await getChildren(sessionId, id, 0, PAGE_LIMIT);
    const next = new Map(byId);
    next.set(id, { loaded: r.items, total: r.total, expanded: true });
    setById(next);
  }

  async function loadMore(id: NodeId) {
    const cur = byId.get(id);
    if (!cur || cur.loaded.length >= cur.total) return;
    const r = await getChildren(sessionId, id, cur.loaded.length, PAGE_LIMIT);
    const next = new Map(byId);
    next.set(id, {
      ...cur,
      loaded: [...cur.loaded, ...r.items],
    });
    setById(next);
  }

  // Flatten the tree to a linear list for virtualization.
  const rows: FlatRow[] = [];
  function flatten(id: NodeId, depth: number) {
    const state = byId.get(id);
    if (!state) return;
    for (const c of state.loaded) {
      rows.push({ child: c, depth, parentId: id });
      if (c.id !== null && byId.get(c.id)?.expanded) {
        flatten(c.id, depth + 1);
      }
    }
  }
  flatten(rootId, 0);

  const virtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement: () => containerRef.current,
    estimateSize: () => ROW_HEIGHT,
    overscan: 20,
  });

  // Trigger load-more when scrolled near the bottom of any partially loaded
  // container. Run once per render (cheap).
  useEffect(() => {
    for (const [id, state] of byId) {
      if (state.loaded.length < state.total) {
        // Find this container's first visible row in the flat list and
        // check if the visible window is within 50 rows of its end.
        const lastIdx = rows.findLastIndex((r) => r.parentId === id);
        if (lastIdx === -1) continue;
        const visibleEnd =
          (virtualizer.getVirtualItems().at(-1)?.index ?? 0) + 1;
        if (visibleEnd >= lastIdx - 50) {
          loadMore(id);
        }
      }
    }
  });

  return (
    <div
      ref={containerRef}
      style={{
        height: "100%",
        overflow: "auto",
        contain: "strict",
      }}
    >
      <div
        style={{
          height: virtualizer.getTotalSize(),
          width: "100%",
          position: "relative",
        }}
      >
        {virtualizer.getVirtualItems().map((vi) => {
          const row = rows[vi.index];
          const expanded = row.child.id !== null && (byId.get(row.child.id)?.expanded ?? false);
          const selected = selectedId !== undefined && row.child.id === selectedId;
          return (
            <div
              key={vi.key}
              style={{
                position: "absolute",
                top: 0,
                left: 0,
                width: "100%",
                transform: `translateY(${vi.start}px)`,
                background: selected ? "#cce4ff" : "transparent",
              }}
            >
              <TreeRow
                child={row.child}
                depth={row.depth}
                expanded={expanded}
                onToggle={() => row.child.id !== null && toggle(row.child.id)}
                onSelect={() => onSelect?.(row.child.id)}
              />
            </div>
          );
        })}
      </div>
    </div>
  );
}
```

- [ ] **Step 3: Update `TreeRow.tsx` to accept `onSelect`**

Edit `apps/jfmt-viewer/src/components/TreeRow.tsx`. Replace the file:

```tsx
import { ChildSummary } from "../api";

interface Props {
  child: ChildSummary;
  depth: number;
  expanded: boolean;
  onToggle: () => void;
  onSelect: () => void;
}

export function TreeRow({ child, depth, expanded, onToggle, onSelect }: Props) {
  const isContainer = child.id !== null;
  const chevron = isContainer ? (expanded ? "▾" : "▸") : "•";
  const sizeHint = isContainer ? `[${child.child_count}]` : (child.preview ?? "");

  function handleClick(e: React.MouseEvent) {
    onSelect();
    if (isContainer && (e.target as HTMLElement).tagName === "SPAN") {
      // Plain row click selects only; chevron click toggles.
      // We let the bubbling proceed to the toggle handler.
    }
  }

  return (
    <div
      style={{
        height: 22,
        paddingLeft: depth * 16,
        cursor: "pointer",
        whiteSpace: "nowrap",
        fontFamily: "ui-monospace, monospace",
        fontSize: 13,
        userSelect: "none",
      }}
      onClick={handleClick}
    >
      <span
        style={{ width: 14, display: "inline-block" }}
        onClick={(e) => {
          if (isContainer) {
            e.stopPropagation();
            onToggle();
          }
        }}
      >
        {chevron}
      </span>
      <span style={{ color: "#888" }}> {child.kind}</span>{" "}
      <strong>{child.key}</strong>{" "}
      <span style={{ color: "#444" }}>{sizeHint}</span>
    </div>
  );
}
```

- [ ] **Step 4: Build the frontend**

```bash
cd apps/jfmt-viewer && pnpm build && cd ../..
```
Expected: `vite build` clean. Verify `dist/index.html` exists.

- [ ] **Step 5: Commit**

```bash
git add apps/jfmt-viewer
git commit -m "$(cat <<'EOF'
feat(viewer): virtual scroll for tree via TanStack Virtual

Tree flattens to a linear FlatRow list and the visible window is
rendered through useVirtualizer. Auto-pages additional children as
the user scrolls within 50 rows of a partially-loaded container's
end. Row click selects (propagates onSelect upward); chevron click
toggles expansion via stopPropagation.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 5: Preview pane

**Files:**
- Create: `apps/jfmt-viewer/src/components/Preview.tsx`
- Modify: `apps/jfmt-viewer/src/App.tsx`

- [ ] **Step 1: Create Preview.tsx**

```tsx
import { useEffect, useState } from "react";
import { getValue, NodeId } from "../api";

interface Props {
  sessionId: string;
  node: NodeId | null;
}

export function Preview({ sessionId, node }: Props) {
  const [json, setJson] = useState<string>("");
  const [truncated, setTruncated] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  useEffect(() => {
    if (node === null) {
      setJson("");
      setTruncated(false);
      setErr(null);
      return;
    }
    let cancelled = false;
    setLoading(true);
    setErr(null);
    getValue(sessionId, node)
      .then((r) => {
        if (cancelled) return;
        setJson(r.json);
        setTruncated(r.truncated);
        setLoading(false);
      })
      .catch((e) => {
        if (cancelled) return;
        setErr(String(e));
        setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [sessionId, node]);

  if (node === null) {
    return (
      <div style={{ padding: 16, color: "#999", fontStyle: "italic" }}>
        Select a node in the tree to preview.
      </div>
    );
  }
  if (loading) {
    return <div style={{ padding: 16, color: "#666" }}>Loading…</div>;
  }
  if (err) {
    return (
      <div style={{ padding: 16, color: "#c00" }}>
        Error: {err}
      </div>
    );
  }
  return (
    <pre
      style={{
        margin: 0,
        padding: 16,
        fontFamily: "ui-monospace, monospace",
        fontSize: 12,
        whiteSpace: "pre",
        overflow: "auto",
        height: "100%",
      }}
    >
      {json}
      {truncated && (
        <span style={{ color: "#a60", fontStyle: "italic" }}>
          {"\n(see truncation marker above; full export ships in M9)"}
        </span>
      )}
    </pre>
  );
}
```

- [ ] **Step 2: Wire into App.tsx with a flexbox split**

Replace `apps/jfmt-viewer/src/App.tsx`:

```tsx
import { useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { closeFile, NodeId, openFile } from "./api";
import { Tree } from "./components/Tree";
import { Preview } from "./components/Preview";

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
  const [selected, setSelected] = useState<NodeId | null>(null);

  async function pickFile() {
    const picked = await open({
      multiple: false,
      filters: [{ name: "JSON", extensions: ["json", "ndjson", "jsonl"] }],
    });
    if (!picked || Array.isArray(picked)) return;
    if (session) await closeFile(session.sessionId);
    setProgress("opening…");
    setSelected(null);
    const resp = await openFile(picked, (p) => {
      if (p.phase === "scanning") {
        const pct = ((p.bytes_done / Math.max(1, p.bytes_total)) * 100).toFixed(0);
        setProgress(`scanning: ${pct}%`);
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
    <main
      style={{
        fontFamily: "system-ui",
        height: "100vh",
        display: "flex",
        flexDirection: "column",
      }}
    >
      <header style={{ padding: 8, borderBottom: "1px solid #ddd" }}>
        <button onClick={pickFile}>📁 Open</button>{" "}
        <span style={{ color: "#666" }}>{progress}</span>
        {session && (
          <span style={{ marginLeft: 16, color: "#444", fontSize: 12 }}>
            {session.path} · {session.format} · {session.totalBytes} bytes
          </span>
        )}
      </header>
      {session && (
        <div style={{ flex: 1, display: "flex", overflow: "hidden" }}>
          <div style={{ flex: "0 0 40%", borderRight: "1px solid #ddd" }}>
            <Tree
              sessionId={session.sessionId}
              rootId={session.rootId}
              onSelect={setSelected}
              selectedId={selected}
            />
          </div>
          <div style={{ flex: 1, overflow: "hidden" }}>
            <Preview sessionId={session.sessionId} node={selected} />
          </div>
        </div>
      )}
    </main>
  );
}
```

- [ ] **Step 3: Build**

```bash
cd apps/jfmt-viewer && pnpm build && cd ../..
```
Expected: clean.

- [ ] **Step 4: Commit**

```bash
git add apps/jfmt-viewer/src
git commit -m "$(cat <<'EOF'
feat(viewer): preview pane + 40/60 split layout

Right pane shows the selected node's pretty-printed JSON via
get_value. Truncated subtrees flag the M9 export hook. Layout
becomes header + flex row with the tree on the left (40%) and
preview on the right (60%); both panes overflow independently.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 6: JSON Pointer copy UI

**Files:**
- Modify: `apps/jfmt-viewer/src/App.tsx`
- Create: `apps/jfmt-viewer/src/lib/clipboard.ts`

- [ ] **Step 1: Clipboard helper**

Create `apps/jfmt-viewer/src/lib/clipboard.ts`:

```ts
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { getPointer, NodeId } from "../api";

export async function copyPointer(sessionId: string, node: NodeId): Promise<string> {
  const pointer = await getPointer(sessionId, node);
  await writeText(pointer);
  return pointer;
}
```

- [ ] **Step 2: Add a Copy button + Ctrl+C handler in App.tsx**

In `App.tsx`, append to the imports:

```tsx
import { useEffect } from "react";
import { copyPointer } from "./lib/clipboard";
```

Inside the `App` component (after `setSelected` declaration), add:

```tsx
  const [pointerHint, setPointerHint] = useState<string>("");

  async function copyCurrentPointer() {
    if (!session || selected === null) return;
    const p = await copyPointer(session.sessionId, selected);
    setPointerHint(`copied: ${p || "(root)"}`);
    setTimeout(() => setPointerHint(""), 2000);
  }

  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      const isCopy = (e.ctrlKey || e.metaKey) && e.key === "c";
      if (isCopy && session && selected !== null) {
        // Don't intercept text selection in <pre>; only fire when no
        // text selection exists.
        const sel = window.getSelection();
        if (sel && sel.toString().length === 0) {
          e.preventDefault();
          copyCurrentPointer();
        }
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [session, selected]);
```

In the header JSX, insert next to the Open button:

```tsx
        {session && selected !== null && (
          <button
            onClick={copyCurrentPointer}
            title="Copy JSON Pointer (Ctrl+C with no text selected)"
            style={{ marginLeft: 8 }}
          >
            📋 Copy ptr
          </button>
        )}
        {pointerHint && (
          <span style={{ marginLeft: 8, color: "#080", fontSize: 12 }}>
            {pointerHint}
          </span>
        )}
```

- [ ] **Step 3: Build + manual smoke**

```bash
cd apps/jfmt-viewer && pnpm build && cd ../..
```
Expected: clean. Manual smoke is `pnpm tauri dev`, open `crates/jfmt-viewer-core/tests/fixtures/small.json`, click `users` → `1` → `name`, click "📋 Copy ptr", paste into terminal: should be `/users/1/name`.

- [ ] **Step 4: Commit**

```bash
git add apps/jfmt-viewer/src
git commit -m "$(cat <<'EOF'
feat(viewer): JSON Pointer copy UI

Toolbar Copy button writes the selected node's RFC 6901 pointer to
the system clipboard via tauri-plugin-clipboard-manager. Ctrl+C
with no active text selection triggers the same. Pointer hint
flashes for 2 seconds confirming the copy.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 7: NDJSON top-level virtualized rendering

**Files:**
- Modify: `apps/jfmt-viewer/src/components/Tree.tsx`
- Modify: `apps/jfmt-viewer/src/components/TreeRow.tsx`

NDJSON's synthetic root may have millions of doc children. The Tree component already uses TanStack Virtual but it loads only PAGE_LIMIT=200 children per call, then auto-pages. For NDJSON the root's `total` can be 10M+ — TanStack handles 10M virtual rows fine, but we must NEVER call `getChildren` with `limit=10_000_000`. The PAGE_LIMIT cap already covers this; verify and add a small visual differentiation.

- [ ] **Step 1: Add a test fixture**

Create `crates/jfmt-viewer-core/tests/fixtures/large-ndjson.ndjson` programmatically:

```bash
python -c "import json; print('\n'.join(json.dumps({'i': i, 'k': 'v' + str(i)}) for i in range(50000)))" > crates/jfmt-viewer-core/tests/fixtures/large-ndjson.ndjson
```

(50 K lines — fits in the test fixture set without bloating the repo, since `.gitignore` already excludes `target/`. Actually this fixture WILL be committed; size is ~1.4 MB, large but tolerable. Add to `.gitattributes` with `binary` if line-ending normalization causes issues; otherwise leave as-is.)

- [ ] **Step 2: Add a viewer-core unit test on the large fixture**

Append inside `mod tests` in `crates/jfmt-viewer-core/src/ndjson.rs`:

```rust
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
        // Generous bound — CI on slow runners. Fail loud if the indexer
        // regresses by 10x.
        assert!(
            elapsed < std::time::Duration::from_secs(5),
            "indexed in {elapsed:?}"
        );
    }
```

- [ ] **Step 3: Visual differentiation in TreeRow.tsx**

Edit the existing `TreeRow.tsx`. In the `kind` rendering, special-case `ndjson_doc`:

```tsx
      <span style={{ color: "#888" }}>
        {child.kind === "ndjson_doc" ? "doc" : child.kind}
      </span>
```

This makes the NDJSON root's children read as `doc 0`, `doc 1`, ... rather than `ndjson_doc 0`.

Wait — `kind` for an NDJSON root's children is the kind of the line content (Object, Array, etc.), NOT `ndjson_doc`. The synthetic root is `ndjson_doc`; its children inherit the line's kind. So this branch only ever fires on the root's badge. Apply anyway since it's harmless and clarifies the root row.

- [ ] **Step 4: Run tests + build**

```bash
cargo test -p jfmt-viewer-core 2>&1 | tail -3
cd apps/jfmt-viewer && pnpm build && cd ../..
```
Expected: 20 viewer-core tests pass; vite clean.

- [ ] **Step 5: Commit**

```bash
git add crates/jfmt-viewer-core/tests/fixtures/large-ndjson.ndjson crates/jfmt-viewer-core/src/ndjson.rs apps/jfmt-viewer/src/components/TreeRow.tsx
git commit -m "$(cat <<'EOF'
feat(viewer): NDJSON top-level renders virtualized via existing Tree

The Tree component's TanStack Virtual + paginated getChildren
already handles million-row roots; this task adds a 50 K-line
fixture and a perf-floor unit test (< 5s on CI to index), and
renders ndjson_doc badges as "doc" for clarity.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 8: Search toolbar + IPC plumbing

**Files:**
- Create: `apps/jfmt-viewer/src/components/SearchBar.tsx`
- Create: `apps/jfmt-viewer/src/lib/searchState.ts`
- Modify: `apps/jfmt-viewer/src/App.tsx`

- [ ] **Step 1: Search state hook**

Create `apps/jfmt-viewer/src/lib/searchState.ts`:

```ts
import { useEffect, useRef, useState } from "react";
import { cancelSearch, search, SearchEvent, SearchQuery } from "../api";

export interface Hit {
  node: number | null;
  path: string;
  matched_in: "key" | "value";
  snippet: string;
}

export interface SearchState {
  query: SearchQuery;
  hits: Hit[];
  totalSoFar: number;
  scanning: boolean;
  cancelled: boolean;
  error: string | null;
  hitCap: boolean;
}

const HIT_CAP = 1000;

export function useSearch(sessionId: string | null) {
  const [state, setState] = useState<SearchState>({
    query: { needle: "", case_sensitive: false, scope: "both" },
    hits: [],
    totalSoFar: 0,
    scanning: false,
    cancelled: false,
    error: null,
    hitCap: false,
  });
  const handleRef = useRef<string | null>(null);

  function reset() {
    setState((s) => ({
      ...s,
      hits: [],
      totalSoFar: 0,
      scanning: false,
      cancelled: false,
      error: null,
      hitCap: false,
    }));
  }

  async function start(query: SearchQuery) {
    if (!sessionId) return;
    if (handleRef.current) {
      await cancelSearch(handleRef.current);
      handleRef.current = null;
    }
    setState({
      query,
      hits: [],
      totalSoFar: 0,
      scanning: true,
      cancelled: false,
      error: null,
      hitCap: false,
    });
    if (!query.needle.trim()) {
      setState((s) => ({ ...s, scanning: false }));
      return;
    }
    const handle = await search(sessionId, query, (e: SearchEvent) => {
      setState((prev) => {
        if (e.kind === "hit") {
          if (prev.hits.length >= HIT_CAP) {
            return { ...prev, totalSoFar: prev.totalSoFar + 1, hitCap: true };
          }
          return {
            ...prev,
            hits: [...prev.hits, e],
            totalSoFar: prev.totalSoFar + 1,
          };
        }
        if (e.kind === "progress") {
          return { ...prev, totalSoFar: e.hits_so_far };
        }
        if (e.kind === "done") {
          return { ...prev, scanning: false };
        }
        if (e.kind === "cancelled") {
          return { ...prev, scanning: false, cancelled: true };
        }
        if (e.kind === "error") {
          return { ...prev, scanning: false, error: e.message };
        }
        return prev;
      });
    });
    handleRef.current = handle.id;
  }

  async function cancel() {
    if (handleRef.current) {
      await cancelSearch(handleRef.current);
      handleRef.current = null;
    }
  }

  useEffect(() => {
    return () => {
      // Cancel on unmount.
      cancel();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return { state, start, cancel, reset };
}
```

- [ ] **Step 2: Search bar component**

Create `apps/jfmt-viewer/src/components/SearchBar.tsx`:

```tsx
import { useEffect, useRef, useState } from "react";
import { SearchQuery } from "../api";
import { SearchState } from "../lib/searchState";

interface Props {
  onQuery: (q: SearchQuery) => void;
  onCancel: () => void;
  state: SearchState;
  cursor: number; // index into state.hits
  onCursorChange: (next: number) => void;
}

const DEBOUNCE_MS = 250;

export function SearchBar({ onQuery, onCancel, state, cursor, onCursorChange }: Props) {
  const [needle, setNeedle] = useState("");
  const [caseSensitive, setCaseSensitive] = useState(false);
  const [scope, setScope] = useState<SearchQuery["scope"]>("both");
  const tRef = useRef<number | null>(null);
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (tRef.current !== null) clearTimeout(tRef.current);
    tRef.current = window.setTimeout(() => {
      if (needle.trim() === "") {
        onCancel();
      } else {
        onQuery({ needle, case_sensitive: caseSensitive, scope });
      }
    }, DEBOUNCE_MS);
    return () => {
      if (tRef.current !== null) clearTimeout(tRef.current);
    };
  }, [needle, caseSensitive, scope, onQuery, onCancel]);

  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if ((e.ctrlKey || e.metaKey) && e.key === "f") {
        e.preventDefault();
        inputRef.current?.focus();
        inputRef.current?.select();
        return;
      }
      if (state.hits.length === 0) return;
      if (e.key === "F3" && !e.shiftKey) {
        e.preventDefault();
        onCursorChange((cursor + 1) % state.hits.length);
      } else if (e.key === "F3" && e.shiftKey) {
        e.preventDefault();
        onCursorChange((cursor - 1 + state.hits.length) % state.hits.length);
      } else if (e.key === "Escape" && document.activeElement === inputRef.current) {
        setNeedle("");
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [cursor, state.hits.length, onCursorChange]);

  const counter = state.scanning
    ? `${cursor + (state.hits.length > 0 ? 1 : 0)}/${state.totalSoFar}+`
    : state.hits.length > 0
      ? `${cursor + 1}/${state.hits.length}`
      : state.totalSoFar > 0
        ? "(no results)"
        : "";

  return (
    <span style={{ display: "inline-flex", alignItems: "center", gap: 4 }}>
      <input
        ref={inputRef}
        value={needle}
        onChange={(e) => setNeedle(e.target.value)}
        placeholder="🔍 search"
        style={{
          width: 200,
          padding: "2px 6px",
          fontFamily: "ui-monospace, monospace",
          fontSize: 12,
        }}
      />
      <button
        onClick={() => setCaseSensitive((b) => !b)}
        title="Case sensitive"
        style={{ fontWeight: caseSensitive ? "bold" : "normal" }}
      >
        Aa
      </button>
      <select
        value={scope}
        onChange={(e) => setScope(e.target.value as SearchQuery["scope"])}
      >
        <option value="both">both</option>
        <option value="keys">keys</option>
        <option value="values">values</option>
      </select>
      <span style={{ color: "#666", fontSize: 12, minWidth: 60 }}>
        {counter}
      </span>
      {state.hits.length > 0 && (
        <>
          <button
            onClick={() => onCursorChange((cursor - 1 + state.hits.length) % state.hits.length)}
            title="Previous (Shift+F3)"
          >
            ↑
          </button>
          <button
            onClick={() => onCursorChange((cursor + 1) % state.hits.length)}
            title="Next (F3)"
          >
            ↓
          </button>
        </>
      )}
      {needle && (
        <button onClick={() => setNeedle("")} title="Clear (Esc)">
          ✕
        </button>
      )}
    </span>
  );
}
```

- [ ] **Step 3: Wire SearchBar into App.tsx**

Edit `apps/jfmt-viewer/src/App.tsx`. Append to imports:

```tsx
import { SearchBar } from "./components/SearchBar";
import { useSearch } from "./lib/searchState";
```

Inside the App component, add:

```tsx
  const sessionId = session?.sessionId ?? null;
  const { state: searchState, start: startSearch, cancel: cancelSearchOp } = useSearch(sessionId);
  const [searchCursor, setSearchCursor] = useState(0);

  useEffect(() => {
    setSearchCursor(0);
  }, [searchState.query.needle, searchState.query.scope, searchState.query.case_sensitive]);
```

In the header, append next to the existing Copy button:

```tsx
        {session && (
          <span style={{ marginLeft: 16 }}>
            <SearchBar
              state={searchState}
              cursor={searchCursor}
              onCursorChange={setSearchCursor}
              onQuery={startSearch}
              onCancel={cancelSearchOp}
            />
          </span>
        )}
```

- [ ] **Step 4: Build**

```bash
cd apps/jfmt-viewer && pnpm build && cd ../..
```
Expected: clean.

- [ ] **Step 5: Commit**

```bash
git add apps/jfmt-viewer/src
git commit -m "$(cat <<'EOF'
feat(viewer): search toolbar with debounced query + nav

Toolbar input debounces 250ms then issues a search command via the
useSearch hook. Aa toggles case sensitivity; scope dropdown picks
keys / values / both. Counter reads "N/M+" while scanning, "N/M"
once done. Ctrl+F focuses; F3 / Shift+F3 cycle hits; Esc clears
the box. Hit-list display lands in Task 9.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 9: Hit-list panel + jump-to-hit

**Files:**
- Create: `apps/jfmt-viewer/src/components/HitList.tsx`
- Modify: `apps/jfmt-viewer/src/App.tsx`
- Modify: `apps/jfmt-viewer/src/components/Tree.tsx` (expose `expandToPath` API)

Jumping to a hit requires expanding the path from root → target. The Tree owns `byId` state so it must expose an imperative handle.

- [ ] **Step 1: Hit list with virtualization**

Create `apps/jfmt-viewer/src/components/HitList.tsx`:

```tsx
import { useRef } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { Hit, SearchState } from "../lib/searchState";

interface Props {
  state: SearchState;
  cursor: number;
  onPick: (idx: number) => void;
}

export function HitList({ state, cursor, onPick }: Props) {
  const containerRef = useRef<HTMLDivElement>(null);
  const v = useVirtualizer({
    count: state.hits.length,
    getScrollElement: () => containerRef.current,
    estimateSize: () => 22,
    overscan: 10,
  });

  if (state.hits.length === 0 && !state.scanning) return null;

  return (
    <div
      style={{
        flex: "0 0 240px",
        borderRight: "1px solid #ddd",
        display: "flex",
        flexDirection: "column",
        overflow: "hidden",
      }}
    >
      <div style={{ padding: "4px 8px", fontSize: 11, color: "#666", borderBottom: "1px solid #eee" }}>
        {state.scanning
          ? `Scanning… ${state.totalSoFar} hits`
          : `${state.hits.length} hits${state.hitCap ? " (1000+ — refine query)" : ""}`}
        {state.error && <span style={{ color: "#c00" }}> · {state.error}</span>}
      </div>
      <div ref={containerRef} style={{ flex: 1, overflow: "auto" }}>
        <div style={{ height: v.getTotalSize(), position: "relative" }}>
          {v.getVirtualItems().map((vi) => (
            <HitRow
              key={vi.key}
              hit={state.hits[vi.index]}
              top={vi.start}
              selected={vi.index === cursor}
              onClick={() => onPick(vi.index)}
            />
          ))}
        </div>
      </div>
    </div>
  );
}

function HitRow({
  hit,
  top,
  selected,
  onClick,
}: {
  hit: Hit;
  top: number;
  selected: boolean;
  onClick: () => void;
}) {
  return (
    <div
      onClick={onClick}
      style={{
        position: "absolute",
        top: 0,
        left: 0,
        right: 0,
        height: 22,
        transform: `translateY(${top}px)`,
        background: selected ? "#cce4ff" : "transparent",
        cursor: "pointer",
        padding: "2px 8px",
        fontFamily: "ui-monospace, monospace",
        fontSize: 11,
        whiteSpace: "nowrap",
        overflow: "hidden",
        textOverflow: "ellipsis",
      }}
    >
      <span style={{ color: hit.matched_in === "key" ? "#06a" : "#a60" }}>
        {hit.matched_in === "key" ? "K" : "V"}{" "}
      </span>
      <span style={{ color: "#444" }}>{hit.path}</span>{" "}
      <span style={{ color: "#888" }} dangerouslySetInnerHTML={{ __html: renderSnippet(hit.snippet) }} />
    </div>
  );
}

function renderSnippet(s: string): string {
  // Convert **match** markers to <strong>match</strong>.
  return s
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/\*\*(.+?)\*\*/g, "<strong>$1</strong>");
}
```

- [ ] **Step 2: Add a Tree imperative handle for jump-to-path**

Edit `apps/jfmt-viewer/src/components/Tree.tsx`. Wrap the `Tree` function with `forwardRef` and expose:

```tsx
import { forwardRef, useImperativeHandle, useEffect, useRef, useState } from "react";

export interface TreeHandle {
  expandToPointer(pointer: string): Promise<NodeId | null>;
}

interface Props { /* same as before */ }

export const Tree = forwardRef<TreeHandle, Props>(function Tree(
  { sessionId, rootId, onSelect, selectedId },
  ref,
) {
  // ... existing body ...

  useImperativeHandle(ref, () => ({
    async expandToPointer(pointer: string) {
      // Decode RFC 6901 segments.
      if (pointer === "") return rootId;
      const segs = pointer
        .split("/")
        .slice(1)
        .map((s) => s.replace(/~1/g, "/").replace(/~0/g, "~"));
      let cur: NodeId = rootId;
      for (const seg of segs) {
        // Make sure cur is loaded + expanded.
        if (!byId.get(cur)) {
          const r = await getChildren(sessionId, cur, 0, PAGE_LIMIT);
          setById((m) => {
            const next = new Map(m);
            next.set(cur, { loaded: r.items, total: r.total, expanded: true });
            return next;
          });
          // Wait one tick for state to flush.
          await new Promise((r) => setTimeout(r, 0));
        }
        const cs = byId.get(cur)?.loaded ?? [];
        const child = cs.find((c) => c.key === seg);
        if (!child || child.id === null) return null;
        cur = child.id;
      }
      onSelect?.(cur);
      return cur;
    },
  }));

  // ... rest of component
});
```

(Keep the existing `flatten`, `useVirtualizer`, JSX. The `useImperativeHandle` slot just goes after `setById` and before `flatten`.)

- [ ] **Step 3: Wire HitList + Tree handle in App.tsx**

Edit `apps/jfmt-viewer/src/App.tsx`. Add imports:

```tsx
import { useRef } from "react";
import { HitList } from "./components/HitList";
import { Tree, TreeHandle } from "./components/Tree";
```

Add state:

```tsx
  const treeRef = useRef<TreeHandle>(null);

  async function jumpToHit(idx: number) {
    setSearchCursor(idx);
    const hit = searchState.hits[idx];
    if (!hit) return;
    const id = await treeRef.current?.expandToPointer(hit.path);
    if (id !== null && id !== undefined) {
      setSelected(id);
    }
  }

  useEffect(() => {
    if (searchState.hits.length > 0) {
      jumpToHit(searchCursor);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [searchCursor]);
```

Update JSX to include the HitList between the header and the tree:

```tsx
      {session && (
        <div style={{ flex: 1, display: "flex", overflow: "hidden" }}>
          <HitList state={searchState} cursor={searchCursor} onPick={jumpToHit} />
          <div style={{ flex: "0 0 40%", borderRight: "1px solid #ddd" }}>
            <Tree
              ref={treeRef}
              sessionId={session.sessionId}
              rootId={session.rootId}
              onSelect={setSelected}
              selectedId={selected}
            />
          </div>
          <div style={{ flex: 1, overflow: "hidden" }}>
            <Preview sessionId={session.sessionId} node={selected} />
          </div>
        </div>
      )}
```

- [ ] **Step 4: Build + smoke**

```bash
cd apps/jfmt-viewer && pnpm build && cd ../..
```
Expected: clean. Manual smoke: open `small.json`, search "Alice" — hit list shows one row, click it → tree expands to `users/0/name`.

- [ ] **Step 5: Commit**

```bash
git add apps/jfmt-viewer/src
git commit -m "$(cat <<'EOF'
feat(viewer): hit list panel + jump-to-hit

Left-most pane shows hits while scanning + after done. Each row is
"K/V path snippet" with the matched span bolded; click a row (or
F3) jumps the tree to that pointer, which Tree's exposed
expandToPointer handle decodes RFC 6901 segments and walks node by
node, fetching children as needed.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 10: Real `jfmt view` binary discovery

**Files:**
- Replace: `crates/jfmt-cli/src/commands/view.rs`
- Modify: `crates/jfmt-cli/tests/cli_view_placeholder.rs` (rename + rewrite)

- [ ] **Step 1: Failing test**

Move `crates/jfmt-cli/tests/cli_view_placeholder.rs` → `crates/jfmt-cli/tests/cli_view.rs`. Replace contents:

```rust
use assert_cmd::Command;
use predicates::prelude::*;

#[test]
fn view_help_lists_subcommand() {
    let mut cmd = Command::cargo_bin("jfmt").unwrap();
    cmd.args(["--help"]);
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("view"));
}

#[test]
fn view_with_missing_file_errors_clearly() {
    let mut cmd = Command::cargo_bin("jfmt").unwrap();
    cmd.args(["view", "definitely-does-not-exist.json"]);
    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("definitely-does-not-exist.json"));
}

#[test]
fn view_with_existing_file_attempts_to_spawn() {
    // We can't actually verify the GUI launches in a unit test, but we can
    // verify the command resolves the binary and produces a "not found"
    // diagnostic mentioning the search paths it checked.
    let mut cmd = Command::cargo_bin("jfmt").unwrap();
    cmd.env_remove("PATH"); // force the "binary not found" path
    cmd.args(["view", "Cargo.toml"]); // file exists; path is real
    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("jfmt-viewer").and(predicate::str::contains("could not find")));
}
```

- [ ] **Step 2: Run; expect FAIL (placeholder still in place)**

```bash
cargo test -p jfmt-cli --test cli_view 2>&1 | tail -10
```
Expected: failure ("GUI viewer not yet bundled" predicate not matched against new tests).

- [ ] **Step 3: Implement**

Replace `crates/jfmt-cli/src/commands/view.rs`:

```rust
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};

const BINARY_NAME: &str = if cfg!(windows) { "jfmt-viewer.exe" } else { "jfmt-viewer" };

pub fn run<P: AsRef<Path>>(file: P) -> Result<()> {
    let file = file.as_ref();
    if !file.exists() {
        return Err(anyhow!(
            "file not found: {} — did you mean a different path?",
            file.display()
        ));
    }
    let abs = std::fs::canonicalize(file)
        .with_context(|| format!("canonicalize {}", file.display()))?;

    let viewer = locate_viewer().ok_or_else(|| {
        anyhow!(
            "could not find {BINARY_NAME} on PATH or next to jfmt — install the GUI \
             from https://github.com/jokerlix/XJsonView/releases"
        )
    })?;

    let status = std::process::Command::new(&viewer)
        .arg(&abs)
        .status()
        .with_context(|| format!("spawn {}", viewer.display()))?;
    if !status.success() {
        return Err(anyhow!(
            "{} exited with status {}",
            viewer.display(),
            status
        ));
    }
    Ok(())
}

/// Search order:
/// 1. Same directory as the current `jfmt` executable.
/// 2. PATH lookup.
/// 3. macOS only: /Applications/jfmt-viewer.app/Contents/MacOS/jfmt-viewer.
fn locate_viewer() -> Option<PathBuf> {
    if let Ok(jfmt_self) = std::env::current_exe() {
        if let Some(dir) = jfmt_self.parent() {
            let candidate = dir.join(BINARY_NAME);
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }
    if let Ok(path) = which(BINARY_NAME) {
        return Some(path);
    }
    #[cfg(target_os = "macos")]
    {
        let app = PathBuf::from("/Applications/jfmt-viewer.app/Contents/MacOS/jfmt-viewer");
        if app.exists() {
            return Some(app);
        }
    }
    None
}

fn which(name: &str) -> std::io::Result<PathBuf> {
    let path = std::env::var_os("PATH").ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::NotFound, "PATH not set")
    })?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    Err(std::io::Error::new(std::io::ErrorKind::NotFound, name.to_string()))
}
```

- [ ] **Step 4: Run tests; expect PASS**

```bash
cargo test -p jfmt-cli --test cli_view 2>&1 | tail -10
```
Expected: 3 passed.

- [ ] **Step 5: Run clippy** — clean.

- [ ] **Step 6: Commit**

```bash
git add crates/jfmt-cli/src/commands/view.rs crates/jfmt-cli/tests/cli_view_placeholder.rs crates/jfmt-cli/tests/cli_view.rs
git commit -m "$(cat <<'EOF'
feat(cli): real jfmt view binary discovery + spawn

Looks for jfmt-viewer in (1) the directory of the current jfmt
executable, (2) PATH, (3) /Applications/jfmt-viewer.app on macOS.
Spawns it with the canonicalized file path; propagates exit status.
Replaces the M8.1 placeholder.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 11: E2E suite (Linux only)

**Files:**
- Create: `apps/jfmt-viewer/e2e/package.json`
- Create: `apps/jfmt-viewer/e2e/tsconfig.json`
- Create: `apps/jfmt-viewer/e2e/wdio.conf.ts`
- Create: `apps/jfmt-viewer/e2e/specs/open-tree-pointer.e2e.ts`
- Modify: `apps/jfmt-viewer/package.json` (add `"e2e"` script)

Tauri's E2E story is `tauri-driver` (a WebDriver shim) + WebdriverIO. CI runs only on Linux per the spec; Windows / macOS runners need extra setup deferred to M9.

- [ ] **Step 1: e2e package**

Create `apps/jfmt-viewer/e2e/package.json`:

```json
{
  "name": "jfmt-viewer-e2e",
  "private": true,
  "type": "module",
  "scripts": {
    "test": "wdio run wdio.conf.ts"
  },
  "devDependencies": {
    "@wdio/cli": "^8",
    "@wdio/local-runner": "^8",
    "@wdio/mocha-framework": "^8",
    "@wdio/spec-reporter": "^8",
    "ts-node": "^10",
    "tsx": "^4",
    "typescript": "^5",
    "webdriverio": "^8"
  }
}
```

Create `apps/jfmt-viewer/e2e/tsconfig.json`:

```json
{
  "compilerOptions": {
    "target": "ES2022",
    "module": "ESNext",
    "moduleResolution": "bundler",
    "esModuleInterop": true,
    "strict": true,
    "types": ["@wdio/mocha-framework", "@wdio/globals/types"]
  },
  "include": ["**/*.ts"]
}
```

Create `apps/jfmt-viewer/e2e/wdio.conf.ts`:

```ts
import { spawn, spawnSync, ChildProcess } from "node:child_process";
import { resolve } from "node:path";

let driver: ChildProcess | null = null;

export const config: WebdriverIO.Config = {
  hostname: "127.0.0.1",
  port: 4444,
  specs: ["./specs/**/*.e2e.ts"],
  maxInstances: 1,
  capabilities: [
    {
      "tauri:options": {
        application: resolve(
          __dirname,
          "../../target/release/jfmt-viewer-app",
        ),
      },
      browserName: "wry",
    } as WebdriverIO.Capabilities,
  ],
  reporters: ["spec"],
  framework: "mocha",
  mochaOpts: { ui: "bdd", timeout: 60_000 },
  logLevel: "info",
  onPrepare() {
    spawnSync("cargo", ["build", "--release", "-p", "jfmt-viewer-app"], {
      stdio: "inherit",
    });
    driver = spawn("tauri-driver", [], { stdio: "inherit" });
  },
  onComplete() {
    driver?.kill();
  },
};
```

- [ ] **Step 2: First spec**

Create `apps/jfmt-viewer/e2e/specs/open-tree-pointer.e2e.ts`:

```ts
import { browser, $ } from "@wdio/globals";
import { resolve } from "node:path";

const FIXTURE = resolve(
  __dirname,
  "../../../../crates/jfmt-viewer-core/tests/fixtures/small.json",
);

describe("jfmt-viewer", () => {
  it("opens a file and shows the tree root", async () => {
    // Bypass the OS dialog by using the window.__INITIAL_FILE__ shortcut
    // (set via the CLI integration in Task 10; for E2E we inject it via
    // the URL hash before the app boots).
    await browser.url(`tauri://localhost?file=${encodeURIComponent(FIXTURE)}`);
    const root = await $("strong=users");
    await root.waitForExist({ timeout: 10_000 });
    expect(await root.getText()).toBe("users");
  });

  it("copies a JSON Pointer", async () => {
    // Click users → 0 → name, then Copy ptr.
    await $("strong=users").click();
    await $("strong=0").waitForExist();
    await $("strong=0").click();
    await $("strong=name").waitForExist();
    await $("strong=name").click();
    await $("button*=Copy ptr").click();
    // The hint span should flash; we just check it appears.
    const hint = await $("span=copied: /users/0/name");
    await hint.waitForExist({ timeout: 3_000 });
  });
});
```

**Note:** the URL-hash boot path (`?file=…`) requires App.tsx to read `window.location` on mount — add this in this task:

Edit `apps/jfmt-viewer/src/App.tsx`. Inside the App component, before the `pickFile` declaration, add:

```tsx
  useEffect(() => {
    const url = new URL(window.location.href);
    const f = url.searchParams.get("file");
    if (f) {
      // Construct an OpenSession path the same way pickFile does.
      (async () => {
        setProgress("opening…");
        const resp = await openFile(f, (p) => {
          if (p.phase === "ready") setProgress(`ready (${p.build_ms} ms)`);
        });
        setSession({
          sessionId: resp.session_id,
          rootId: resp.root_id,
          path: f,
          totalBytes: resp.total_bytes,
          format: resp.format,
        });
      })();
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
```

- [ ] **Step 3: Add e2e script**

Edit `apps/jfmt-viewer/package.json`:

```json
"scripts": {
  "dev": "vite",
  "build": "tsc && vite build",
  "tauri": "tauri",
  "e2e": "cd e2e && pnpm install --frozen-lockfile=false && pnpm test"
}
```

- [ ] **Step 4: Smoke run on Linux only**

CI step matrix gets `if: runner.os == 'Linux'` — see Task 12.

Local Linux smoke (skip on Windows / macOS — `tauri-driver` requires xvfb / WebKitGTK):

```bash
cd apps/jfmt-viewer && pnpm e2e
```

If `tauri-driver` is not in PATH, instruct the user to install it: `cargo install tauri-driver --locked`.

- [ ] **Step 5: Commit**

```bash
git add apps/jfmt-viewer/e2e apps/jfmt-viewer/src/App.tsx apps/jfmt-viewer/package.json
git commit -m "$(cat <<'EOF'
test(viewer): WebdriverIO + tauri-driver E2E suite (Linux only)

First two specs: open small.json via URL ?file= hint, walk to
/users/0/name, copy pointer, verify hint flash. App.tsx auto-opens
when ?file= is present. CI gates this on runner.os == 'Linux';
Windows / macOS deferred to M9.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 12: Tauri bundle config + CI release matrix

**Files:**
- Modify: `apps/jfmt-viewer/src-tauri/tauri.conf.json`
- Create: `.github/workflows/viewer-release.yml`
- Modify: `.github/workflows/release.yml` (assuming cargo-dist's existing workflow — adjust path if your file is named differently)

- [ ] **Step 1: Enable bundling in tauri.conf.json**

Edit `apps/jfmt-viewer/src-tauri/tauri.conf.json`. Replace the `bundle` block with:

```json
"bundle": {
  "active": true,
  "targets": ["msi", "nsis", "deb", "appimage", "dmg"],
  "category": "DeveloperTool",
  "shortDescription": "Streaming viewer for large JSON / NDJSON files",
  "longDescription": "jfmt-viewer browses JSON / NDJSON files of arbitrary size with virtual scrolling and streaming search. Pairs with the jfmt CLI."
}
```

- [ ] **Step 2: Add the viewer release workflow**

Create `.github/workflows/viewer-release.yml`:

```yaml
name: viewer-release
on:
  push:
    tags: ["v*"]
  workflow_dispatch:

jobs:
  bundle:
    strategy:
      fail-fast: false
      matrix:
        include:
          - os: windows-latest
            target: msi
          - os: macos-latest
            target: dmg
          - os: ubuntu-22.04
            target: deb
    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@1.85
      - uses: pnpm/action-setup@v3
        with: { version: 9 }
      - uses: actions/setup-node@v4
        with: { node-version: 20, cache: pnpm, cache-dependency-path: apps/jfmt-viewer/pnpm-lock.yaml }

      - name: Install Linux build deps
        if: runner.os == 'Linux'
        run: |
          sudo apt-get update
          sudo apt-get install -y libwebkit2gtk-4.1-dev build-essential curl wget file libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev

      - name: Install frontend deps
        run: cd apps/jfmt-viewer && pnpm install --frozen-lockfile

      - uses: tauri-apps/tauri-action@v0
        with:
          projectPath: apps/jfmt-viewer
          tagName: ${{ github.ref_name }}
          releaseName: jfmt ${{ github.ref_name }}
          includeUpdaterJson: false
        env:
          GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}

  e2e:
    runs-on: ubuntu-22.04
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@1.85
      - uses: pnpm/action-setup@v3
        with: { version: 9 }
      - uses: actions/setup-node@v4
        with: { node-version: 20 }
      - name: Install Linux deps
        run: |
          sudo apt-get update
          sudo apt-get install -y libwebkit2gtk-4.1-dev xvfb
      - run: cargo install tauri-driver --locked
      - run: cd apps/jfmt-viewer && pnpm install --frozen-lockfile
      - run: cd apps/jfmt-viewer/e2e && pnpm install --frozen-lockfile=false
      - run: xvfb-run -a pnpm --dir apps/jfmt-viewer e2e
```

- [ ] **Step 3: Verify the existing cargo-dist workflow still attaches CLI binaries to the same release**

Inspect `.github/workflows/release.yml` (or whatever cargo-dist generated). It should be tag-triggered on `v*`. Both workflows attach to the same GitHub Release for the tag — `tauri-action` with `tagName: ${{ github.ref_name }}` upserts; cargo-dist does the same. They coexist.

If they collide (e.g. both try to create the release), add to the tauri job:

```yaml
        with:
          # ...
          createRelease: false
```

This is the safe default in M8.2 — cargo-dist creates the release; tauri-action attaches.

- [ ] **Step 4: Local sanity check (no actual release)**

```bash
cd apps/jfmt-viewer && pnpm tauri build --no-bundle && cd ../..
```
Expected: builds without error. `--no-bundle` skips installer creation but verifies the Tauri build pipeline before pushing the workflow.

- [ ] **Step 5: Commit**

```bash
git add apps/jfmt-viewer/src-tauri/tauri.conf.json .github/workflows/viewer-release.yml
git commit -m "$(cat <<'EOF'
ci(viewer): tauri bundle config + GitHub release workflow

Enables msi / dmg / deb / AppImage targets. New viewer-release.yml
tag-trigger workflow runs tauri-action across the three platform
runners and attaches installers to the same v0.x.y GitHub Release
that cargo-dist already populates with CLI binaries. createRelease:
false on tauri-action so cargo-dist owns the release object.

E2E job runs on Linux only with xvfb + tauri-driver.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 13: README + CHANGELOG + v0.3.0 release

**Files:**
- Modify: `README.md`
- Modify: `CHANGELOG.md`
- Modify: `Cargo.toml` (root `[workspace.package].version`)
- Modify: `apps/jfmt-viewer/src-tauri/Cargo.toml` (`version = "0.3.0"`)
- Modify: `apps/jfmt-viewer/package.json` (`"version": "0.3.0"`)
- Modify: `apps/jfmt-viewer/src-tauri/tauri.conf.json` (`"version": "0.3.0"`)

- [ ] **Step 1: Update README**

Edit `README.md`. Insert a new section after `## Convert`:

```markdown
## View

`jfmt view <file>` launches the GUI viewer (Phase 2). Supports
JSON and NDJSON files; streams the index so multi-GB files open in
seconds. Features:

- Virtual scrolling for trees with millions of nodes
- Right-pane preview of the selected subtree (pretty-printed)
- Toolbar substring search across keys and string-leaf values
  (case-insensitive default; ASCII fast path)
- One-click copy of the selected node's JSON Pointer (RFC 6901)

Install the standalone GUI from the GitHub Release alongside the
CLI. The `jfmt` CLI auto-discovers `jfmt-viewer` on PATH or next
to itself.

```

- [ ] **Step 2: Update CHANGELOG**

Edit `CHANGELOG.md`. Insert under `## [Unreleased]`:

```markdown
## [0.3.0] — 2026-05-09

### Added
- `jfmt view <file>` launches the new desktop viewer (Phase 2 M8).
- Tauri 2 + React + TanStack Virtual GUI capable of browsing 10 GB
  JSON / NDJSON files.
- Streaming substring search across keys and string-leaf values
  with cancel + progress events.
- JSON Pointer (RFC 6901) copy from any selected node.
- Right-pane subtree preview (pretty-printed; truncates ≥ 4 MB).

### Changed
- **MSRV bumped from 1.75 to 1.85.** Required by Tauri 2's
  transitive dependency on `toml_writer 1.1.1+spec-1.1.0`.

### Fixed
- M7 proptest generators (`proptest_convert`, `proptest_roundtrip`)
  now dedupe attribute names; the previous behaviour produced
  invalid XML that surfaced as a flake under 1.85's proptest
  shrinking.
```

- [ ] **Step 3: Bump versions**

Edit root `Cargo.toml`: `version = "0.3.0"` under `[workspace.package]`.

Edit `apps/jfmt-viewer/src-tauri/Cargo.toml`: change `version = "0.0.1"` to `version = "0.3.0"`.

Edit `apps/jfmt-viewer/package.json`: `"version": "0.3.0"`.

Edit `apps/jfmt-viewer/src-tauri/tauri.conf.json`: `"version": "0.3.0"`.

Edit `crates/jfmt-viewer-core/Cargo.toml`: change `version = "0.0.1"` to `version = "0.3.0"` (align with the workspace release line).

- [ ] **Step 4: Verify everything still builds + tests + clippy + fmt**

```bash
cargo test --workspace 2>&1 | grep -E "^test result:" | tail -25
cargo clippy --workspace --all-targets -- -D warnings 2>&1 | tail -3
cargo fmt --all -- --check
cd apps/jfmt-viewer && pnpm build && cd ../..
```
All four must succeed.

- [ ] **Step 5: Commit**

```bash
git add README.md CHANGELOG.md Cargo.toml Cargo.lock apps/jfmt-viewer/src-tauri/Cargo.toml apps/jfmt-viewer/src-tauri/tauri.conf.json apps/jfmt-viewer/package.json crates/jfmt-viewer-core/Cargo.toml
git commit -m "$(cat <<'EOF'
chore: bump version to 0.3.0 (M8 — Phase 2 viewer)

M8 ships:
- Streaming GUI viewer (Tauri 2 + React + TanStack Virtual)
- jfmt view CLI subcommand with binary discovery + spawn
- Streaming substring search with progress + cancel
- JSON Pointer copy
- 10 GB-scale file browsing

MSRV: 1.75 → 1.85 (required by Tauri 2 transitive deps).

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

- [ ] **Step 6: Tag and push (USER CONFIRMATION REQUIRED)**

```bash
git tag -a v0.3.0 -m "v0.3.0 — Phase 2 streaming viewer"
```

Stop here and report. Do not push without explicit user approval; pushing the tag triggers the release workflow which builds installers on three CI runners.

---

## Plan summary

13 tasks. Final state: rustc 1.85.1, all tests green, clippy clean, frontend build clean, Tauri bundles produce on three platforms, E2E suite gates merges on Linux. v0.3.0 tag is created locally; the user pushes it when ready.

After M8.2 ships, candidate M9 plan topics (do not start without user approval):
- Disk-persistent index sidecar (`.jfmtidx`)
- Search regex / number-leaf / fuzzy
- jq filter view
- macOS / Windows code signing
- Multi-tab UI
- XML viewing (unified node model across JSON / XML / NDJSON)
