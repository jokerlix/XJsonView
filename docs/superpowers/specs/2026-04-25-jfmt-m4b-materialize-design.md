# jfmt M4b — `--materialize` Filter (Design)

**Status:** approved 2026-04-25 (brainstormed with user)
**Predecessor specs:** Phase 1 design §5.1, §6.1; M4a design `docs/superpowers/specs/2026-04-25-jfmt-m4a-filter-design.md`
**Predecessor plan:** `docs/superpowers/plans/2026-04-25-jfmt-m4a-filter.md` (shipped as `v0.0.4`)
**Target tag:** `v0.0.5`

## 1. Scope

M4b adds full-jq semantics to `jfmt filter` via a new flag:

```
jfmt filter -m | --materialize EXPR [INPUT] [-o OUTPUT]
            [--force] [--strict] [--pretty | --compact] [--indent N]
```

- Loads the entire input document into a `serde_json::Value` and runs the
  jaq filter against it once, with full jq semantics (aggregates,
  `sort_by`, `group_by`, `length`, …).
- Outputs a multi-value stream (jq default style): each jq output value is
  written as an independent JSON document, separated by `\n` in compact mode
  and `\n\n` in pretty mode.
- File input: estimate peak RAM = `file_size × 6` (× 5 again if compressed).
  Reject when the estimate exceeds 80 % of total system RAM unless
  `--force` is passed.
- stdin input: skip the RAM check entirely.
- `-m` and `--ndjson` are mutually exclusive.

### 1.1 Out of scope (M4b)

- Progress bars (one-shot load — no meaningful progress signal).
- jq multi-document streams (`input` / `inputs`) — still rejected.
- jq module loading (`-L`, `import`) — Phase 1 permanent OOS.

## 2. Design decisions

| # | Decision |
|---|---|
| D1 | Multi-value output is a **stream of JSON values**, not a wrapping array. Matches `jq` CLI default; pipeline-friendly (each value is parseable on its own). |
| D2 | Static check has two blacklist groups: aggregate names (allowed in `-m`) and multi-input names (`input` / `inputs`, **always** rejected). |
| D3 | stdin + `-m` proceeds without a RAM check. The `-m` flag itself is the user's "I have enough memory" promise; stdin gives no size signal worth checking against. |
| D4 | No trailing newline after the last output value, matching `jfmt minify` / `jfmt pretty` conventions (and intentionally diverging from `jq -c`). Documented in `--help`. |

## 3. Module layout

### 3.1 New file (`jfmt-core`)

| Path | Responsibility |
|---|---|
| `crates/jfmt-core/src/filter/materialize.rs` | `run_materialize(reader, writer, &Compiled, FilterOutput, FilterOptions) -> Result<MaterializeReport, FilterError>`. Loads → jaq runs once → writes value stream. |

### 3.2 Modified files

| Path | Change |
|---|---|
| `crates/jfmt-core/src/filter/static_check.rs` | Public `Mode { Streaming, Materialize }` enum; `check()` takes `mode`; aggregate group skipped in `Materialize`; multi-input group always enforced. |
| `crates/jfmt-core/src/filter/compile.rs` | `compile()` takes `mode: Mode`; passes through to `static_check::check`. All call sites in `mod.rs` updated. |
| `crates/jfmt-core/src/filter/mod.rs` | Re-export `run_materialize`, `Mode`. New `FilterError::MultiInput` and `FilterError::BudgetExceeded` variants. |
| `crates/jfmt-cli/src/cli.rs` | `FilterArgs` gains `materialize: bool` (`-m` / `--materialize`) and `force: bool` (clap `requires = "materialize"`). `materialize.conflicts_with("ndjson")`. |
| `crates/jfmt-cli/src/commands/filter.rs` | New branch when `args.materialize`: estimate RAM if input is a file, abort with `BudgetExceeded` unless `--force`, otherwise call `run_materialize`. |
| `crates/jfmt-cli/src/main.rs` | `classify` maps `MultiInput` → `SyntaxError` (exit 2). |
| `Cargo.toml` (workspace) | Add `sysinfo` (version pinned by Task 1's spike). |
| `crates/jfmt-cli/Cargo.toml` | Pull `sysinfo`. |

## 4. Core data flow (single-document materialize)

```
Read(file or stdin) → serde_json::from_reader → Value
                                                  ↓
                          compiled.inner.filter.run((Ctx, Val))
                                                  ↓
                                            0 / 1 / N Values
                                                  ↓
                                  for each: serialize as one JSON doc
                                  compact: '\n' between docs
                                  pretty:  '\n\n' between docs
                                  no trailing newline after the last
```

### 4.1 Multi-value output framing

| Mode | 0 outputs | 1 output | N outputs |
|---|---|---|---|
| Compact | empty file | `<value>` | `<v1>\n<v2>\n…\n<vN>` (no trailing `\n`) |
| Pretty | empty file | `<formatted>` | `<f1>\n\n<f2>\n\n…\n\n<fN>` (no trailing `\n`) |

Pretty mode reuses `crate::writer::PrettyWriter` (the existing jfmt
formatter) for parity with `jfmt pretty`. Each output value is written
through a fresh `PrettyWriter` instance over the same underlying
`Write`, so the depth stack / indent buffer start clean per value.
Compact mode reuses `crate::writer::MinifyWriter` symmetrically.

### 4.2 Static check + runtime guard

`compile.rs`:

1. Lex + parse → `Term<&str>`.
2. `static_check::check(&term, mode)`:
   - Mode `Streaming` (M4a behaviour): reject aggregate **and** multi-input names.
   - Mode `Materialize`: reject multi-input names only.
3. `Loader::load` + `Compiler::compile` → `Filter`.

`runtime::run_one` is unchanged: empty `inputs` iterator, so any
multi-input usage that bypassed the check (shouldn't happen with
explicit static check, but defence in depth) raises a jaq runtime
error mapped to `FilterError::Runtime`.

### 4.3 RAM budget

Implemented in `crates/jfmt-cli/src/commands/filter.rs` (CLI-layer concern).

```rust
fn estimate_peak_ram_bytes(input: &jfmt_io::InputSpec) -> Option<u64> {
    let path = input.path.as_ref()?;
    let meta = std::fs::metadata(path).ok()?;
    let file_size = meta.len();
    let multiplier: u64 = match detect_compression(input) {
        jfmt_io::Compression::None => 6,
        jfmt_io::Compression::Gzip | jfmt_io::Compression::Zstd => 5 * 6, // 30
    };
    Some(file_size.saturating_mul(multiplier))
}

fn budget_ok(estimate: u64, total_ram: u64) -> bool {
    // 80 % of total RAM
    estimate < total_ram / 5 * 4
}
```

`detect_compression` already exists in `jfmt-io`'s open-input path; expose
it (or re-derive from extension) for this caller.

If `estimate.is_some()` and `!budget_ok(...)` and `!args.force`:

```
jfmt: estimated peak memory <X> exceeds 80% of total RAM (<Y>);
      rerun with --force to override.
```

Exit code 1 (`InputError`).

stdin path: `estimate_peak_ram_bytes` returns `None` → skip the check
unconditionally (D3).

## 5. Errors and exit codes

| Variant | When | Default exit | `--strict` exit |
|---|---|---|---|
| `Aggregate { name }` | static-check (Streaming mode only) | 2 | 2 |
| `MultiInput { name }` | static-check, any mode | 2 | 2 |
| `BudgetExceeded { estimate_bytes, total_ram_bytes }` | CLI pre-flight, no `--force` | 1 | 1 |
| `Runtime { … }` | jaq runtime error | stderr report, exit 0 | exit 1 |
| `Parse { … }` | jaq parser failure | 2 | 2 |
| `Io(…)` | reader/writer | 1 | 1 |

`OutputShape` (M4a, object/scalar N>1) **does not** appear in materialize
mode — its data flow does not use `OutputShaper`.

## 6. CLI

```
jfmt filter EXPR [INPUT]
    [-o, --output FILE]
    [-m, --materialize]      # full-jq mode; conflicts with --ndjson
    [--force]                # skip RAM budget check; requires --materialize
    [--ndjson]               # conflicts with --materialize
    [--strict]
    [--compact | --pretty]
    [--indent N]
```

clap relations:
- `materialize.conflicts_with("ndjson")`
- `force.requires("materialize")`

## 7. Testing strategy

### 7.1 jfmt-core unit tests

- `static_check.rs`: each aggregate name passes under `Mode::Materialize`;
  each multi-input name still rejected.
- `materialize.rs`: 0 / 1 / N outputs framed correctly (no trailing newline);
  pretty vs compact separators; type-error path produces `Runtime`.

### 7.2 jfmt-core integration (`tests/filter_materialize.rs`)

- `length` on an array returns its length.
- `sort_by(.x)` reorders an array of objects.
- `group_by(.k)` produces grouped output.
- `.[]` on a 3-element array produces 3 separate JSON values in the
  output stream.
- `--strict` flips runtime error from collected to fatal.

### 7.3 jfmt-cli e2e (append to `tests/cli_filter.rs`)

- `jfmt filter -m 'length' file.json` → exit 0, stdout = number.
- `jfmt filter -m '.[]' array.json` → multi-line output, count matches input length.
- `jfmt filter -m --ndjson '.'` → clap conflict, exit 2.
- `jfmt filter --force '.'` → clap error (requires `--materialize`).
- **RAM-budget trip**: rather than constructing a multi-GB fixture, refactor
  the budget check into a pure function `budget_ok(estimate, total_ram)` plus
  a thin shell that queries `sysinfo`. The unit / e2e test exercises the
  pure function with synthetic numbers; the shell function gets a smoke
  e2e (`-m` on a tiny fixture should always pass with `--force` removed).
- stdin + `-m`: pipe a small JSON document; assert no budget message
  appears in stderr.

### 7.4 Out of scope (testing)

- Multi-GB OOM behaviour (no CI support).
- jaq's own jq-language coverage of aggregates (we trust upstream).

## 8. Risks and mitigations

1. **`sysinfo` MSRV.** Same risk class as jaq in M4a. Plan Task 1 spikes
   `sysinfo` against MSRV 1.75 and pins. Fallback if sysinfo doesn't fit:
   hand-rolled syscall (Windows `GlobalMemoryStatusEx`, Linux
   `/proc/meminfo`, macOS `host_statistics`) — more code but zero deps.
2. **`6×` heuristic accuracy.** `serde_json::Value` overhead varies with
   data shape (lots of small strings amplify more). 6 is the spec's number
   and we keep it. If e2e shows it's clearly low for representative
   inputs, bump to 8× — but only once, not iteratively. Documented in
   `--help` as "rough heuristic, not a guarantee."
3. **No trailing newline.** Differs from `jq -c`. `--help` and README must
   call this out so users running `jfmt filter -m '…' | wc -l` understand
   the off-by-one.

## 9. Rejected alternatives

- **Wrap multi-output in a JSON array (`[v1, v2, …]`).** Always-parseable
  but breaks the single-value case (`-m '.x'` → `[123]` instead of
  `123`); user expectation is `jq` semantics.
- **Reject N>1 outputs in materialize mode (mirror M4a streaming
  scalar/object behaviour).** Too restrictive — `to_entries[]` is a
  common pattern.
- **Auto-fallback from streaming to materialize on aggregate.**
  Silently consuming GiBs of RAM is more dangerous than a clear error;
  user must opt in.

## 10. Acceptance criteria

- `cargo test --workspace` green.
- `cargo clippy --workspace --all-targets -- -D warnings` clean.
- `jfmt filter -m 'length' file.json` works.
- `jfmt filter -m '.[]' array.json` produces a value stream.
- `jfmt filter --force` without `-m` errors at clap level.
- `jfmt filter -m --ndjson` errors at clap level.
- README updated with a `### Filter — full jq mode` subsection (or
  inline addendum to the existing `### Filter` block).
- Phase 1 spec marked: M4b shipped as `v0.0.5`.

## Annex B — sysinfo API mapping (frozen by Task 1 spike)

- Version: sysinfo=0.30.13.
- Constructor: `sysinfo::System::new()`.
- Refresh: `sys.refresh_memory()` (required before reading totals).
- Total RAM: `sys.total_memory() -> u64` returning bytes.
- Unit: bytes.

The `cli/commands/filter.rs::system_total_ram_bytes()` helper calls
this exact sequence.
