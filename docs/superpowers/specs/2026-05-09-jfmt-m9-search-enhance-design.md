# jfmt M9 — Regex Search, Subtree Scoping, Subtree Export Design

**Status:** approved 2026-05-09.
**Target version:** v0.4.0 (additive: extends `search` IPC contract; new `export_subtree` IPC + UI; no breaking change to v0.3.x CLI surface).
**Predecessor:** M8.2 shipped at v0.3.0.
**Successor scope deferred:** see §8.

## 1. Goals

1. Add **regex matching** to viewer search. Users can switch the toolbar between substring and regex modes; regex syntax errors surface inline without launching a query.
2. Allow **scoping a search to a subtree**: `from_node: NodeId` on `SearchQuery`. The viewer's right-click menu adds "Search from this node…", which puts focus in the toolbar with the scope locked to the selected subtree.
3. Add **subtree export**: new `export_subtree` IPC command writes a node's pretty-printed JSON to a target file path. Right-click → "Export subtree…" opens the platform save-as dialog and calls the command. The `get_value` truncation trailer in M8 becomes a clickable "Export full subtree" button on the preview pane.
4. Backend regex performance: linear scan across keys + string-leaf values stays ≤ 2× substring throughput on a 1 GB JSON file (regex compiled once, fixed-prefix optimization where applicable).
5. Cancel + progress events keep working with regex; subtree-scoped searches emit `bytes_total` proportional to the scoped byte range, not the whole file.

**Done definition:** on a 5 GB JSON sample, a regex like `^/users/\d+/email` (against pointer paths, future M10) and a value-side regex like `^[A-Z]{2}\d{4}$` complete in ≤ 2× the time of an equivalent literal substring; "Search from this node" on a 1 M-element array's child returns first hit < 100 ms; "Export subtree" on the root produces a byte-identical pretty-print of the file (modulo whitespace) for ≤ 1 GB inputs and a streaming export for larger.

## 2. Non-Goals (defer to M10+)

- **Number / bool / null leaf search.** Today only `Scalar::String` leaves are scanned; numeric matching wants different UX (range comparators) and is best designed alongside.
- **Fuzzy / scored matching** (skim-style or `fuzzy-matcher` crate). Adds ranking complexity and a separate hit-list sort mode.
- **Path-glob filtering** (e.g., only return hits whose pointer matches `/users/*/email`). Path regex is implementable via the existing `path` field on `SearchHit` plus client-side filter, but a true server-side path filter wants its own design.
- **Search-and-replace.** Out of scope; jfmt remains read-only.
- **Saved searches / search history UI.**
- **Disk-persistent index sidecar.** Still queued for the milestone after M10.
- **Code signing on macOS / Windows.** Same.

## 3. Architecture

### 3.1 Backend changes

`jfmt-viewer-core` extensions:

- `SearchQuery` gains two fields:
  ```rust
  pub struct SearchQuery {
      pub needle: String,
      pub mode: SearchMode,    // NEW: Substring | Regex
      pub case_sensitive: bool,
      pub scope: SearchScope,  // existing: Both | Keys | Values
      pub from_node: Option<NodeId>,  // NEW: None = whole file
  }

  pub enum SearchMode { Substring, Regex }
  ```
- `run_search` builds either a `memchr::memmem::Finder` (Substring) or a compiled `regex::Regex` (Regex) once before the loop. Regex compile errors return `Err(ViewerError::InvalidQuery(msg))` — new variant.
- When `from_node` is `Some`, `run_search` reads only `bytes[entry.file_offset..entry.byte_end]` instead of the whole file. The path tracking in the search loop initializes its `path_segments` stack from the parent chain of `from_node` so `SearchHit.path` remains absolute (RFC 6901 from root).
- New `Session::export_subtree(node, target_path: &Path, options: ExportOptions) -> Result<u64>` returns bytes written. `ExportOptions { pretty: bool }`. For containers it walks the index and re-streams the bytes; for leaves it writes the leaf value as JSON.

`regex` crate is added as a workspace dep (already present transitively via `tracing-subscriber`'s dev tree, but pin explicitly):
```toml
regex = "1"
```

### 3.2 IPC additions

```typescript
type SearchMode = "substring" | "regex";

interface SearchQuery {
  needle: string;
  mode: SearchMode;
  case_sensitive: boolean;
  scope: "both" | "keys" | "values";
  from_node?: NodeId;
}

// New command
interface ExportSubtreeArgs {
  session_id: string;
  node: NodeId;
  target_path: string;     // absolute
  pretty: boolean;
}
interface ExportSubtreeResp {
  bytes_written: number;
  elapsed_ms: number;
}
```

Eight IPC commands now (added `export_subtree`).

### 3.3 ViewerError variants

```rust
#[error("invalid query: {0}")]
InvalidQuery(String),
```

Frontend maps `InvalidQuery` to a red border on the search input + tooltip with the message.

### 3.4 Regex performance plan

- Use `regex::Regex::new(needle)` once.
- For mode=Regex with `case_sensitive: false`, build with `RegexBuilder::new(needle).case_insensitive(true)`.
- Apply `regex::Regex` to keys and string-leaf values; do NOT apply to whole-file bytes — would require careful boundary handling.
- Hot loop unchanged otherwise; regex matches are typically 2-5× slower than memchr on long strings, but JSON keys / leaf values are short, so the regex DFA fast path covers most cases.

## 4. UI changes

### 4.1 Search toolbar

Add a third button next to the existing `Aa` toggle: `.*` toggles regex mode. When enabled:
- Input border turns blue while query is non-empty
- If `regex::Regex::new` fails server-side, the response error sets `query_error` state which renders the border red + tooltip.

### 4.2 Subtree-scoped search

- Right-click on a tree row → context menu with "Search from this node".
- Clicking sets the search input's `from_node` to that NodeId, autofocuses the input, and (if input non-empty) re-issues the query.
- A small chip near the input shows `scope: /users/3` when active; clicking the chip clears the scope back to whole-file.

### 4.3 Export subtree

Two entry points:
1. Right-click any tree row → "Export subtree…" → save dialog (`tauri-plugin-dialog`'s `save`) → call `export_subtree(node, path, { pretty: true })` → toast on success ("Exported 4.2 MB to bigfile-users-3.json").
2. Preview pane truncation marker in M8 (`(see truncation marker above; full export ships in M9)`) becomes a button: `[ Export full subtree → ]`. Click → save dialog → same `export_subtree`.

### 4.4 Keyboard

No new shortcuts. Existing search nav (F3, Shift+F3, Ctrl+F, Esc) all carry over.

## 5. Testing

### 5.1 Backend

| Layer | Coverage |
|---|---|
| `jfmt-viewer-core` unit | regex compile error → `InvalidQuery`; substring + regex modes both find expected hits in `small.json`; case-insensitive regex; subtree-scoped search filters out parent siblings. |
| `jfmt-viewer-core` proptest | for any random JSON, an arbitrary substring matches the same set of leaves under both `mode=Substring(s)` and `mode=Regex(escape(s))`. (Anchors regex behaves identically to substring when the pattern is the escaped literal.) |
| `jfmt-viewer-core` bench | regex throughput vs substring on the 50K-line NDJSON fixture. Floor: regex < 5× substring, otherwise document the regression. |
| Tauri commands | `export_subtree` smoke: write to tempfile, read back, parse, equals `serde_json::from_str(get_value(...).json)`. |

### 5.2 Frontend

| Layer | Coverage |
|---|---|
| Vitest | regex toggle UI; right-click menu emits the right `from_node`; truncation button calls `export_subtree`. |
| E2E (WebdriverIO) | open `small.json`, switch to regex, type `Al.+e`, click hit → tree expands. Right-click → "Search from /users", verify scope chip. Right-click → Export → write to tempfile, then read it back via `fs.readFileSync` in the spec to assert content. |

### 5.3 Fixtures

No new fixtures required. Reuses `small.json`, `large-ndjson.ndjson`, `unicode.json`, `wide-array.json` from M8.

## 6. Distribution

v0.4.0 release. cargo-dist + viewer-release.yml continue to build and attach installers. No new platform targets, no signing.

## 7. Risks

| Risk | Mitigation |
|---|---|
| User-supplied regex can be catastrophically slow (ReDoS). | `regex` crate is linear-time by design (no backreferences). Cap query at 256 bytes; reject longer needles client-side. |
| Subtree-scoped search emits absolute paths but the backend reads only the slice — path tracking must initialize the stack from `from_node`'s parent chain. | Cover with a unit test that asserts the emitted `SearchHit.path` is absolute (e.g. `/users/3/profile/email`, not `/profile/email`). |
| `export_subtree` for huge subtrees blocks the GUI. | Run on `tokio::task::spawn_blocking`; emit no progress in M9 (file dialogs already block); document that export of multi-GB subtrees may take seconds. |
| Tauri's `save` dialog returns `null` if the user cancels — the command must handle that without erroring. | Frontend short-circuits: if `path === null`, no IPC call. |
| Regex case-insensitive Unicode handling differs from substring's ASCII fast path. | Document; Unicode case folding is the intended behavior — users who want ASCII can use `(?-u)` syntax. |

## 8. Out of scope (M10+)

- Number / bool / null leaf search.
- Fuzzy / scored matching.
- Path-glob filtering.
- Search-and-replace.
- Saved searches.
- Disk-persistent index sidecar.
- Code signing.
- jq / jaq filter view.

## 9. References

- M8.2 spec: `docs/superpowers/specs/2026-05-08-jfmt-m8-viewer-skeleton-design.md`
- M8.2 plan: `docs/superpowers/plans/2026-05-09-jfmt-m8-2-viewer-polish.md`
- `regex` crate: https://docs.rs/regex
- Tauri 2 dialog `save`: https://v2.tauri.app/plugin/dialog/#save
