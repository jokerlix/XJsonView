# jfmt

Streaming JSON / NDJSON formatter in Rust. Designed for **TB-scale** files
with **constant memory** (O(nesting depth), not O(file size)).

## Status

**M1 preview (v0.0.1)** — `pretty` and `minify` subcommands over plain,
gzip, and zstd JSON. See
[`docs/superpowers/specs/2026-04-23-jfmt-phase1-design.md`](docs/superpowers/specs/2026-04-23-jfmt-phase1-design.md)
for the Phase 1 roadmap (validation, filtering, NDJSON parallel pipeline
coming in M2–M6).

## Install

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
