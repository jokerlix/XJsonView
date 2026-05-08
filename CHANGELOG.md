# Changelog

All notable changes to jfmt will be documented in this file. The format
is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/).

## [Unreleased]

## [0.3.0] - 2026-05-09

First Phase 2 release. Adds a desktop GUI viewer.

### Added

- `jfmt view <file>` launches the new desktop viewer.
- Tauri 2 + React + TanStack Virtual GUI capable of browsing 10 GB
  JSON / NDJSON files with virtual scrolling.
- Streaming substring search across object keys and string-leaf
  values, with cancel + progress events.
- JSON Pointer (RFC 6901) copy from any selected node.
- Right-pane subtree preview (pretty-printed; truncates at 4 MB
  with an export hook reserved for M9).
- New `jfmt-viewer-core` crate exposing `Session::open_with_progress`,
  `get_children`, `get_value`, `get_pointer`, `run_search`.

### Changed

- **MSRV bumped from 1.75 to 1.85.** Required by Tauri 2's
  transitive dependency on `toml_writer 1.1.1+spec-1.1.0`.

### Fixed

- M7 proptest generators (`proptest_convert`, `proptest_roundtrip`)
  now dedupe attribute names; the previous behaviour produced
  invalid XML that surfaced as a flake under 1.85's proptest
  shrinking.

## [0.2.0] - 2026-04-26

First Phase 1b release.

### Added

- `jfmt convert` subcommand: streaming XML ↔ JSON conversion.
  - XML → JSON: `@attr` / `#text` mapping, always-array default,
    `--array-rule` opt-out, mixed-content text concatenation, namespace
    prefix preservation.
  - JSON → XML: single-key root convention; `--root NAME` to wrap
    multi-key / array / scalar top levels; `--xml-decl` prologue;
    `--pretty` / `--indent` / `--tabs` formatting.
  - `--strict`: error (exit 34) on non-contiguous same-name XML
    siblings; forbid `--root` rescue when JSON top level isn't a
    single-key object.
- New `jfmt-xml` crate exposing `EventReader` / `XmlWriter` over
  `quick-xml`, mirroring `jfmt-core`'s shape.

### Exit codes

- `21` — XML syntax error.
- `34` — `--strict` non-contiguous same-name siblings violation.
- `40` — Translation error (e.g. invalid XML name from JSON, multi-key
  JSON top level without `--root`).

### Notes

- `--array-rule` paths assume exactly one occurrence per parent; multiple
  occurrences at a collapsed path are rejected with exit 40 in v0.2.0.
- JSON → XML translation materializes input via `serde_json::Value`. The
  XML side is fully streaming. Constant-memory streaming of JSON → XML is
  a candidate for a follow-up release.

## [0.1.1] - 2026-04-26

### Fixed

- Repository / homepage / install URLs across `Cargo.toml`, `README.md`,
  and `CHANGELOG.md` now point at `github.com/jokerlix/XJsonView`
  (the actual remote). Prior `0.1.0` shipped with stale
  `github.com/lizhongwei/XJsonView` URLs that 404'd.

## [0.1.0] - 2026-04-26

First non-preview release. Phase 1 complete.

### Added

- `jfmt pretty` and `jfmt minify` for streaming JSON / NDJSON formatting
  at constant memory (O(nesting depth), not O(file size)).
- `jfmt validate` with syntax checking, NDJSON parallel pipeline,
  optional JSON Schema validation (`--schema FILE`), `--strict` flag,
  and `--materialize` mode.
- `jfmt filter EXPR` with three modes:
  - Single-document streaming: per-shard jaq evaluation, constant memory.
  - NDJSON parallel: full jq semantics per line.
  - `--materialize`: full jq semantics on the whole document, with a
    RAM budget pre-flight check.
- Static-check + runtime-guard rejecting whole-document jq operations
  in streaming mode (`length`, `sort_by`, `group_by`, …) and
  multi-document operations (`input`, `inputs`) in any mode.
- Cross-platform binary releases for Linux x86_64 + aarch64,
  macOS x86_64 + aarch64, and Windows x86_64 (cargo-dist).
- Criterion benchmarks for the parser, writer, and NDJSON pipeline.
- Compression: gzip and zstd input/output detected by file extension.

### Pre-1.0 history

| Tag | Date | Highlights |
|---|---|---|
| v0.0.1 | 2026-04-23 | M1: streaming pretty / minify + core + I/O. |
| v0.0.2 | 2026-04-24 | M2: `validate` syntax + stats. |
| v0.0.3 | 2026-04-24 | M3: NDJSON parallel pipeline. |
| v0.0.4 | 2026-04-25 | M4a: `filter` streaming + NDJSON. |
| v0.0.5 | 2026-04-25 | M4b: `filter --materialize`. |
| v0.0.6 | 2026-04-25 | M5: JSON Schema validation. |

[Unreleased]: https://github.com/jokerlix/XJsonView/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/jokerlix/XJsonView/releases/tag/v0.2.0
[0.1.1]: https://github.com/jokerlix/XJsonView/releases/tag/v0.1.1
[0.1.0]: https://github.com/jokerlix/XJsonView/releases/tag/v0.1.0
