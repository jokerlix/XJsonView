# jfmt

Streaming JSON / NDJSON formatter in Rust. Designed for **TB-scale** files
with **constant memory** (O(nesting depth), not O(file size)).

[![CI](https://github.com/jokerlix/XJsonView/actions/workflows/ci.yml/badge.svg)](https://github.com/jokerlix/XJsonView/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)

## Status

**M5 preview (v0.0.6)** — `pretty`, `minify`, `validate` (with
JSON Schema, `--strict`, `--materialize`, NDJSON parallel + per-element
streaming), and `filter` (streaming + NDJSON parallel + `--materialize`).
See [`docs/superpowers/specs/2026-04-23-jfmt-phase1-design.md`](docs/superpowers/specs/2026-04-23-jfmt-phase1-design.md)
for the Phase 1 roadmap.

## Install

### Prebuilt binaries (recommended)

```bash
# Linux / macOS
curl --proto '=https' --tlsv1.2 -LsSf \
  https://github.com/jokerlix/XJsonView/releases/latest/download/jfmt-installer.sh | sh
```

```powershell
# Windows
irm https://github.com/jokerlix/XJsonView/releases/latest/download/jfmt-installer.ps1 | iex
```

Or download a tarball directly from the
[Releases page](https://github.com/jokerlix/XJsonView/releases) and
extract `jfmt` to a directory on your `PATH`. Tarballs are provided for
Linux x86_64 + aarch64, macOS x86_64 + aarch64, and Windows x86_64.

### From source

```bash
cargo install --path crates/jfmt-cli
```

Or build locally:

```bash
cargo build --release
./target/release/jfmt --help
```

## Usage

### Pretty-print

```bash
jfmt pretty big.json                    # to stdout, 2-space indent
jfmt pretty big.json --indent 4         # 4-space indent
jfmt pretty big.json --tabs             # tab indent
jfmt pretty big.json.gz -o out.json     # decompress + pretty
jfmt pretty big.json -o out.json.zst    # pretty + zstd compress
cat x.json | jfmt pretty                # stdin → stdout
```

### Minify

```bash
jfmt minify pretty.json -o small.json
jfmt minify in.json.gz -o out.json.zst  # transcoding compression
```

### Validate

```bash
jfmt validate data.json                        # exit 0 if clean, 2 if not
jfmt validate data.json --stats                # human summary on stderr
jfmt validate data.json --stats-json out.json  # machine-readable summary
jfmt validate events.ndjson --ndjson           # per-line errors, keeps going
jfmt validate events.ndjson --ndjson --fail-fast
```

Stats include: record count (valid / invalid), top-level type distribution,
max nesting depth, and top-level key frequencies (capped at 1024 distinct
keys). JSON Schema validation lands in a later milestone.

### Validate — JSON Schema

```bash
# NDJSON: validate each line against the schema, report violations
jfmt validate --ndjson --schema schema.json data.ndjson

# Streaming top-level array: validate each element
jfmt validate --schema schema.json users.json

# Whole-document validation (e.g., to use `minItems` / `required` at root)
jfmt validate -m --schema schema.json config.json

# Strict CI mode: any failure exits non-zero
jfmt validate --ndjson --strict --schema schema.json events.ndjson
echo $?  # 3 if any record violated the schema
```

`--schema FILE` runs alongside the existing syntax validation. Mode +
top-level form decides what gets validated:

- `--ndjson` → each line is one record, validated independently.
- Default streaming + top-level array → each element validated
  (constant memory).
- Default streaming + top-level object/scalar → error; pass
  `--materialize` or `--ndjson`.
- `--materialize` (`-m`) → whole document validated as one value.
  Triggers a RAM budget pre-flight (file_size × 6, × 30 if compressed,
  abort unless `--force` when over 80 % RAM).

Violations stream to stderr immediately as they occur (TB-safe; no
unbounded accumulation). The schema's draft is auto-detected from
its `$schema` keyword (Draft 4 / 6 / 7 / 2019-09 / 2020-12).

**Exit codes:**

| Code | Meaning |
|---|---|
| 0 | success (or non-strict run with reported violations) |
| 1 | I/O failure, bad schema file, or non-array-root + `--schema` without `-m` |
| 2 | syntax error or clap usage error |
| 3 | schema violation under `--strict` |

Stats output (`--stats` / `--stats-json`) gains `schema_pass`,
`schema_fail`, and `top_violation_paths` (top 10 most-frequent
violated JSON Pointer paths).

### Filter

Apply a jq expression to JSON or NDJSON. Streaming mode is the default;
each top-level element of an array (or each value of an object) is one
*shard*, and the expression runs once per shard.

```bash
# Streaming: filter elements of an array
jfmt filter 'select(.x > 1)' big.json

# NDJSON: full jq semantics per line, in parallel
jfmt filter --ndjson 'select(.level == "error")' logs.ndjson

# Pretty-print streaming output
jfmt filter --pretty --indent 4 '.user' file.json

# Strict mode: runtime jq errors abort with exit 1
jfmt filter --ndjson --strict '.x + 1' may-have-bad-types.ndjson
```

Streaming-mode rules:

- Top-level array → output array (drop / expand per shard).
- Top-level object → output object (drop key on 0 outputs; multi-output is an error).
- Top-level scalar → 0 or 1 output (multi-output is an error).
- Aggregate jq builtins (`length`, `sort_by`, `group_by`, `add`, `min`, `max`, `unique`, …) are
  rejected at compile time. Use `--ndjson` for per-line full semantics, or `--materialize` for
  full-document semantics.

**Full-jq mode (`-m` / `--materialize`):**

```bash
# Allows aggregates: length, sort_by, group_by, add, min, max, unique
jfmt filter -m 'sort_by(.x)' file.json
jfmt filter -m 'length' file.json
jfmt filter -m '.[]' file.json   # multi-value stream output

# Override the RAM budget check (file_size * 6, file_size * 30 if compressed)
jfmt filter -m --force 'sort_by(.x)' big.json
```

`-m` loads the entire input into memory and runs the jq expression
with full semantics. For file inputs, jfmt estimates peak memory and
aborts unless `--force` is set or the estimate is under 80 % of total
RAM. stdin input skips the check (the `-m` flag itself is the user's
"I have enough memory" promise). `-m` and `--ndjson` are mutually
exclusive.

Multiple jq output values are emitted as a JSON-value stream
(separated by `\n`, or `\n\n` with `--pretty`). No trailing newline,
matching jfmt's `pretty` / `minify` output. This intentionally
differs from `jq -c`'s trailing newline.

Object output keys are emitted in alphabetical order (jaq round-trip
through `serde_json::Map` does not preserve insertion order). Use
`--ndjson` if your downstream consumer cares about original key order.

The streaming-mode hint prints to stderr on first invocation; pipe with
`2>/dev/null` to silence.

### Parallelism

The `--ndjson` pipeline runs splitter → N workers → reorder on
separate threads. Control with the global `--threads` flag:

```bash
jfmt --threads 8 pretty   --ndjson big.ndjson      # 8 workers
jfmt --threads 1 validate --ndjson big.ndjson      # force serial
jfmt minify --ndjson big.ndjson                    # default = physical cores
```

Output is always written in input order. `--threads` is silently
ignored in single-document mode.

## Exit codes

| Code | Meaning |
|------|---------|
| 0    | Success |
| 1    | I/O or argument error (file not found, bad flags) |
| 2    | JSON syntax error in input |

## Architecture

Three crates:

- [`jfmt-core`](crates/jfmt-core) — streaming parser + writers, zero I/O
- [`jfmt-io`](crates/jfmt-io) — file/stdin/stdout + gz/zst stream adapters
- [`jfmt-cli`](crates/jfmt-cli) — `jfmt` binary

## License

MIT OR Apache-2.0
