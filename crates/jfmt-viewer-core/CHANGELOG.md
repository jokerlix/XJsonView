# jfmt-viewer-core changelog

All notable changes to this crate will be documented in this file.
The format follows Keep a Changelog; this crate predates a stable
release line and is versioned `0.0.x` until M8.2 ships v0.3.0.

## [0.0.1] — 2026-05-09 (M8.1 internal)

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
