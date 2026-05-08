# jfmt M8 — Phase 2 Streaming Viewer Skeleton Design

**Status:** approved 2026-05-08.
**Target version:** v0.3.0 (additive: new GUI binary + `jfmt view` subcommand; no breaking change to v0.2.x CLI surface).
**Predecessor:** Phase 1 complete at v0.1.0; Phase 1b (M7 XML) shipped at v0.2.0.
**Successor scope deferred:** see §10.

## 1. Goals

1. Provide a desktop GUI that opens JSON / NDJSON files and presents them as a collapsible tree.
2. Support **single files ≥ 10 GB** with smooth interaction via virtual scrolling and on-demand loading. Targets: cold-open < 3 s on 5 GB JSON, expand any node < 100 ms, scrolling 60 fps.
3. When a tree node is selected, render the full pretty-formatted JSON of that subtree in a side preview pane.
4. Right-click / toolbar button on any node copies its RFC 6901 JSON Pointer (e.g. `/users/3/profile`) to the system clipboard.
5. NDJSON files render with a virtual top-level list (`doc #0`, `doc #1`, ...); each doc expands as a normal tree.
6. `jfmt view <file>` CLI command launches the GUI with the path pre-loaded.
7. Tauri produces Windows MSI / macOS `.app` / Linux `.deb`+`.AppImage` installers, attached to the v0.3.0 GitHub Release alongside the existing `jfmt` CLI binary.
8. Toolbar keyword search streams matches across the whole file (substring, case-insensitive default; matches both object keys and string-leaf values). Hit list panel shows path + snippet; clicking a hit jumps to that node.

**Done definition:** on a 5 GB JSON sample, cold-open completes < 3 s; scrolling root → end stays at 60 fps; copying any node's pointer produces a string accepted by `jq`'s `getpath`; CLI `jfmt view bigfile.json` launches the GUI directly; a 3-character substring search on the same 5 GB file returns first hit < 1 s and full scan < 15 s.

## 2. Non-Goals (deferred to M9+)

- **Editing / saving** — viewer only.
- **XML viewing** in the GUI. `jfmt-xml` exists but a unified node model covering both JSON and XML hierarchies is out of scope; revisit in M9.
- **YAML / TOML viewing** (none of these formats are read by jfmt yet).
- **Multiple tabs / multiple windows** — one document per process in M8.
- **Disk-persistent index** (`.jfmtidx` sidecar). M8 rebuilds the in-memory index on every open.
- **Theme switcher UI.** M8 follows the system light/dark theme; no in-app toggle.
- **Search beyond plain substring on string values & keys** — no regex, no number/bool/null leaf matching, no fuzzy or word-boundary modes.
- **jq / jaq integration in the GUI** (filter view, transformed export).
- **Code-signed macOS / Windows binaries.** Users will see Gatekeeper / SmartScreen warnings; mitigations in §7.
- **Internationalization.** UI strings English-only.

## 3. Architecture

### 3.1 Layered design

```
┌─────────────────────────────────────────────────┐
│  React UI (apps/jfmt-viewer/src)                │
│  - Tree (virtualized), Preview, Toolbar,        │
│    Search hit list                              │
│  - invoke() Tauri commands                      │
└────────────────┬────────────────────────────────┘
                 │ Tauri IPC (JSON over channel)
┌────────────────▼────────────────────────────────┐
│  Tauri commands (apps/jfmt-viewer/src-tauri)    │
│  - open_file / close_file / get_children /      │
│    get_value / get_pointer / search /           │
│    cancel_search                                │
│  - Owns ViewerState: HashMap<SessionId,         │
│    Arc<RwLock<SessionState>>>                   │
└────────────────┬────────────────────────────────┘
                 │ Rust API
┌────────────────▼────────────────────────────────┐
│  jfmt-viewer-core (no UI, no Tauri)             │
│  - SparseIndex over file: containers only       │
│  - get_children / get_value (re-parses children │
│    on demand using EventReader)                 │
│  - search(): streaming scan + hit dispatch      │
│  - Reuses jfmt-core EventReader                 │
└─────────────────────────────────────────────────┘
```

### 3.2 Workspace layout (changes vs current)

```
crates/
  jfmt-core/              (existing)
  jfmt-io/                (existing)
  jfmt-cli/               (existing) — adds src/commands/view.rs
  jfmt-xml/               (existing)
  jfmt-viewer-core/       NEW — index + IPC data model, no UI deps
apps/                     NEW directory at repo root
  jfmt-viewer/            Tauri 2 application
    src-tauri/            Cargo workspace member: tauri commands
    src/                  React + TypeScript + Vite frontend
    package.json
    pnpm-lock.yaml
    tauri.conf.json
```

`apps/jfmt-viewer/src-tauri` is added to the root `Cargo.toml` `[workspace] members`. The frontend tree is *not* a Cargo member — it lives under the same git repo but uses pnpm.

### 3.3 Indexing model

On open, `jfmt-viewer-core` performs a single forward pass over the file using `jfmt-core::EventReader`, producing a **sparse index** that records *only container* nodes (objects and arrays). Leaf values are not indexed.

```rust
pub type NodeId = u64;          // index into Vec<ContainerEntry>; 0 = root
pub const ROOT: NodeId = 0;

pub struct ContainerEntry {
    pub file_offset: u64,        // byte offset of the opening `{` or `[`
    pub byte_end: u64,           // byte offset *after* the closing `}` or `]`
    pub parent: Option<NodeId>,  // None for root
    pub key_or_index: KeyRef,    // SmallVec<u8;16> — key bytes for object child;
                                 // ASCII decimal for array child
    pub kind: ContainerKind,     // Object | Array | NdjsonDoc
    pub child_count: u32,        // direct children (containers + leaves)
    pub first_child: Option<NodeId>,  // first child container in the index, if any
}
```

Approximate size: 48 bytes per entry. Memory budget:

| File size | Containers (approx) | Index RAM |
|---|---|---|
| 100 MB    | 400 K              | 19 MB |
| 1 GB      | 4 M                | 192 MB |
| 5 GB      | 20 M               | 960 MB |
| 10 GB     | 40 M               | 1.9 GB |

The 10 GB ceiling is the M8 hard limit on practical use; M9 sidecar index lifts this.

### 3.4 Leaf rendering

When `get_children(parent, offset, limit)` is called, the core seeks to the parent's `file_offset`, parses with `EventReader` until the container closes, and emits a `ChildSummary` per direct child:

```rust
pub struct ChildSummary {
    pub id: Option<NodeId>,         // container child: index id; leaf: None
    pub key: String,
    pub kind: Kind,                 // object|array|string|number|bool|null|ndjson_doc
    pub child_count: u32,           // containers: real count; leaves: 0
    pub preview: Option<String>,    // leaves only; full value if ≤ 256 bytes,
                                    // otherwise first 200 bytes + "…"
}
```

This implies a small re-read cost on each container expansion. The kernel page cache absorbs this in practice for the working set.

### 3.5 NDJSON

When the file is detected as NDJSON (extension `.ndjson` / `.jsonl`, or — when forced via `jfmt view --ndjson` — line-by-line parsing), the index has a synthetic root of `ContainerKind::NdjsonDoc` with one child per non-blank line. Each line's `file_offset` / `byte_end` cover that line; expanding emits the line's own root container as a child.

## 4. IPC contract

Seven commands. All return `Result<T, ViewerError>`. Cancellation and progress use Tauri 2 typed `Channel<T>`.

### 4.1 Types (TypeScript mirror)

```typescript
type NodeId = number;
type Kind = "object" | "array" | "string" | "number" | "bool" | "null" | "ndjson_doc";

interface OpenFileResp {
  session_id: string;
  root_id: NodeId;          // always 0
  format: "json" | "ndjson";
  total_bytes: number;
}

interface ChildSummary {
  id: NodeId | null;
  key: string;
  kind: Kind;
  child_count: number;
  preview: string | null;
}

type IndexProgress =
  | { phase: "scanning"; bytes_done: number; bytes_total: number }
  | { phase: "ready"; build_ms: number }
  | { phase: "error"; message: string };

interface SearchQuery {
  needle: string;             // trimmed; ≥ 1 character
  case_sensitive: boolean;
  scope: "both" | "keys" | "values";
}

interface SearchHandle { id: string; }

type SearchEvent =
  | { kind: "hit"; node: NodeId | null; path: string;
      matched_in: "key" | "value"; snippet: string }
  | { kind: "progress"; bytes_done: number; bytes_total: number; hits_so_far: number }
  | { kind: "done"; total_hits: number; elapsed_ms: number }
  | { kind: "cancelled" }
  | { kind: "error"; message: string };
```

### 4.2 Commands

| Command | Args | Returns | Notes |
|---|---|---|---|
| `open_file` | `{ path, on_progress: Channel<IndexProgress> }` | `OpenFileResp` | Returns immediately with session id; index built on a `tokio::task::spawn_blocking`. Channel receives `Scanning` events (~50 ms or 64 MB cadence) ending in `Ready` or `Error`. |
| `close_file` | `{ session_id }` | `()` | Drops session state and any open file handle. |
| `get_children` | `{ session_id, parent: NodeId, offset: u32, limit: u32 }` | `{ items: ChildSummary[], total: u32 }` | Window pagination for virtual scroll; `total` is the full count regardless of `limit`. |
| `get_value` | `{ session_id, node: NodeId, max_bytes?: u64 }` | `{ json: string, truncated: bool }` | Containers → pretty-print full subtree; leaves → full value. `max_bytes` defaults to 4 MB; oversized subtrees return prefix + truncated marker. |
| `get_pointer` | `{ session_id, node: NodeId }` | `{ pointer: string }` | RFC 6901 (escapes `~` → `~0`, `/` → `~1`). Root → `""`. |
| `search` | `{ session_id, query: SearchQuery, on_event: Channel<SearchEvent> }` | `SearchHandle` | Spawns background scan; cancelable via `cancel_search`. |
| `cancel_search` | `{ handle: SearchHandle }` | `()` | Sets an `AtomicBool` watched by the scanner. |

### 4.3 Errors

```rust
#[derive(Serialize, thiserror::Error)]
pub enum ViewerError {
    #[error("file not found: {0}")] NotFound(String),
    #[error("session not found")]   InvalidSession,
    #[error("node out of range")]   InvalidNode,
    #[error("indexing in progress")] NotReady,
    #[error("parse error at byte {pos}: {msg}")] Parse { pos: u64, msg: String },
    #[error("io: {0}")]             Io(String),
}
```

`Parse` and `Io` propagate from `jfmt-core`. The frontend maps each variant to a toast or modal — see §5.4.

### 4.4 Truncation hook

`get_value` with subtree size > `max_bytes` returns the pretty-printed prefix plus a literal trailer:

```
... (truncated, NNN MB total — export full subtree feature lands in M9)
```

This is a literal placeholder; M9 adds an `export_subtree` command and the trailer text changes.

## 5. UX

### 5.1 Layout

```
┌────────────────────────────────────────────────────────────────────┐
│ jfmt-viewer — bigfile.json (5.2 GB)         [system theme]   ─ ☐ ✕│
├────────────────────────────────────────────────────────────────────┤
│ [📁 Open] | 🔍 [_______] [Aa] [both▾]  3/127 ↑↓ ✕ | [📋 Copy ptr] │
├──────────────────────────┬─────────────────────────────────────────┤
│ ▾ {root} (3 keys)        │ {                                       │
│   ▸ users [10,000,000]   │   "id": 3,                              │
│   ▾ meta {4 keys}        │   "name": "Alice",                      │
│     • version: "2.1"     │   "profile": {                          │
│     • created: "2026-01" │     "email": "a@x.io",                  │
│     ▾ tags [3]           │     "joined": "2024-02-15"              │
│       • 0: "prod"        │   }                                     │
│       • 1: "v2"          │ }                                       │
│   ▸ events [N/A]         │                                         │
├──────────────────────────┴─────────────────────────────────────────┤
│ Indexing: ████████████░░  4.1 / 5.2 GB · est. 3 s left             │
└────────────────────────────────────────────────────────────────────┘
```

- **Toolbar (top, 40 px):** Open button, search box (input + Aa + scope dropdown + counter + nav arrows + clear), current selection's pointer text, Copy Pointer button.
- **Tree pane (left, default 40 % width, draggable splitter):** virtualized list, ~22 px row height.
- **Preview pane (right, default 60 %):** `<pre>`, monospace, no syntax highlight (M9).
- **Status bar (bottom):** indexing progress while scanning; afterwards file stats and last-query latency.

### 5.2 Tree row

```
[chevron] [icon] [key]  [type-badge]  [preview / count]
   ▾       {}    meta   object        {4 keys}
   ▸       []    users  array         [10,000,000]
   •       •     ver    string        "2.1"
```

- Containers: `▾`/`▸` chevron toggles expansion.
- Leaves: `•` bullet, no chevron.
- Type badge in 11 px gray text.
- Preview clipped to row width with ellipsis.

### 5.3 Interactions

| Action | Trigger | Backend |
|---|---|---|
| Open file | Toolbar button (tauri-plugin-dialog), CLI `jfmt view`, drag-drop into window | `open_file` |
| Expand container | Click chevron / double-click row | `get_children(parent, offset=0, limit=200)`; subsequent windows on scroll-near-bottom |
| Select node | Single click row | `get_value` + `get_pointer` (parallel) |
| Copy pointer | Toolbar button / right-click menu / `Ctrl+C` (tree focus) | None — uses already-fetched pointer |
| Search | Type in box (250 ms debounce), `Aa`, scope dropdown | New `search`; cancels prior handle |
| Search nav | `↑` / `↓` arrows, `F3` / `Shift+F3` | Local — uses cached hit list |
| Jump to hit | Click hit row in panel | Auto-expands path, selects, `get_value` for preview |

### 5.4 Keyboard

- `↑`/`↓` move tree selection
- `→` expand current; `←` collapse current
- `Enter` toggle expansion
- `Ctrl+C` (tree focus) copy pointer of selected node
- `Ctrl+O` open file
- `Ctrl+F` focus search box
- `F3` / `Shift+F3` next / previous search hit
- `Esc` clear search if box focused; otherwise no-op (cancel-during-load is M9)

### 5.5 Search hit list

When a search has at least one hit, a hit-list panel slides in from the left, pushing the tree right, or stacks above the tree on narrow windows. Each row:

```
/users/3/name        VAL  "...Al**ice** Smith..."
/users/3/email       VAL  "...al**ice**@x.io"
/meta/contact/alice  KEY  **alice**
```

Hit-list cap: 1 000 rows. When exceeded, status banner reads "more than 1 000 hits — refine your query"; scan continues but the tail is dropped. Snippet rule: leaf values are clipped to ~32 chars before/after the match; matched span is delimited with `**` markers (frontend renders bold).

### 5.6 Theme

Tauri reports system theme; React reads `prefers-color-scheme` and applies CSS variables. No in-app toggle (M9).

## 6. Testing

### 6.1 Layers

| Layer | Tool | Coverage |
|---|---|---|
| `jfmt-viewer-core` unit | `cargo test` | Index correctness, `get_children` pagination, pointer escaping, NDJSON detection, JSON Pointer round-trip |
| `jfmt-viewer-core` property | `proptest` | Arbitrary JSON → index → reconstructed value via `get_value` equals `serde_json` parse |
| `jfmt-viewer-core` bench | `criterion` | Index throughput (MB/s), `get_children` p50/p99, `search` throughput |
| `src-tauri` unit | `cargo test` + handcrafted IPC fixtures | Command argument serialization, error mapping |
| Frontend unit | Vitest + React Testing Library | Tree row rendering, virtual scroll mock, pointer copy, search debounce |
| End-to-end | WebdriverIO + tauri-driver | Open small fixture → expand → select → assert preview text + clipboard |
| Big-file smoke | shell + 5 GB generated fixture | Local `cargo make big-smoke`; nightly CI job, not on every push |

### 6.2 Fixtures

```
crates/jfmt-viewer-core/tests/fixtures/
  small.json          ~50 lines — unit + E2E
  ndjson.ndjson       1 000 lines
  deep.json           500-level nesting — stack-overflow regression
  wide-array.json     100 K elements — virtual scroll exercise
  unicode.json        emoji + 4-byte UTF-8 + RFC 6901 special chars
```

The 5 GB fixture is generated by `scripts/gen-big-fixture.py`; it is `.gitignore`d.

### 6.3 CI matrix

| Platform | Build | Unit | E2E |
|---|---|---|---|
| Linux x64 | yes | yes | yes (Linux runner) |
| Windows x64 | yes | yes | no (M8); manual or self-hosted later |
| macOS arm64 | yes | yes | no (M8) |

## 7. Distribution

### 7.1 Release pipeline

| Artifact | Tool | Trigger |
|---|---|---|
| `jfmt` CLI (with `jfmt view` subcommand) | cargo-dist (existing) | tag `v0.3.0` → GitHub Release |
| `jfmt-viewer` GUI installer | `tauri build` in CI | same tag, attached to same Release |

### 7.2 Platform installers

- **Windows x64:** `.msi` via Tauri's WiX backend. Unsigned in M8; SmartScreen warning expected. README documents the "More info → Run anyway" path.
- **macOS:** universal `.dmg` (arm64 + x64). Unsigned and unnotarized in M8; README documents `xattr -cr /Applications/jfmt-viewer.app` to bypass Gatekeeper.
- **Linux x64:** `.deb` and portable `.AppImage`.

### 7.3 `jfmt view` resolution

`jfmt view <file>` finds the GUI binary in this order:

1. Same directory as the `jfmt` executable (single-archive distribution).
2. `jfmt-viewer` on `PATH`.
3. macOS only: `/Applications/jfmt-viewer.app/Contents/MacOS/jfmt-viewer`.
4. Otherwise prints an error with the install URL and exits non-zero.

The CLI passes the absolute file path as the first arg; the GUI's startup logic detects this and calls `open_file` immediately on launch.

## 8. Sub-milestones

M8 is one Phase 2 release (v0.3.0) but is implemented in two internal phases. Only M8.2 ships publicly.

### M8.1 — Core + minimal UI (3 weeks, internal only)

- `jfmt-viewer-core` complete: index, all 7 commands' core logic, search backend with cancelation.
- Tauri shell with raw `<ul>` tree (no virtual scroll yet) — proves IPC contract end-to-end.
- Unit + property tests, bench harness.
- Search backend wired but no UI.
- Internal dogfood; **no public release**.

### M8.2 — Production polish (2 weeks, ships v0.3.0)

- Virtual scroll, preview pane, pointer copy, NDJSON top-level rendering, search UI (toolbar + hit list panel).
- CLI `jfmt view` integration + binary discovery.
- E2E suite, Tauri build pipeline, cargo-dist GitHub Action coordination.
- README and CHANGELOG updates.
- Tag `v0.3.0`, dual-pipeline release.

## 9. Risks & Open Questions

| Risk | Mitigation |
|---|---|
| Tauri 2 IPC large-string serialization (`get_value` ≥ few MB) shows perf cliff. | `max_bytes = 4 MB` default; if > 100 ms in real measurements, switch `get_value` to a `Channel<String>`-based streaming path before M8.2 ships. |
| `get_children` re-reads leaves N+1 times for narrow-deep arrays. | Inline-cache leaves ≤ 256 B in the index; if real-world MB/s falls below threshold, add an LRU cache of recent windows. |
| `tauri-driver` E2E flaky on Windows runner. | M8 only runs E2E on Linux; Windows E2E in M9. |
| macOS unsigned bundle blocked by Gatekeeper. | Document `xattr -cr` workaround; budget Apple Developer account for M9. |
| pnpm-lock vs Cargo.lock drift causes irreproducible builds. | CI step verifies both lockfiles are committed and not out of sync with manifests. |
| Unicode case folding tanks search throughput. | ASCII fast path (memchr); Unicode mapping only on ASCII match candidates. |
| High-frequency hit pushes flood IPC. | Backend batches hits (16 per burst or every 50 ms). |
| 1 000-hit cap surprises users on dense matches. | Status banner explains the cap; M9 lifts it via streaming hit-list virtualization. |
| Rapid re-typing in search box launches dueling scans. | Each new query first calls `cancel_search` on the prior handle; debounce 250 ms before issuing. |
| 10 GB index (~1.9 GB RAM) approaches desktop limits. | Document the limit; M9 sidecar `.jfmtidx` reduces RAM by 10×. |

## 10. Out of Scope (M9+ candidates)

- XML viewer (unified node model across JSON / XML / NDJSON).
- Persistent disk index (`.jfmtidx` sidecar, mtime invalidation).
- Search regex / number leaves / fuzzy.
- jq / jaq filter view + transformed export.
- Code signing on macOS and Windows.
- Multi-document tabs.
- Internationalization.
- Edit / save.
- Subtree export to file (`get_value` truncation hook becomes `export_subtree` command).
- Theme switcher UI.
- Cancel-during-load (`Esc` interrupts `get_value`).

## 11. References

- Phase 1 spec: `docs/superpowers/specs/2026-04-23-jfmt-phase1-design.md`
- M7 (Phase 1b XML) spec: `docs/superpowers/specs/2026-04-26-jfmt-m7-xml-support-design.md`
- RFC 6901 (JSON Pointer): https://www.rfc-editor.org/rfc/rfc6901
- Tauri 2 channels: https://v2.tauri.app/develop/calling-rust/#channels
- TanStack Virtual: https://tanstack.com/virtual/latest
