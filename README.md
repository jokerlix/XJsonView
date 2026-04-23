# jfmt

Streaming JSON / NDJSON formatter. Built for **TB-scale** files with **constant memory**.

## Status

M1 preview: `pretty` and `minify` subcommands on plain / gzip / zstd JSON files.

See [`docs/superpowers/specs/2026-04-23-jfmt-phase1-design.md`](docs/superpowers/specs/2026-04-23-jfmt-phase1-design.md)
for the full Phase 1 design.

## Build

```bash
cargo build --release
./target/release/jfmt --help
```

## Usage

```bash
jfmt pretty big.json                 # stdout
jfmt pretty big.json.gz -o out.json  # decompress + pretty
jfmt minify out.json -o out.min.json.zst
cat tiny.json | jfmt pretty --indent 4
```

## License

MIT OR Apache-2.0
