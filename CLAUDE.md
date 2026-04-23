# CLAUDE.md — jfmt (XJsonView repo)

Project memory for Claude sessions working in this repo. Read this first.

## Project identity

**jfmt** is a Rust CLI + core library for formatting, minifying, validating, and
filtering JSON/NDJSON at **TB scale** with **constant memory**. The repo
directory is named `XJsonView` because future Phase 3 adds a GUI viewer; the
crate and binary are named `jfmt`.

- **Language:** Rust 1.75+ (MSRV pinned in `rust-toolchain.toml`)
- **Edition:** 2021
- **License:** MIT OR Apache-2.0
- **Owner:** lizhongwei (<lzw1003362793@gmail.com>)

## Authoritative documents

Before proposing changes, read the relevant one:

- **Spec (what we're building and why):** `docs/superpowers/specs/2026-04-23-jfmt-phase1-design.md`
- **Active implementation plan:** `docs/superpowers/plans/2026-04-23-jfmt-m1-core-pretty-minify.md`

The spec is the contract. The plan is the execution script. If a
conflict surfaces, the spec wins — update the plan, not the spec,
without re-approval.

## Workspace layout

```
crates/
  jfmt-core/   streaming parser + writers, zero I/O (Read/Write only)
  jfmt-io/    file/stdin/stdout + gzip/zstd adapters
  jfmt-cli/   the `jfmt` binary (clap)
docs/superpowers/
  specs/       approved design documents (do not rewrite without approval)
  plans/       implementation plans, one per milestone
```

## Roadmap

Phase 1 ships over 6 milestones. Each milestone tags a `0.0.x` preview.

| M | Deliverable | Status |
|---|---|---|
| M1 | `pretty` / `minify` + core + I/O | in progress — plan written |
| M2 | `validate` + stats | not started |
| M3 | NDJSON parallel pipeline | not started |
| M4 | `filter` (streaming + materialize, embedded jaq) | not started |
| M5 | JSON Schema support | not started |
| M6 | release polish + cargo-dist | not started |

Each milestone gets its own plan document when its predecessor ships.
**Do not plan ahead more than one milestone** — wait for the learnings.

## Iteration workflow

Use the `jfmt-iterate` skill in `.claude/skills/jfmt-iterate/`:

1. The user names a phase (e.g. "scaffolding", "task 3", "all of M1").
2. Work through **only that phase's tasks**, then stop and report.
3. Wait for the user to name the next phase before continuing.

**Never batch beyond what the user asked for.** Running Tasks 1–5 when the
user said "task 1" is a plan violation.

## Coding conventions

- **TDD:** failing test → minimal impl → passing test → commit. The plan
  spells this out step-by-step. Don't skip the failing-test step; it proves
  the test actually exercises the code.
- **Commits:** small, one logical change per commit. Use the prefixes the
  plan uses (`feat(core):`, `feat(io):`, `feat(cli):`, `test(...)`, `docs:`,
  `chore:`, `ci:`). Each plan task ends with a commit step.
- **Modules stay focused:** if a `.rs` file grows past ~400 lines, split
  along responsibility boundaries (see how `writer/` is already split).
- **Error handling:**
  - Libraries (`jfmt-core`, `jfmt-io`) return `thiserror`-based enums.
  - CLI (`jfmt-cli`) uses `anyhow::Result` at the top level and downcasts
    to map to exit codes in `exit.rs`.
- **No `println!` in libraries.** Progress / user messages are CLI-layer.
- **No `unwrap()` in non-test code.** Use `?` and real errors.
- **Clippy clean:** `cargo clippy --workspace --all-targets -- -D warnings`
  is enforced in CI.
- **Rustfmt:** `cargo fmt --all` before commit. Default config.

## Testing expectations

- **Unit tests** live next to the code (`#[cfg(test)] mod tests`).
- **Property tests** (`proptest`) live in `crates/<crate>/tests/` and guard
  invariants (round-trip equivalence, parallel == serial output).
- **CLI end-to-end tests** live in `crates/jfmt-cli/tests/` and use
  `assert_cmd` + golden fixtures in `tests/fixtures/`.
- **Large-file tests** are gated behind `--features big-tests` and only run
  in CI on demand. Never generate > 1 GB test data in a default test run.
- Every new behavior gets at least one unit test AND one CLI test.

## Common commands

All from repo root (`D:\code\XJsonView`).

```bash
cargo build --workspace                  # compile everything
cargo test --workspace                   # run all tests
cargo test -p jfmt-core                  # one crate
cargo test -p jfmt-cli --test cli_pretty # one e2e file
cargo fmt --all                          # format
cargo clippy --workspace --all-targets -- -D warnings
cargo run -p jfmt-cli -- pretty some.json
```

## Git hygiene

- `main` is the only long-lived branch in M1.
- Tag each shipped milestone: `v0.0.1` for M1, `v0.0.2` for M2, etc.
- Never force-push. Never amend after push.
- Commit messages in English, imperative mood.

## What NOT to do

- Don't rewrite the spec without the user's explicit approval.
- Don't expand M1 scope. `--sort-keys`, `--array-per-line`, `--ndjson`
  fast path, progress bars — all defer to later plans.
- Don't add dependencies not listed in the workspace `[workspace.dependencies]`
  without flagging it.
- Don't skip the TDD "write failing test → verify it fails" step. That step
  is load-bearing; failures here have caught bugs in the past.
- Don't run multiple plan tasks in one iteration unless the user explicitly
  says so. See the `jfmt-iterate` skill.

## Dialog preferences

- Answer in Chinese when the user writes in Chinese; English otherwise.
- Be concise. No preamble. Show code, show results.
- When something blocks progress, say so clearly and ask — don't guess.
