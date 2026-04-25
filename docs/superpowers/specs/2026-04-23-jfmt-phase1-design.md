# jfmt — Phase 1 Design

**Date:** 2026-04-23
**Status:** Draft — pending user review
**Scope:** Phase 1 only (streaming core + CLI for JSON/NDJSON)

---

## 1. Goal

Build a Rust CLI tool and reusable core library that can format, minify, validate, and filter JSON / NDJSON files at **TB scale** with **constant memory** (O(nesting depth), not O(file size)).

The project is the first of four planned phases:

- **Phase 1 (this spec):** streaming core engine + CLI — JSON / NDJSON only
- **Phase 1b:** XML, YAML, SQL dump support; format conversion
- **Phase 2:** on-disk index generator (sidecar `.idx` files)
- **Phase 3:** desktop GUI viewer (virtual scroll + lazy load, depends on Phase 2)
- **Phase 4:** local HTTP server + web UI (reuses Phase 3 core)

Each phase ships independently usable artifacts.

## 2. Non-Goals (Phase 1)

Explicitly **out of scope** for this spec to prevent mid-project argument:

- XML / YAML / SQL / CSV support
- Format-to-format conversion (`jfmt convert`)
- On-disk indexing or random-access reads
- Any GUI or web surface
- Deep streaming filter with full jq semantics — violates jq's evaluation model; users who need it should use `--materialize` or pre-split data
- `--in-place` editing of TB files (too dangerous; require explicit `-o`)

## 3. Target Users & Use Cases

1. **Log engineer** formatting a 200 GB NDJSON log file for readability / grep.
2. **Database admin** validating a 1 TB `pg_dump`-style JSON backup against a schema (later phase — noted for direction).
3. **Data analyst** filtering events out of an NDJSON archive: `jfmt filter '.[] | select(.error)' archive.ndjson.gz`.
4. **CI pipeline** running `jfmt validate --schema … --stats-json report.json` against nightly exports.

## 4. Architecture

Three-layer Cargo workspace. Each layer compiles independently and is consumable by future phases (GUI, Web) without the CLI.

```
jfmt-cli            binary: user interface, clap, progress, exit codes
  └── jfmt-io       lib:    stream adapters, gzip/zstd (de)compression, stdin/stdout
       └── jfmt-core  lib:  parser, writer, filter, validator — zero I/O assumptions
```

**Design principle:** `jfmt-core` only accepts `impl Read` / `impl Write`. No `println`, no `File::open`. This keeps the engine reusable across all four phases.

### 4.1 jfmt-core modules

- `event.rs` — `Event` enum (`StartObject`, `EndObject`, `StartArray`, `EndArray`, `Name(&str)`, `Value(Scalar)`) plus a depth cursor.
- `parser.rs` — wraps `struson` and exposes the unified `Event` iterator. Struson is the chosen backend because `serde_json::StreamDeserializer` only streams NDJSON, not the interior of a single document.
- `writer/pretty.rs` — indent, `--sort-keys`, `--array-per-line`, escape handling.
- `writer/minify.rs` — strips whitespace, shortest-valid output.
- `filter/mod.rs` — embeds `jaq-core` + `jaq-std` + `jaq-parse`. Drives jaq with one top-level shard at a time in streaming mode.
- `filter/static_check.rs` — walks jaq AST before execution; rejects expressions that need the full document (`length`, `sort_by`, `group_by`, `add`, `min`, `max`, `unique`, aggregate array constructors). Emits an error suggesting `--materialize` or `--ndjson`.
- `filter/materialize.rs` — `serde_json::from_reader` → jaq full-semantic evaluation. Memory budget check at startup.
- `validate/syntax.rs` — reuses the tokenizer, reports byte offset + line/column on failure.
- `validate/schema.rs` — wraps `jsonschema` crate. Feeds it whole values, so only works on NDJSON lines, top-level array elements, or materialized documents.
- `validate/stats.rs` — streaming `StatsCollector`: counts records, type distribution, max depth, top-level key frequencies, Schema violation histogram.
- `ndjson/splitter.rs` — single-threaded `\n` splitter, assigns monotonic sequence numbers, pushes into a bounded `crossbeam-channel`.
- `ndjson/worker.rs` — N worker threads (N = physical cores), each parses → transforms → serializes a line.
- `ndjson/reorder.rs` — min-heap reorder buffer; emits in input order.

### 4.2 jfmt-io

- `input.rs` — opens a path or stdin; detects `.gz`/`.zst` by extension (configurable via `--compress`); returns an `impl BufRead`.
- `output.rs` — symmetric: `-o foo.json.gz` wraps the writer in a gzip encoder.

### 4.3 jfmt-cli

- `clap` derive-based subcommand router.
- `indicatif` progress bar keyed off bytes-read, shown on stderr when stdout is a TTY and `--quiet` is unset.
- Exit codes: `0` success · `1` input/system error · `2` syntax error · `3` schema validation failure.

## 5. CLI Surface

```
jfmt pretty   [INPUT] [-o OUTPUT] [--indent N] [--sort-keys] [--array-per-line] [--ndjson]
jfmt minify   [INPUT] [-o OUTPUT] [--ndjson]
jfmt validate [INPUT] [--schema FILE] [--stats] [--stats-json FILE] [--strict] [--fail-fast] [--ndjson]
jfmt filter   EXPR [INPUT] [-o OUTPUT] [--ndjson] [-m|--materialize] [--force]
jfmt convert  …                                          # reserved for Phase 1b, rejected in 1
```

Common rules:
- `[INPUT]` omitted or `-` means stdin.
- Compression detected by extension; overridable with `--compress none|gz|zst`.
- `--ndjson` auto-set for `.ndjson` / `.jsonl` extensions.
- Progress bar on stderr by default; `--quiet` suppresses.

### 5.1 Filter execution modes

| Mode | Trigger | Memory | Capability |
|---|---|---|---|
| Streaming | default | O(depth) | top-level shard evaluation only; TB-safe |
| Materialize | `-m` / `--materialize` | ~6× file size | full jq semantics (`sort_by`, `group_by`, `length`, aggregates) |
| NDJSON parallel | `--ndjson` or auto | O(line × cores) | full jq per line, ordered output |

Materialize mode estimates peak RAM = `file_size × 6` (post-decompression; assumes 5× compression ratio when input is compressed). If estimate exceeds 80 % of system RAM, require `--force` to proceed.

### 5.2 Regex in filter

jq regex builtins (`test`, `match`, `capture`, `scan`, `sub`, `gsub`) come from `jaq-std`, backed by the Rust `regex` crate. Dialect = `regex` crate syntax (no backreferences, no look-around; supports Unicode + named captures). Documented in `jfmt filter --help`.

Examples:
```bash
jfmt filter '.[] | select(.url | test("^https://"))' urls.json
jfmt filter '.[] | select(.msg | test("error|fatal"; "i"))' logs.ndjson --ndjson
jfmt filter '.[] | .email |= sub("@old\\.com$"; "@new.com")' users.json
```

## 6. Streaming Semantics

### 6.1 Single-document JSON (non-NDJSON)

Event-driven. Constant memory ≈ depth of nesting. Cannot parallelize because element byte boundaries are not known without sequential scanning.

Filter in streaming mode is restricted to **top-level shards**:
- Top-level array → each element is one shard.
- Top-level object → each value is one shard (keyed by its name).
- Top-level scalar → one shard.

Each shard is materialized into a `serde_json::Value`, handed to jaq as **one input value**, then dropped.

**Expression semantics in streaming mode:** the user writes the filter as if the input were a single shard, not the whole document. So for `big.json` whose top-level is `[{"id":1},…]`, the streaming-mode expression is `select(.id > 100)` or `{id, name}` — *not* `.[] | select(.id > 100)`. The CLI prints a one-line hint the first time streaming runs:

```
note: streaming mode evaluates your expression once per top-level element.
      write '.id' not '.[].id'  (use --materialize for whole-document semantics)
```

Expressions that need the full document (`length`, `sort_by`, `group_by`, aggregates) fail at static-check time before any I/O, with a suggestion to use `--materialize` or `--ndjson`.

### 6.2 NDJSON parallel pipeline

```
bytes → splitter(single) → bounded channel → worker pool(N) → reorder → writer
```

- Each line tagged with a sequence number.
- Worker = parse + transform + serialize; no shared state.
- Reorder = min-heap on sequence number, emit contiguously.
- Backpressure from bounded channel prevents unbounded memory growth.
- Malformed lines: worker emits an error record to stderr, main stream skips the line (per spec choice `ii`).

Expected throughput on 16-core NVMe: ~3 GB/s decompressed (bound by disk, not CPU).

### 6.3 Compression placement

- Decompression: streaming, before the splitter.
- Compression: streaming, after the writer.
- No temporary files at any stage.

## 7. Validation & Stats

Single pass covers syntax validation, Schema validation, and stats collection.

### 7.1 Syntax errors

Report location (byte offset + line + column) + expected token. NDJSON mode keeps going and reports per-line; single-document mode exits on first error with code 2.

### 7.2 JSON Schema

Uses `jsonschema` crate (Draft 4 through 2020-12). Because it consumes whole values, Schema validation applies to:
- NDJSON: each line.
- Top-level array JSON: each element (controlled by `--schema-applies-to elements|root`).
- Single JSON with `--materialize`: the whole document.

Violation report includes JSON Pointer path + violated keyword:

```
line 12: /address/zip: "ABC" does not match pattern "^[0-9]{5}$"
```

`--fail-fast` stops at first violation; default collects all. `--strict` promotes any Schema failure to a fatal exit (code 3); without `--strict`, Schema failures are reported to stderr but the process exits 0 as long as syntax is valid — useful for CI "warn but don't block" modes.

### 7.3 Stats output

`--stats` writes a human-readable summary to stderr. `--stats-json FILE` writes the same data as machine-readable JSON for CI consumption. Fields:

- input path, sizes (compressed / decompressed estimate), duration, throughput
- record count, valid/invalid split
- Schema pass/fail count, top-N violation paths
- top-level type distribution, max nesting depth, top-N top-level keys

## 8. Dependencies

| Purpose | Crate | Notes |
|---|---|---|
| Streaming JSON read/write | `struson` | event-based, constant memory |
| DOM values | `serde_json` | used per-shard in streaming filter + fully in `--materialize` |
| jq engine | `jaq-core`, `jaq-std`, `jaq-parse` | pure Rust; regex via `regex` |
| JSON Schema | `jsonschema` | Draft 4 / 6 / 7 / 2019-09 / 2020-12 |
| CLI | `clap` (derive) | subcommands + help |
| Channels / parallelism | `crossbeam-channel`, `rayon` | NDJSON pipeline |
| gzip | `flate2` (MultiGzDecoder) | streaming |
| zstd | `zstd` | streaming |
| Progress | `indicatif` | stderr, TTY-aware |
| Regex | `regex` | transitive via `jaq-std` |
| Errors | `thiserror` (lib) + `anyhow` (cli) | — |
| Test helpers | `assert_cmd`, `predicates`, `tempfile` | CLI e2e |
| Benchmarks | `criterion` | regression guard |
| Property tests | `proptest` | parser/writer invariants |

**MSRV:** Rust 1.75.
**Platforms:** Linux x86_64, Linux aarch64, macOS, Windows (CI builds all four).

## 9. Project Layout

```
jfmt/
├── Cargo.toml                 # workspace
├── crates/
│   ├── jfmt-core/
│   ├── jfmt-io/
│   └── jfmt-cli/
├── tests/                     # workspace-level end-to-end
├── benches/
├── docs/superpowers/specs/    # this file lives here
├── README.md
└── LICENSE
```

Full per-module tree is in section 4.1 / 4.2 / 4.3 above.

## 10. Testing Strategy

1. **Unit tests** per module — `#[cfg(test)]`. Boundary cases: escapes, Unicode, deep nesting, malformed input.
2. **Property tests** (`proptest`):
   - `parse(pretty(parse(x))) == parse(x)` — pretty preserves semantics.
   - `minify(pretty(x)) == minify(x)` — shapes equivalent.
   - NDJSON parallel output == NDJSON single-threaded output, exactly.
   - `validate(serde_json::to_string(v))` always passes.
3. **CLI end-to-end** (`assert_cmd`) — golden files, compression round-trips, error paths, exit codes.
4. **Large-file smoke** (`#[ignore]` by default, `--features big-tests` in CI):
   - Generate 1 GB synthetic NDJSON at runtime from a fixed seed.
   - Assert peak RSS < 200 MB (read via `procfs` / `GetProcessMemoryInfo`).
   - Assert multi-threaded output equals single-threaded output byte-for-byte.
   - No TB-scale test in CI; 1 GB + constant-memory property is the proxy.
5. **Benchmarks** (`criterion`, `benches/`) — retained baseline in CI, regression > 15 % blocks merge.

## 11. Milestones

Each milestone merges to `main` and ships a `0.0.x` preview release.

| M | Deliverable | Key work | Estimate |
|---|---|---|---|
| M1 | `jfmt pretty` / `minify`, single-thread, single-file | event parser/writer, CLI skeleton, I/O (gz/zst) | 1 week |
| M2 | `jfmt validate` + stats | syntax errors with location, `StatsCollector` | 3–4 days |
| M3 | NDJSON parallel pipeline | splitter / worker / reorder, backpressure, property tests | 1 week |
| M4a | `jfmt filter` streaming + NDJSON | embed jaq, top-level shard driver, static check, runtime guard | 1 week |
| M4b | `jfmt filter --materialize` | full-document jq semantics, memory budget check | 3–5 days |
| M5 | JSON Schema support | `jsonschema` integration, path reporting, combined validation | shipped (v0.0.6, 2026-04-25) |
| M6 | Release polish | progress bar, `cargo-dist`, README, multi-platform CI | 1 week |

| M1 ✓ | Shipped v0.0.1 on 2026-04-24 (tag `v0.0.1`) |
| M2 ✓ | Shipped v0.0.2 on 2026-04-25 (tag `v0.0.2`) |
| M3 ✓ | Shipped v0.0.3 on 2026-04-25 (tag `v0.0.3`) |
| M4a ✓ | Shipped v0.0.4 on 2026-04-25 (tag `v0.0.4`) |
| M4b ✓ | Shipped v0.0.5 on 2026-04-25 (tag `v0.0.5`) |
| M5 ✓ | Shipped v0.0.6 on 2026-04-25 |

**Total:** ~6–8 weeks full-time for v0.1.

## 12. Open Questions

None at spec-approval time. Questions that arose during brainstorming and were resolved:

- Scope of formats → JSON/NDJSON only (other formats in Phase 1b).
- Filter DSL → embed `jaq` for full jq syntax + regex.
- CLI shape → subcommand style (`jfmt pretty`, `jfmt filter`, …).
- Filter semantics → three modes (streaming / materialize / NDJSON parallel) with static check guiding the user.
- Output compression → auto by extension, symmetric with input.
- Bad-line handling in NDJSON → skip and log to stderr.
- No `--in-place` in Phase 1 (TB-file risk).
