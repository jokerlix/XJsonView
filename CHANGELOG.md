# Changelog

All notable changes to jfmt will be documented in this file. The format
is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/).

## [Unreleased]

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

[Unreleased]: https://github.com/lizhongwei/XJsonView/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/lizhongwei/XJsonView/releases/tag/v0.1.0
