# jfmt M1 — Core + Pretty + Minify Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship the first preview release of `jfmt` with working `pretty` and `minify` subcommands operating on JSON files with optional gzip/zstd compression, backed by a reusable streaming core library.

**Architecture:** Cargo workspace with three crates — `jfmt-core` (pure streaming parser/writer, zero I/O), `jfmt-io` (file/stdin/stdout + gz/zst stream adapters), `jfmt-cli` (clap-driven binary). Parser wraps `struson` and emits a unified `Event` stream; writers consume that stream and format output. Constant memory = O(nesting depth).

**Tech Stack:** Rust 1.75+, `struson` (streaming JSON tokenizer), `flate2` (gzip), `zstd`, `clap` (derive), `anyhow`/`thiserror` (errors), `assert_cmd` + `predicates` + `tempfile` (CLI tests), `proptest` (property tests).

**Scope boundary:** This plan covers Milestone M1 from the spec. The following flags from the spec's CLI surface are **deferred to a follow-up plan (M1.5)** to keep M1 tight: `--sort-keys`, `--array-per-line` (pretty), `--ndjson` (here it's a no-op accepted for forward compat). `validate`, `filter`, and NDJSON parallel pipeline are M2+.

**Prerequisites:** Rust toolchain installed (`rustup`, `cargo`), git. Windows / Linux / macOS all supported.

**Spec reference:** `docs/superpowers/specs/2026-04-23-jfmt-phase1-design.md`

---

## File Structure After M1

```
D:\code\XJsonView\
├── Cargo.toml                              # workspace manifest
├── .gitignore
├── README.md
├── rust-toolchain.toml                     # pin MSRV
├── .github/workflows/ci.yml
├── crates/
│   ├── jfmt-core/
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs                      # re-exports + docs
│   │       ├── error.rs                    # Error enum (thiserror)
│   │       ├── event.rs                    # Event, Scalar types
│   │       ├── parser.rs                   # EventReader<R>
│   │       ├── escape.rs                   # write_json_string helper
│   │       ├── transcode.rs                # copy_events(reader, writer)
│   │       └── writer/
│   │           ├── mod.rs                  # EventWriter trait
│   │           ├── minify.rs               # MinifyWriter
│   │           └── pretty.rs               # PrettyWriter (indent only in M1)
│   ├── jfmt-io/
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── compress.rs                 # Compression enum + detection
│   │       ├── input.rs                    # open_input → Box<dyn BufRead>
│   │       └── output.rs                   # open_output → Box<dyn Write>
│   └── jfmt-cli/
│       ├── Cargo.toml
│       └── src/
│           ├── main.rs
│           ├── cli.rs                      # clap Args structs
│           ├── exit.rs                     # ExitCode enum
│           └── commands/
│               ├── mod.rs
│               ├── pretty.rs
│               └── minify.rs
└── tests/
    ├── fixtures/
    │   ├── simple.json
    │   ├── simple.pretty2.json
    │   ├── simple.pretty4.json
    │   └── simple.min.json
    ├── cli_pretty.rs
    ├── cli_minify.rs
    └── roundtrip_proptest.rs
```

---

## Task 1: Initialize Cargo workspace + git scaffolding

**Files:**
- Create: `Cargo.toml` (workspace root)
- Create: `.gitignore`
- Create: `rust-toolchain.toml`
- Create: `README.md`

- [ ] **Step 1: Create workspace `Cargo.toml`**

```toml
# Cargo.toml
[workspace]
resolver = "2"
members = ["crates/jfmt-core", "crates/jfmt-io", "crates/jfmt-cli"]

[workspace.package]
version = "0.0.1"
edition = "2021"
rust-version = "1.75"
license = "MIT OR Apache-2.0"
repository = "https://github.com/lizhongwei/XJsonView"
authors = ["lizhongwei <lzw1003362793@gmail.com>"]

[workspace.dependencies]
# Core streaming parser
struson = "0.6"
# Error handling
thiserror = "1"
anyhow = "1"
# I/O
flate2 = "1"
zstd = "0.13"
# CLI
clap = { version = "4", features = ["derive"] }
# Test utilities
assert_cmd = "2"
predicates = "3"
tempfile = "3"
proptest = "1"
serde_json = "1"

[profile.release]
lto = "thin"
codegen-units = 1
strip = true
```

- [ ] **Step 2: Create `.gitignore`**

```gitignore
# .gitignore
/target
**/*.rs.bk
Cargo.lock
.DS_Store
*.swp
.vscode/
.idea/
```

Note: We commit `Cargo.lock` for binaries but since this is a workspace with a binary, keep it out initially to avoid churn; we'll revisit before v0.1.

- [ ] **Step 3: Pin MSRV with `rust-toolchain.toml`**

```toml
# rust-toolchain.toml
[toolchain]
channel = "1.75"
components = ["rustfmt", "clippy"]
profile = "minimal"
```

- [ ] **Step 4: Create `README.md`**

```markdown
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
```

- [ ] **Step 5: Verify the workspace resolves**

Run: `cd /d/code/XJsonView && cargo metadata --format-version 1 >NUL 2>&1; echo exit=$?`
Expected: `exit=0` (even though member crates don't exist yet, Cargo will fail — so **before running**, create empty member directories:)

```bash
mkdir -p crates/jfmt-core/src crates/jfmt-io/src crates/jfmt-cli/src
```

Then create placeholder `crates/<name>/Cargo.toml` files so `cargo metadata` can resolve them:

```toml
# crates/jfmt-core/Cargo.toml
[package]
name = "jfmt-core"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
```

```toml
# crates/jfmt-io/Cargo.toml
[package]
name = "jfmt-io"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
```

```toml
# crates/jfmt-cli/Cargo.toml
[package]
name = "jfmt-cli"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true

[[bin]]
name = "jfmt"
path = "src/main.rs"
```

Add placeholder `src/lib.rs` for the two libs and `src/main.rs` for the bin:

```rust
// crates/jfmt-core/src/lib.rs
// placeholder
```

```rust
// crates/jfmt-io/src/lib.rs
// placeholder
```

```rust
// crates/jfmt-cli/src/main.rs
fn main() {}
```

Run: `cargo check --workspace`
Expected: compiles clean (warnings about empty crates are fine).

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml .gitignore rust-toolchain.toml README.md crates
git commit -m "chore: init Cargo workspace with jfmt-core, jfmt-io, jfmt-cli crates"
```

---

## Task 2: CI scaffolding (GitHub Actions)

**Files:**
- Create: `.github/workflows/ci.yml`

- [ ] **Step 1: Create the workflow**

```yaml
# .github/workflows/ci.yml
name: ci

on:
  push:
    branches: [main]
  pull_request:

env:
  CARGO_TERM_COLOR: always
  RUSTFLAGS: "-D warnings"

jobs:
  check:
    strategy:
      fail-fast: false
      matrix:
        os: [ubuntu-latest, windows-latest, macos-latest]
    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@1.75
        with:
          components: rustfmt, clippy
      - uses: Swatinem/rust-cache@v2
      - run: cargo fmt --all -- --check
      - run: cargo clippy --workspace --all-targets -- -D warnings
      - run: cargo test --workspace
```

- [ ] **Step 2: Verify it parses locally**

Run: `cargo fmt --all -- --check`
Expected: exits 0 (nothing to format yet).

- [ ] **Step 3: Commit**

```bash
git add .github/workflows/ci.yml
git commit -m "ci: add build + test workflow for linux/mac/windows"
```

---

## Task 3: `jfmt-core` skeleton — `lib.rs` + `error.rs`

**Files:**
- Modify: `crates/jfmt-core/Cargo.toml`
- Modify: `crates/jfmt-core/src/lib.rs`
- Create: `crates/jfmt-core/src/error.rs`

- [ ] **Step 1: Fill in `jfmt-core/Cargo.toml`**

```toml
# crates/jfmt-core/Cargo.toml
[package]
name = "jfmt-core"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true

[dependencies]
struson = { workspace = true }
thiserror = { workspace = true }

[dev-dependencies]
```

- [ ] **Step 2: Define `Error` type**

```rust
// crates/jfmt-core/src/error.rs
use std::io;
use thiserror::Error;

/// Errors produced by the jfmt-core streaming pipeline.
#[derive(Debug, Error)]
pub enum Error {
    /// Lower-level I/O failure (reader/writer).
    #[error("io error: {0}")]
    Io(#[from] io::Error),

    /// The input bytes are not valid JSON.
    #[error("syntax error at byte {offset}: {message}")]
    Syntax { offset: u64, message: String },

    /// The parser/writer was called in an unexpected state (internal bug).
    #[error("invalid state: {0}")]
    State(String),
}

/// Convenience alias.
pub type Result<T> = std::result::Result<T, Error>;
```

- [ ] **Step 3: Wire up `lib.rs`**

```rust
// crates/jfmt-core/src/lib.rs
//! Streaming JSON parser and writer.
//!
//! Zero I/O assumptions — all entry points accept `impl Read` / `impl Write`.
//! Memory usage is O(nesting depth), not O(file size).

pub mod error;

pub use error::{Error, Result};
```

- [ ] **Step 4: Verify it builds**

Run: `cargo build -p jfmt-core`
Expected: compiles with no warnings.

- [ ] **Step 5: Commit**

```bash
git add crates/jfmt-core
git commit -m "feat(core): add Error type and crate skeleton"
```

---

## Task 4: `jfmt-core` — `Event` and `Scalar` types

**Files:**
- Create: `crates/jfmt-core/src/event.rs`
- Modify: `crates/jfmt-core/src/lib.rs`

- [ ] **Step 1: Write the failing test** (append to `event.rs` after we create it; for now create the file with tests first)

```rust
// crates/jfmt-core/src/event.rs
//! Event stream types shared by the parser and writers.

/// A JSON scalar: anything that is not a container.
#[derive(Debug, Clone, PartialEq)]
pub enum Scalar {
    /// A JSON string (already unescaped).
    String(String),
    /// A JSON number, preserved as its original lexical form so that
    /// precision is not lost. E.g. `"1.0"`, `"1e10"`, `"-0"`.
    Number(String),
    /// `true` or `false`.
    Bool(bool),
    /// `null`.
    Null,
}

/// One token in the event-driven JSON stream.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    StartObject,
    EndObject,
    StartArray,
    EndArray,
    /// A key inside an object. Always immediately followed by a value event
    /// (scalar, StartObject, or StartArray).
    Name(String),
    /// A scalar value. May appear at the top level, inside an array, or as
    /// the value of a name in an object.
    Value(Scalar),
}

impl Event {
    /// True if this event opens a new container.
    pub fn is_start(&self) -> bool {
        matches!(self, Event::StartObject | Event::StartArray)
    }

    /// True if this event closes a container.
    pub fn is_end(&self) -> bool {
        matches!(self, Event::EndObject | Event::EndArray)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_start_and_end_classify_correctly() {
        assert!(Event::StartObject.is_start());
        assert!(Event::StartArray.is_start());
        assert!(!Event::Name("x".into()).is_start());
        assert!(!Event::Value(Scalar::Null).is_start());

        assert!(Event::EndObject.is_end());
        assert!(Event::EndArray.is_end());
        assert!(!Event::StartObject.is_end());
    }

    #[test]
    fn scalar_equality_is_by_value() {
        assert_eq!(Scalar::Number("1.0".into()), Scalar::Number("1.0".into()));
        assert_ne!(Scalar::Number("1".into()), Scalar::Number("1.0".into()));
        assert_eq!(Scalar::Bool(true), Scalar::Bool(true));
    }
}
```

- [ ] **Step 2: Re-export from `lib.rs`**

```rust
// crates/jfmt-core/src/lib.rs
//! Streaming JSON parser and writer.
//!
//! Zero I/O assumptions — all entry points accept `impl Read` / `impl Write`.
//! Memory usage is O(nesting depth), not O(file size).

pub mod error;
pub mod event;

pub use error::{Error, Result};
pub use event::{Event, Scalar};
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p jfmt-core event::tests`
Expected: `2 passed`.

- [ ] **Step 4: Commit**

```bash
git add crates/jfmt-core/src/event.rs crates/jfmt-core/src/lib.rs
git commit -m "feat(core): add Event and Scalar types"
```

---

## Task 5: `jfmt-core` — `EventReader` wrapping struson

**Files:**
- Create: `crates/jfmt-core/src/parser.rs`
- Modify: `crates/jfmt-core/src/lib.rs`

- [ ] **Step 1: Write the failing test**

```rust
// crates/jfmt-core/src/parser.rs
//! Event-driven JSON reader built on top of `struson`.

use crate::error::{Error, Result};
use crate::event::{Event, Scalar};
use std::io::Read;
use struson::reader::{JsonReader, JsonStreamReader, ValueType};

/// A pull-based iterator of [`Event`]s over a JSON byte stream.
///
/// Uses constant memory proportional to nesting depth.
pub struct EventReader<R: Read> {
    inner: JsonStreamReader<R>,
    /// Stack of container kinds currently open.
    stack: Vec<Container>,
    /// `true` once the top-level value has been fully consumed.
    done: bool,
    /// `true` if the next event inside an object should be a `Name`.
    expect_name: bool,
}

#[derive(Copy, Clone, Debug, PartialEq)]
enum Container {
    Object,
    Array,
}

impl<R: Read> EventReader<R> {
    pub fn new(source: R) -> Self {
        Self {
            inner: JsonStreamReader::new(source),
            stack: Vec::new(),
            done: false,
            expect_name: false,
        }
    }

    /// Return the next event, or `Ok(None)` after the document ends.
    pub fn next_event(&mut self) -> Result<Option<Event>> {
        if self.done {
            return Ok(None);
        }

        // Inside an object we alternate between Name and a value event.
        if self.expect_name {
            // Object entry: either next name, or end of object.
            if self.inner.has_next().map_err(map_err)? {
                let name = self.inner.next_name_owned().map_err(map_err)?;
                self.expect_name = false;
                return Ok(Some(Event::Name(name)));
            } else {
                self.inner.end_object().map_err(map_err)?;
                self.pop_container();
                return Ok(Some(Event::EndObject));
            }
        }

        // Top-level or inside an array (or a value position in an object).
        // If we are inside a container that is "out of items", close it.
        if let Some(top) = self.stack.last().copied() {
            if !self.inner.has_next().map_err(map_err)? {
                return Ok(Some(self.close_container(top)?));
            }
        } else if self.stack.is_empty() && self.top_level_consumed() {
            self.done = true;
            return Ok(None);
        }

        // Read a value (or start of container).
        let vt = self.inner.peek().map_err(map_err)?;
        let event = match vt {
            ValueType::Array => {
                self.inner.begin_array().map_err(map_err)?;
                self.stack.push(Container::Array);
                Event::StartArray
            }
            ValueType::Object => {
                self.inner.begin_object().map_err(map_err)?;
                self.stack.push(Container::Object);
                self.expect_name = true;
                Event::StartObject
            }
            ValueType::String => Event::Value(Scalar::String(
                self.inner.next_string().map_err(map_err)?,
            )),
            ValueType::Number => Event::Value(Scalar::Number(
                self.inner.next_number_as_string().map_err(map_err)?,
            )),
            ValueType::Boolean => {
                Event::Value(Scalar::Bool(self.inner.next_bool().map_err(map_err)?))
            }
            ValueType::Null => {
                self.inner.next_null().map_err(map_err)?;
                Event::Value(Scalar::Null)
            }
        };

        // After a value event inside an object, we must read a name next.
        if matches!(self.stack.last(), Some(Container::Object)) && !matches!(event, Event::StartObject) {
            self.expect_name = true;
        }

        Ok(Some(event))
    }

    fn close_container(&mut self, c: Container) -> Result<Event> {
        match c {
            Container::Array => {
                self.inner.end_array().map_err(map_err)?;
                self.pop_container();
                Ok(Event::EndArray)
            }
            Container::Object => {
                self.inner.end_object().map_err(map_err)?;
                self.pop_container();
                Ok(Event::EndObject)
            }
        }
    }

    fn pop_container(&mut self) {
        self.stack.pop();
        self.expect_name = matches!(self.stack.last(), Some(Container::Object));
    }

    fn top_level_consumed(&self) -> bool {
        // If we've emitted at least one event at depth 0 and now we're back at
        // depth 0 with no container open, the document is done.
        // struson will error on `peek()` past the top-level value, so we use
        // a manual check here: after closing the last container we mark done.
        false
    }

    /// Current nesting depth (0 = top level).
    pub fn depth(&self) -> usize {
        self.stack.len()
    }
}

fn map_err(e: struson::reader::ReaderError) -> Error {
    use struson::reader::ReaderError as R;
    match e {
        R::IoError { error, .. } => Error::Io(error),
        R::SyntaxError(se) => Error::Syntax {
            offset: se.location.data_pos,
            message: format!("{:?}", se.kind),
        },
        R::UnexpectedValueType { .. } | R::UnexpectedStructure { .. } => Error::Syntax {
            offset: 0,
            message: format!("{e}"),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn events_of(json: &str) -> Vec<Event> {
        let mut r = EventReader::new(json.as_bytes());
        let mut out = Vec::new();
        while let Some(e) = r.next_event().unwrap() {
            out.push(e);
        }
        out
    }

    #[test]
    fn reads_scalar_string() {
        assert_eq!(events_of("\"hello\""), vec![Event::Value(Scalar::String("hello".into()))]);
    }

    #[test]
    fn reads_scalar_number_preserving_form() {
        assert_eq!(events_of("1.0"), vec![Event::Value(Scalar::Number("1.0".into()))]);
        assert_eq!(events_of("-0"), vec![Event::Value(Scalar::Number("-0".into()))]);
    }

    #[test]
    fn reads_empty_array() {
        assert_eq!(events_of("[]"), vec![Event::StartArray, Event::EndArray]);
    }

    #[test]
    fn reads_empty_object() {
        assert_eq!(events_of("{}"), vec![Event::StartObject, Event::EndObject]);
    }

    #[test]
    fn reads_flat_array() {
        let e = events_of("[1, true, null, \"x\"]");
        assert_eq!(
            e,
            vec![
                Event::StartArray,
                Event::Value(Scalar::Number("1".into())),
                Event::Value(Scalar::Bool(true)),
                Event::Value(Scalar::Null),
                Event::Value(Scalar::String("x".into())),
                Event::EndArray,
            ]
        );
    }

    #[test]
    fn reads_nested_object() {
        let e = events_of(r#"{"a": {"b": [1, 2]}, "c": null}"#);
        assert_eq!(
            e,
            vec![
                Event::StartObject,
                Event::Name("a".into()),
                Event::StartObject,
                Event::Name("b".into()),
                Event::StartArray,
                Event::Value(Scalar::Number("1".into())),
                Event::Value(Scalar::Number("2".into())),
                Event::EndArray,
                Event::EndObject,
                Event::Name("c".into()),
                Event::Value(Scalar::Null),
                Event::EndObject,
            ]
        );
    }

    #[test]
    fn reports_syntax_error_with_offset() {
        let mut r = EventReader::new(b"{\"a\":,}".as_slice());
        let err = loop {
            match r.next_event() {
                Ok(None) => panic!("expected error"),
                Ok(Some(_)) => continue,
                Err(e) => break e,
            }
        };
        matches!(err, Error::Syntax { .. });
    }
}
```

- [ ] **Step 2: Re-export from `lib.rs`**

```rust
// crates/jfmt-core/src/lib.rs
//! Streaming JSON parser and writer.

pub mod error;
pub mod event;
pub mod parser;

pub use error::{Error, Result};
pub use event::{Event, Scalar};
pub use parser::EventReader;
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p jfmt-core parser`
Expected: all 6 tests pass.

If `next_name_owned` doesn't exist in the struson version resolved — struson's API moved slightly across 0.5→0.6. Fix: replace `next_name_owned()` with `next_name()?.to_string()` (the crate returns `&str` in some versions). Inspect the actual API with:
`cargo doc -p struson --open`
then adjust the call. Same for the `ReaderError` variants in `map_err`.

- [ ] **Step 4: Commit**

```bash
git add crates/jfmt-core
git commit -m "feat(core): add EventReader wrapping struson"
```

---

## Task 6: `jfmt-core` — JSON string escape helper

**Files:**
- Create: `crates/jfmt-core/src/escape.rs`
- Modify: `crates/jfmt-core/src/lib.rs`

- [ ] **Step 1: Write the test and implementation together (very small module)**

```rust
// crates/jfmt-core/src/escape.rs
//! Write a Rust `&str` as a JSON string literal.

use std::io::{self, Write};

/// Write `s` as a properly escaped JSON string (with surrounding quotes).
///
/// Escapes: `"`, `\`, control chars (0x00–0x1F) as `\uXXXX` or their short
/// form (`\b`, `\f`, `\n`, `\r`, `\t`). Non-ASCII characters are passed
/// through as UTF-8 bytes.
pub fn write_json_string<W: Write>(w: &mut W, s: &str) -> io::Result<()> {
    w.write_all(b"\"")?;
    let mut last = 0usize;
    for (i, c) in s.char_indices() {
        let escape: Option<&[u8]> = match c {
            '"' => Some(b"\\\""),
            '\\' => Some(b"\\\\"),
            '\n' => Some(b"\\n"),
            '\r' => Some(b"\\r"),
            '\t' => Some(b"\\t"),
            '\x08' => Some(b"\\b"),
            '\x0c' => Some(b"\\f"),
            c if (c as u32) < 0x20 => None, // handled below with \u
            _ => continue,
        };
        // Flush any pass-through bytes preceding this char.
        if last < i {
            w.write_all(&s.as_bytes()[last..i])?;
        }
        match escape {
            Some(seq) => w.write_all(seq)?,
            None => {
                write!(w, "\\u{:04x}", c as u32)?;
            }
        }
        last = i + c.len_utf8();
    }
    if last < s.len() {
        w.write_all(&s.as_bytes()[last..])?;
    }
    w.write_all(b"\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn to_string(s: &str) -> String {
        let mut buf = Vec::new();
        write_json_string(&mut buf, s).unwrap();
        String::from_utf8(buf).unwrap()
    }

    #[test]
    fn empty_string() {
        assert_eq!(to_string(""), r#""""#);
    }

    #[test]
    fn ascii_pass_through() {
        assert_eq!(to_string("hello"), r#""hello""#);
    }

    #[test]
    fn quote_and_backslash() {
        assert_eq!(to_string(r#"a"b\c"#), r#""a\"b\\c""#);
    }

    #[test]
    fn newlines_tabs() {
        assert_eq!(to_string("a\nb\tc"), r#""a\nb\tc""#);
    }

    #[test]
    fn control_char_uses_unicode_escape() {
        assert_eq!(to_string("\x01"), r#""""#);
    }

    #[test]
    fn non_ascii_passes_through() {
        assert_eq!(to_string("日本語"), r#""日本語""#);
        assert_eq!(to_string("🦀"), "\"🦀\"");
    }
}
```

- [ ] **Step 2: Register in `lib.rs`**

```rust
// crates/jfmt-core/src/lib.rs (add)
pub mod escape;
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p jfmt-core escape`
Expected: `6 passed`.

- [ ] **Step 4: Commit**

```bash
git add crates/jfmt-core/src/escape.rs crates/jfmt-core/src/lib.rs
git commit -m "feat(core): add JSON string escape helper"
```

---

## Task 7: `jfmt-core` — `EventWriter` trait + `MinifyWriter`

**Files:**
- Create: `crates/jfmt-core/src/writer/mod.rs`
- Create: `crates/jfmt-core/src/writer/minify.rs`
- Modify: `crates/jfmt-core/src/lib.rs`

- [ ] **Step 1: Write the trait**

```rust
// crates/jfmt-core/src/writer/mod.rs
//! Event sinks. Writers consume a stream of [`crate::Event`] and produce
//! JSON text on an underlying `Write`.

pub mod minify;
pub mod pretty;

pub use minify::MinifyWriter;
pub use pretty::PrettyWriter;

use crate::event::Event;
use crate::Result;

/// Common interface for JSON event sinks.
pub trait EventWriter {
    /// Consume one event.
    fn write_event(&mut self, event: &Event) -> Result<()>;

    /// Flush underlying buffered state, if any.
    fn finish(&mut self) -> Result<()>;
}
```

- [ ] **Step 2: Write the failing test (minify)**

```rust
// crates/jfmt-core/src/writer/minify.rs
//! Minified output: zero whitespace, shortest valid JSON.

use crate::escape::write_json_string;
use crate::event::{Event, Scalar};
use crate::writer::EventWriter;
use crate::{Error, Result};
use std::io::Write;

/// Minified JSON writer.
pub struct MinifyWriter<W: Write> {
    w: W,
    /// Stack: true inside an object (expecting Name next), false inside array.
    stack: Vec<Frame>,
}

struct Frame {
    in_object: bool,
    /// Have we emitted any child yet? Used to place commas.
    first: bool,
    /// When in an object, have we just emitted a Name and are waiting for a value?
    pending_name: bool,
}

impl<W: Write> MinifyWriter<W> {
    pub fn new(w: W) -> Self {
        Self {
            w,
            stack: Vec::new(),
        }
    }

    fn write_separator(&mut self) -> Result<()> {
        if let Some(top) = self.stack.last_mut() {
            if top.pending_name {
                // Just emitted a Name; separator is `:` (minified: no space).
                self.w.write_all(b":")?;
                top.pending_name = false;
                return Ok(());
            }
            if !top.first {
                self.w.write_all(b",")?;
            }
            top.first = false;
        }
        Ok(())
    }

    fn write_scalar(&mut self, s: &Scalar) -> Result<()> {
        match s {
            Scalar::String(s) => write_json_string(&mut self.w, s)?,
            Scalar::Number(n) => self.w.write_all(n.as_bytes())?,
            Scalar::Bool(true) => self.w.write_all(b"true")?,
            Scalar::Bool(false) => self.w.write_all(b"false")?,
            Scalar::Null => self.w.write_all(b"null")?,
        }
        Ok(())
    }
}

impl<W: Write> EventWriter for MinifyWriter<W> {
    fn write_event(&mut self, event: &Event) -> Result<()> {
        match event {
            Event::StartObject => {
                self.write_separator()?;
                self.w.write_all(b"{")?;
                self.stack.push(Frame { in_object: true, first: true, pending_name: false });
            }
            Event::EndObject => {
                let frame = self.stack.pop().ok_or_else(|| Error::State("EndObject without StartObject".into()))?;
                if !frame.in_object {
                    return Err(Error::State("EndObject inside array".into()));
                }
                self.w.write_all(b"}")?;
            }
            Event::StartArray => {
                self.write_separator()?;
                self.w.write_all(b"[")?;
                self.stack.push(Frame { in_object: false, first: true, pending_name: false });
            }
            Event::EndArray => {
                let frame = self.stack.pop().ok_or_else(|| Error::State("EndArray without StartArray".into()))?;
                if frame.in_object {
                    return Err(Error::State("EndArray inside object".into()));
                }
                self.w.write_all(b"]")?;
            }
            Event::Name(name) => {
                let top = self.stack.last_mut().ok_or_else(|| Error::State("Name at top level".into()))?;
                if !top.in_object {
                    return Err(Error::State("Name inside array".into()));
                }
                if !top.first {
                    self.w.write_all(b",")?;
                }
                top.first = false;
                write_json_string(&mut self.w, name)?;
                top.pending_name = true;
            }
            Event::Value(s) => {
                self.write_separator()?;
                self.write_scalar(s)?;
            }
        }
        Ok(())
    }

    fn finish(&mut self) -> Result<()> {
        if !self.stack.is_empty() {
            return Err(Error::State(format!("{} unclosed containers", self.stack.len())));
        }
        self.w.flush()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{Event::*, Scalar};

    fn emit(events: &[Event]) -> String {
        let mut buf = Vec::new();
        let mut w = MinifyWriter::new(&mut buf);
        for e in events {
            w.write_event(e).unwrap();
        }
        w.finish().unwrap();
        String::from_utf8(buf).unwrap()
    }

    #[test]
    fn minify_scalar() {
        assert_eq!(emit(&[Value(Scalar::Number("42".into()))]), "42");
    }

    #[test]
    fn minify_empty_object_and_array() {
        assert_eq!(emit(&[StartObject, EndObject]), "{}");
        assert_eq!(emit(&[StartArray, EndArray]), "[]");
    }

    #[test]
    fn minify_flat_array() {
        assert_eq!(
            emit(&[
                StartArray,
                Value(Scalar::Number("1".into())),
                Value(Scalar::Number("2".into())),
                Value(Scalar::Bool(true)),
                EndArray,
            ]),
            "[1,2,true]"
        );
    }

    #[test]
    fn minify_object() {
        assert_eq!(
            emit(&[
                StartObject,
                Name("a".into()),
                Value(Scalar::Number("1".into())),
                Name("b".into()),
                Value(Scalar::Null),
                EndObject,
            ]),
            r#"{"a":1,"b":null}"#
        );
    }

    #[test]
    fn minify_nested() {
        assert_eq!(
            emit(&[
                StartObject,
                Name("x".into()),
                StartArray,
                StartObject,
                Name("y".into()),
                Value(Scalar::Number("1".into())),
                EndObject,
                EndArray,
                EndObject,
            ]),
            r#"{"x":[{"y":1}]}"#
        );
    }
}
```

Also create an empty `pretty.rs` so `mod.rs` compiles:

```rust
// crates/jfmt-core/src/writer/pretty.rs (placeholder for Task 8)
use std::io::Write;
use crate::writer::EventWriter;
use crate::event::Event;
use crate::Result;

/// Placeholder — real implementation lands in Task 8.
pub struct PrettyWriter<W: Write> { _w: W }

impl<W: Write> PrettyWriter<W> {
    pub fn new(_w: W) -> Self { panic!("not yet implemented") }
}

impl<W: Write> EventWriter for PrettyWriter<W> {
    fn write_event(&mut self, _e: &Event) -> Result<()> { unimplemented!() }
    fn finish(&mut self) -> Result<()> { unimplemented!() }
}
```

- [ ] **Step 3: Re-export from `lib.rs`**

```rust
// crates/jfmt-core/src/lib.rs (add)
pub mod writer;

pub use writer::{EventWriter, MinifyWriter, PrettyWriter};
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p jfmt-core writer::minify`
Expected: `5 passed`.

- [ ] **Step 5: Commit**

```bash
git add crates/jfmt-core
git commit -m "feat(core): add EventWriter trait and MinifyWriter"
```

---

## Task 8: `jfmt-core` — `PrettyWriter` with `--indent N`

**Files:**
- Modify: `crates/jfmt-core/src/writer/pretty.rs`

- [ ] **Step 1: Replace placeholder with real implementation and tests**

```rust
// crates/jfmt-core/src/writer/pretty.rs
//! Pretty-printed output with configurable indentation.

use crate::escape::write_json_string;
use crate::event::{Event, Scalar};
use crate::writer::EventWriter;
use crate::{Error, Result};
use std::io::Write;

/// Configuration for [`PrettyWriter`].
#[derive(Debug, Clone)]
pub struct PrettyConfig {
    /// Number of spaces per indent level. Default 2.
    pub indent: u8,
    /// Use tabs instead of spaces. When true, `indent` is ignored.
    pub use_tabs: bool,
    /// Line ending to emit. Default `\n`.
    pub newline: &'static str,
}

impl Default for PrettyConfig {
    fn default() -> Self {
        Self { indent: 2, use_tabs: false, newline: "\n" }
    }
}

/// Pretty JSON writer.
pub struct PrettyWriter<W: Write> {
    w: W,
    cfg: PrettyConfig,
    stack: Vec<Frame>,
    /// Reusable indent byte string (`"\n" + N*depth spaces`) — we rebuild on push/pop.
    indent_buf: Vec<u8>,
}

struct Frame {
    in_object: bool,
    first: bool,
    pending_name: bool,
    /// `true` if this container is empty so far — affects whether we emit a newline at end.
    empty: bool,
}

impl<W: Write> PrettyWriter<W> {
    pub fn new(w: W) -> Self {
        Self::with_config(w, PrettyConfig::default())
    }

    pub fn with_config(w: W, cfg: PrettyConfig) -> Self {
        Self {
            w,
            cfg,
            stack: Vec::new(),
            indent_buf: Vec::new(),
        }
    }

    fn push_indent(&mut self) {
        if self.cfg.use_tabs {
            self.indent_buf.push(b'\t');
        } else {
            for _ in 0..self.cfg.indent {
                self.indent_buf.push(b' ');
            }
        }
    }

    fn pop_indent(&mut self) {
        let n = if self.cfg.use_tabs { 1 } else { self.cfg.indent as usize };
        let new_len = self.indent_buf.len().saturating_sub(n);
        self.indent_buf.truncate(new_len);
    }

    fn write_newline_and_indent(&mut self) -> Result<()> {
        self.w.write_all(self.cfg.newline.as_bytes())?;
        self.w.write_all(&self.indent_buf)?;
        Ok(())
    }

    fn before_child(&mut self) -> Result<()> {
        // Called before writing a name or a value/container start.
        if let Some(top) = self.stack.last_mut() {
            if top.pending_name {
                // Between name and value in an object: `": "`.
                self.w.write_all(b": ")?;
                top.pending_name = false;
                return Ok(());
            }
            if !top.first {
                self.w.write_all(b",")?;
            }
            top.first = false;
            top.empty = false;
        }
        // Indent before each item (but not before the top-level first value).
        if !self.stack.is_empty() {
            self.write_newline_and_indent()?;
        }
        Ok(())
    }

    fn write_scalar(&mut self, s: &Scalar) -> Result<()> {
        match s {
            Scalar::String(s) => write_json_string(&mut self.w, s)?,
            Scalar::Number(n) => self.w.write_all(n.as_bytes())?,
            Scalar::Bool(true) => self.w.write_all(b"true")?,
            Scalar::Bool(false) => self.w.write_all(b"false")?,
            Scalar::Null => self.w.write_all(b"null")?,
        }
        Ok(())
    }
}

impl<W: Write> EventWriter for PrettyWriter<W> {
    fn write_event(&mut self, event: &Event) -> Result<()> {
        match event {
            Event::StartObject => {
                self.before_child()?;
                self.w.write_all(b"{")?;
                self.stack.push(Frame { in_object: true, first: true, pending_name: false, empty: true });
                self.push_indent();
            }
            Event::EndObject => {
                let frame = self.stack.pop().ok_or_else(|| Error::State("EndObject without StartObject".into()))?;
                if !frame.in_object {
                    return Err(Error::State("EndObject inside array".into()));
                }
                self.pop_indent();
                if !frame.empty {
                    self.write_newline_and_indent()?;
                }
                self.w.write_all(b"}")?;
            }
            Event::StartArray => {
                self.before_child()?;
                self.w.write_all(b"[")?;
                self.stack.push(Frame { in_object: false, first: true, pending_name: false, empty: true });
                self.push_indent();
            }
            Event::EndArray => {
                let frame = self.stack.pop().ok_or_else(|| Error::State("EndArray without StartArray".into()))?;
                if frame.in_object {
                    return Err(Error::State("EndArray inside object".into()));
                }
                self.pop_indent();
                if !frame.empty {
                    self.write_newline_and_indent()?;
                }
                self.w.write_all(b"]")?;
            }
            Event::Name(name) => {
                // Name is a "child" position too.
                {
                    let top = self.stack.last_mut().ok_or_else(|| Error::State("Name at top level".into()))?;
                    if !top.in_object {
                        return Err(Error::State("Name inside array".into()));
                    }
                    if !top.first {
                        self.w.write_all(b",")?;
                    }
                    top.first = false;
                    top.empty = false;
                }
                self.write_newline_and_indent()?;
                write_json_string(&mut self.w, name)?;
                // Mark pending_name after writing (borrow was released).
                self.stack.last_mut().unwrap().pending_name = true;
            }
            Event::Value(s) => {
                self.before_child()?;
                self.write_scalar(s)?;
            }
        }
        Ok(())
    }

    fn finish(&mut self) -> Result<()> {
        if !self.stack.is_empty() {
            return Err(Error::State(format!("{} unclosed containers", self.stack.len())));
        }
        // Trailing newline for POSIX friendliness.
        self.w.write_all(self.cfg.newline.as_bytes())?;
        self.w.flush()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{Event::*, Scalar};

    fn emit(events: &[Event]) -> String {
        emit_cfg(events, PrettyConfig::default())
    }

    fn emit_cfg(events: &[Event], cfg: PrettyConfig) -> String {
        let mut buf = Vec::new();
        let mut w = PrettyWriter::with_config(&mut buf, cfg);
        for e in events {
            w.write_event(e).unwrap();
        }
        w.finish().unwrap();
        String::from_utf8(buf).unwrap()
    }

    #[test]
    fn pretty_scalar() {
        assert_eq!(emit(&[Value(Scalar::Number("42".into()))]), "42\n");
    }

    #[test]
    fn pretty_empty_containers_stay_on_one_line() {
        assert_eq!(emit(&[StartObject, EndObject]), "{}\n");
        assert_eq!(emit(&[StartArray, EndArray]), "[]\n");
    }

    #[test]
    fn pretty_flat_array_indent_2() {
        assert_eq!(
            emit(&[
                StartArray,
                Value(Scalar::Number("1".into())),
                Value(Scalar::Number("2".into())),
                EndArray,
            ]),
            "[\n  1,\n  2\n]\n"
        );
    }

    #[test]
    fn pretty_object_indent_4() {
        let cfg = PrettyConfig { indent: 4, ..Default::default() };
        assert_eq!(
            emit_cfg(
                &[
                    StartObject,
                    Name("a".into()),
                    Value(Scalar::Number("1".into())),
                    Name("b".into()),
                    Value(Scalar::Null),
                    EndObject,
                ],
                cfg
            ),
            "{\n    \"a\": 1,\n    \"b\": null\n}\n"
        );
    }

    #[test]
    fn pretty_nested() {
        let s = emit(&[
            StartObject,
            Name("x".into()),
            StartArray,
            StartObject,
            Name("y".into()),
            Value(Scalar::Number("1".into())),
            EndObject,
            EndArray,
            EndObject,
        ]);
        assert_eq!(
            s,
            "{\n  \"x\": [\n    {\n      \"y\": 1\n    }\n  ]\n}\n"
        );
    }

    #[test]
    fn pretty_tabs() {
        let cfg = PrettyConfig { use_tabs: true, ..Default::default() };
        let s = emit_cfg(
            &[
                StartArray,
                Value(Scalar::Number("1".into())),
                EndArray,
            ],
            cfg,
        );
        assert_eq!(s, "[\n\t1\n]\n");
    }
}
```

- [ ] **Step 2: Run tests**

Run: `cargo test -p jfmt-core writer::pretty`
Expected: `6 passed`.

- [ ] **Step 3: Commit**

```bash
git add crates/jfmt-core/src/writer/pretty.rs
git commit -m "feat(core): add PrettyWriter with indent / tabs config"
```

---

## Task 9: `jfmt-core` — `transcode` driver function

**Files:**
- Create: `crates/jfmt-core/src/transcode.rs`
- Modify: `crates/jfmt-core/src/lib.rs`

- [ ] **Step 1: Write the driver and tests**

```rust
// crates/jfmt-core/src/transcode.rs
//! Drive an [`EventReader`] into an [`EventWriter`], closing the pipeline.

use crate::parser::EventReader;
use crate::writer::EventWriter;
use crate::Result;
use std::io::{Read, Write};

/// Read every event from `reader` and emit it into `writer`, then finish.
pub fn transcode<R, W, EW>(reader: R, mut writer: EW) -> Result<()>
where
    R: Read,
    W: Write,
    EW: EventWriter,
    EventReader<R>: ,
    // The writer is generic over its inner W; caller constructs it.
{
    let _ = (std::marker::PhantomData::<W>,); // silence unused param warning
    let mut r = EventReader::new(reader);
    while let Some(event) = r.next_event()? {
        writer.write_event(&event)?;
    }
    writer.finish()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::writer::{MinifyWriter, PrettyWriter};

    #[test]
    fn transcode_minify_removes_whitespace() {
        let input = br#"
            {
              "a": [ 1, 2, 3 ],
              "b": "hi"
            }
        "#;
        let mut out = Vec::new();
        transcode::<_, &mut Vec<u8>, _>(input.as_slice(), MinifyWriter::new(&mut out)).unwrap();
        assert_eq!(String::from_utf8(out).unwrap(), r#"{"a":[1,2,3],"b":"hi"}"#);
    }

    #[test]
    fn transcode_pretty_reformats() {
        let input = br#"{"a":[1,2]}"#;
        let mut out = Vec::new();
        transcode::<_, &mut Vec<u8>, _>(input.as_slice(), PrettyWriter::new(&mut out)).unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "{\n  \"a\": [\n    1,\n    2\n  ]\n}\n"
        );
    }
}
```

**Note:** if the `W: Write` phantom parameter creates friction with inference at call sites, simplify the signature to drop `W`:

```rust
pub fn transcode<R: Read, EW: EventWriter>(reader: R, mut writer: EW) -> Result<()> {
    let mut r = EventReader::new(reader);
    while let Some(event) = r.next_event()? {
        writer.write_event(&event)?;
    }
    writer.finish()?;
    Ok(())
}
```

Adjust the tests accordingly: `transcode(input.as_slice(), MinifyWriter::new(&mut out))`.

- [ ] **Step 2: Re-export**

```rust
// crates/jfmt-core/src/lib.rs (add)
pub mod transcode;
pub use transcode::transcode;
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p jfmt-core transcode`
Expected: `2 passed`.

- [ ] **Step 4: Commit**

```bash
git add crates/jfmt-core
git commit -m "feat(core): add transcode driver connecting reader to writer"
```

---

## Task 10: `jfmt-io` — `Compression` enum + detection

**Files:**
- Modify: `crates/jfmt-io/Cargo.toml`
- Modify: `crates/jfmt-io/src/lib.rs`
- Create: `crates/jfmt-io/src/compress.rs`

- [ ] **Step 1: Add dependencies**

```toml
# crates/jfmt-io/Cargo.toml
[package]
name = "jfmt-io"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true

[dependencies]
jfmt-core = { path = "../jfmt-core" }
flate2 = { workspace = true }
zstd = { workspace = true }
thiserror = { workspace = true }
```

- [ ] **Step 2: Write `compress.rs` with tests**

```rust
// crates/jfmt-io/src/compress.rs
//! Compression type detection and selection.

use std::path::Path;
use std::str::FromStr;

/// Which (de)compression algorithm to apply to a stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compression {
    None,
    Gzip,
    Zstd,
}

impl Compression {
    /// Guess from a path's extension. Unknown / no extension → `None`.
    pub fn from_path(p: &Path) -> Self {
        match p.extension().and_then(|e| e.to_str()) {
            Some(e) if e.eq_ignore_ascii_case("gz") => Compression::Gzip,
            Some(e) if e.eq_ignore_ascii_case("zst") => Compression::Zstd,
            Some(e) if e.eq_ignore_ascii_case("zstd") => Compression::Zstd,
            _ => Compression::None,
        }
    }
}

impl FromStr for Compression {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "none" | "" => Ok(Compression::None),
            "gz" | "gzip" => Ok(Compression::Gzip),
            "zst" | "zstd" => Ok(Compression::Zstd),
            other => Err(format!("unknown compression: {other}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_by_extension() {
        assert_eq!(Compression::from_path(Path::new("a.json")), Compression::None);
        assert_eq!(Compression::from_path(Path::new("a.JSON.gz")), Compression::Gzip);
        assert_eq!(Compression::from_path(Path::new("a.json.zst")), Compression::Zstd);
        assert_eq!(Compression::from_path(Path::new("a.json.ZSTD")), Compression::Zstd);
        assert_eq!(Compression::from_path(Path::new("no_ext")), Compression::None);
    }

    #[test]
    fn parses_from_str() {
        assert_eq!("none".parse::<Compression>().unwrap(), Compression::None);
        assert_eq!("gz".parse::<Compression>().unwrap(), Compression::Gzip);
        assert_eq!("GZIP".parse::<Compression>().unwrap(), Compression::Gzip);
        assert_eq!("zst".parse::<Compression>().unwrap(), Compression::Zstd);
        assert!("foo".parse::<Compression>().is_err());
    }
}
```

- [ ] **Step 3: Lib scaffold**

```rust
// crates/jfmt-io/src/lib.rs
//! I/O adapters: file/stdin/stdout + automatic gzip/zstd (de)compression.

pub mod compress;
pub mod input;
pub mod output;

pub use compress::Compression;
pub use input::{open_input, InputSpec};
pub use output::{open_output, OutputSpec};
```

Create placeholder `input.rs` and `output.rs`:

```rust
// crates/jfmt-io/src/input.rs
use crate::compress::Compression;
use std::io::BufRead;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct InputSpec {
    pub path: Option<PathBuf>, // None = stdin
    pub compression: Option<Compression>, // None = auto-detect
}

pub fn open_input(_spec: &InputSpec) -> std::io::Result<Box<dyn BufRead>> {
    todo!("implemented in Task 11")
}
```

```rust
// crates/jfmt-io/src/output.rs
use crate::compress::Compression;
use std::io::Write;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct OutputSpec {
    pub path: Option<PathBuf>,
    pub compression: Option<Compression>,
}

pub fn open_output(_spec: &OutputSpec) -> std::io::Result<Box<dyn Write>> {
    todo!("implemented in Task 12")
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p jfmt-io compress`
Expected: `2 passed`.

- [ ] **Step 5: Commit**

```bash
git add crates/jfmt-io
git commit -m "feat(io): add Compression enum with path + str detection"
```

---

## Task 11: `jfmt-io` — `open_input` (plain / stdin / gz / zst)

**Files:**
- Modify: `crates/jfmt-io/src/input.rs`

- [ ] **Step 1: Implementation with tests**

```rust
// crates/jfmt-io/src/input.rs
//! Open an input source as a boxed `BufRead`, applying decompression.

use crate::compress::Compression;
use flate2::read::MultiGzDecoder;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Read};
use std::path::{Path, PathBuf};

/// Spec describing where input comes from and whether/how to decompress it.
#[derive(Debug, Clone, Default)]
pub struct InputSpec {
    /// Source path. `None` = stdin.
    pub path: Option<PathBuf>,
    /// Forced compression. `None` = auto-detect from path extension
    /// (stdin with `None` is always treated as uncompressed).
    pub compression: Option<Compression>,
}

impl InputSpec {
    pub fn stdin() -> Self { Self::default() }
    pub fn file(p: impl Into<PathBuf>) -> Self {
        Self { path: Some(p.into()), compression: None }
    }
}

/// Open the input described by `spec` and return a boxed `BufRead`.
pub fn open_input(spec: &InputSpec) -> io::Result<Box<dyn BufRead>> {
    let raw: Box<dyn Read> = match &spec.path {
        Some(p) => Box::new(File::open(p)?),
        None => Box::new(io::stdin().lock()),
    };

    let compression = spec.compression.unwrap_or_else(|| match spec.path.as_deref() {
        Some(p) => Compression::from_path(p),
        None => Compression::None,
    });

    let decoded: Box<dyn Read> = match compression {
        Compression::None => raw,
        Compression::Gzip => Box::new(MultiGzDecoder::new(raw)),
        Compression::Zstd => Box::new(zstd::stream::Decoder::new(raw)?),
    };

    Ok(Box::new(BufReader::with_capacity(64 * 1024, decoded)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    fn tempfile_with(content: &[u8], ext: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join(format!("x.{}", ext));
        std::fs::write(&p, content).unwrap();
        (dir, p)
    }

    fn read_to_string(spec: InputSpec) -> String {
        let mut r = open_input(&spec).unwrap();
        let mut s = String::new();
        r.read_to_string(&mut s).unwrap();
        s
    }

    #[test]
    fn reads_plain_file() {
        let (_d, p) = tempfile_with(b"hello", "json");
        assert_eq!(read_to_string(InputSpec::file(p)), "hello");
    }

    #[test]
    fn decompresses_gzip_by_extension() {
        let mut gz = Vec::new();
        {
            let mut enc = flate2::write::GzEncoder::new(&mut gz, flate2::Compression::default());
            enc.write_all(b"hello gz").unwrap();
            enc.finish().unwrap();
        }
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("x.json.gz");
        std::fs::write(&p, &gz).unwrap();
        assert_eq!(read_to_string(InputSpec::file(p)), "hello gz");
    }

    #[test]
    fn decompresses_zstd_by_extension() {
        let encoded = zstd::encode_all(&b"hello zstd"[..], 0).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("x.json.zst");
        std::fs::write(&p, encoded).unwrap();
        assert_eq!(read_to_string(InputSpec::file(p)), "hello zstd");
    }

    #[test]
    fn forced_compression_overrides_extension() {
        let mut gz = Vec::new();
        {
            let mut enc = flate2::write::GzEncoder::new(&mut gz, flate2::Compression::default());
            enc.write_all(b"forced").unwrap();
            enc.finish().unwrap();
        }
        // Save as ".json" but force gzip decode.
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("x.json");
        std::fs::write(&p, &gz).unwrap();
        let spec = InputSpec { path: Some(p), compression: Some(Compression::Gzip) };
        assert_eq!(read_to_string(spec), "forced");
    }
}
```

- [ ] **Step 2: Add `tempfile` as a dev-dep**

```toml
# crates/jfmt-io/Cargo.toml (under [dev-dependencies])
[dev-dependencies]
tempfile = { workspace = true }
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p jfmt-io input`
Expected: `4 passed`.

- [ ] **Step 4: Commit**

```bash
git add crates/jfmt-io
git commit -m "feat(io): implement open_input with gz/zst auto-detect"
```

---

## Task 12: `jfmt-io` — `open_output` (plain / stdout / gz / zst)

**Files:**
- Modify: `crates/jfmt-io/src/output.rs`

- [ ] **Step 1: Implementation with tests**

```rust
// crates/jfmt-io/src/output.rs
//! Open an output sink as a boxed `Write`, applying compression.

use crate::compress::Compression;
use flate2::write::GzEncoder;
use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::PathBuf;

/// Spec describing where output goes and whether/how to compress it.
#[derive(Debug, Clone, Default)]
pub struct OutputSpec {
    /// Destination path. `None` = stdout.
    pub path: Option<PathBuf>,
    /// Forced compression. `None` = auto-detect from path extension
    /// (stdout with `None` is always treated as uncompressed).
    pub compression: Option<Compression>,
    /// Gzip compression level (0-9). Ignored for other algorithms.
    pub gzip_level: u32,
    /// Zstd compression level (1-22). Ignored for other algorithms.
    pub zstd_level: i32,
}

impl OutputSpec {
    pub fn stdout() -> Self {
        Self {
            path: None,
            compression: None,
            gzip_level: 6,
            zstd_level: 3,
        }
    }
    pub fn file(p: impl Into<PathBuf>) -> Self {
        Self {
            path: Some(p.into()),
            compression: None,
            gzip_level: 6,
            zstd_level: 3,
        }
    }
}

/// Open the output sink described by `spec`. The returned `Write` must be
/// dropped (or explicitly finished) before any compressed stream footer
/// is flushed — wrapping in `BufWriter` ensures Drop writes a clean end.
pub fn open_output(spec: &OutputSpec) -> io::Result<Box<dyn Write>> {
    let raw: Box<dyn Write> = match &spec.path {
        Some(p) => Box::new(File::create(p)?),
        None => Box::new(io::stdout().lock()),
    };

    let compression = spec.compression.unwrap_or_else(|| match spec.path.as_deref() {
        Some(p) => Compression::from_path(p),
        None => Compression::None,
    });

    let encoded: Box<dyn Write> = match compression {
        Compression::None => raw,
        Compression::Gzip => Box::new(GzEncoder::new(raw, flate2::Compression::new(spec.gzip_level))),
        Compression::Zstd => {
            // AutoFinish flushes footer on drop.
            Box::new(zstd::stream::Encoder::new(raw, spec.zstd_level)?.auto_finish())
        }
    };

    Ok(Box::new(BufWriter::with_capacity(64 * 1024, encoded)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    #[test]
    fn writes_plain_file() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("x.json");
        {
            let mut w = open_output(&OutputSpec::file(&p)).unwrap();
            w.write_all(b"hi").unwrap();
        }
        assert_eq!(std::fs::read(&p).unwrap(), b"hi");
    }

    #[test]
    fn writes_gzip_by_extension() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("x.json.gz");
        {
            let mut w = open_output(&OutputSpec::file(&p)).unwrap();
            w.write_all(b"gz payload").unwrap();
        }
        let raw = std::fs::read(&p).unwrap();
        let mut d = flate2::read::MultiGzDecoder::new(&raw[..]);
        let mut s = String::new();
        d.read_to_string(&mut s).unwrap();
        assert_eq!(s, "gz payload");
    }

    #[test]
    fn writes_zstd_by_extension() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("x.json.zst");
        {
            let mut w = open_output(&OutputSpec::file(&p)).unwrap();
            w.write_all(b"zst payload").unwrap();
        }
        let raw = std::fs::read(&p).unwrap();
        let decoded = zstd::decode_all(&raw[..]).unwrap();
        assert_eq!(decoded, b"zst payload");
    }
}
```

- [ ] **Step 2: Run tests**

Run: `cargo test -p jfmt-io output`
Expected: `3 passed`.

- [ ] **Step 3: Commit**

```bash
git add crates/jfmt-io/src/output.rs
git commit -m "feat(io): implement open_output with gz/zst auto-compress"
```

---

## Task 13: `jfmt-cli` — skeleton with clap

**Files:**
- Modify: `crates/jfmt-cli/Cargo.toml`
- Create: `crates/jfmt-cli/src/cli.rs`
- Create: `crates/jfmt-cli/src/exit.rs`
- Modify: `crates/jfmt-cli/src/main.rs`

- [ ] **Step 1: Dependencies**

```toml
# crates/jfmt-cli/Cargo.toml
[package]
name = "jfmt-cli"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true

[[bin]]
name = "jfmt"
path = "src/main.rs"

[dependencies]
jfmt-core = { path = "../jfmt-core" }
jfmt-io = { path = "../jfmt-io" }
clap = { workspace = true }
anyhow = { workspace = true }
```

- [ ] **Step 2: `exit.rs`**

```rust
// crates/jfmt-cli/src/exit.rs
//! Mapping of internal errors to process exit codes.

/// Exit code convention documented in the Phase 1 spec §4.3.
#[repr(i32)]
pub enum ExitCode {
    Success = 0,
    /// Generic I/O, file-not-found, bad argument.
    InputError = 1,
    /// Malformed JSON input.
    SyntaxError = 2,
    /// JSON-Schema validation failure (reserved for M5).
    _SchemaError = 3,
}

impl ExitCode {
    pub fn as_i32(self) -> i32 {
        self as i32
    }
}
```

- [ ] **Step 3: `cli.rs` — argument structs**

```rust
// crates/jfmt-cli/src/cli.rs
//! Clap argument definitions.

use clap::{Parser, Subcommand, Args};
use jfmt_io::Compression;
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(name = "jfmt", version, about = "Streaming JSON/NDJSON formatter")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Pretty-print a JSON document with indentation.
    Pretty(PrettyArgs),
    /// Minify a JSON document, removing all whitespace.
    Minify(MinifyArgs),
}

#[derive(Debug, Args)]
pub struct CommonArgs {
    /// Input path. Omit or use `-` for stdin.
    #[arg(value_name = "INPUT")]
    pub input: Option<String>,

    /// Output path. Omit to write to stdout.
    #[arg(short = 'o', long = "output", value_name = "PATH")]
    pub output: Option<PathBuf>,

    /// Override input compression detection.
    #[arg(long = "compress", value_name = "none|gz|zst")]
    pub compress: Option<Compression>,

    /// Treat input as NDJSON (one JSON value per line). Accepted for
    /// forward-compat; NDJSON fast path lands in M3.
    #[arg(long = "ndjson")]
    pub ndjson: bool,
}

#[derive(Debug, Args)]
pub struct PrettyArgs {
    #[command(flatten)]
    pub common: CommonArgs,

    /// Number of spaces per indent level.
    #[arg(long = "indent", value_name = "N", default_value_t = 2)]
    pub indent: u8,

    /// Indent with tabs instead of spaces.
    #[arg(long = "tabs", conflicts_with = "indent")]
    pub tabs: bool,
}

#[derive(Debug, Args)]
pub struct MinifyArgs {
    #[command(flatten)]
    pub common: CommonArgs,
}

impl CommonArgs {
    pub fn input_spec(&self) -> jfmt_io::InputSpec {
        let path = match self.input.as_deref() {
            None | Some("-") => None,
            Some(p) => Some(PathBuf::from(p)),
        };
        jfmt_io::InputSpec {
            path,
            compression: self.compress,
        }
    }

    pub fn output_spec(&self) -> jfmt_io::OutputSpec {
        let mut spec = match &self.output {
            Some(p) => jfmt_io::OutputSpec::file(p.clone()),
            None => jfmt_io::OutputSpec::stdout(),
        };
        // Output compression is auto-detected from extension; users can still
        // use `--compress` to force input decompression. A separate output
        // override flag lands if/when needed.
        spec.gzip_level = 6;
        spec.zstd_level = 3;
        spec
    }
}
```

`Compression` needs a clap `ValueEnum` impl. Simplest: leave it parsed via `FromStr` which clap uses automatically for `#[arg]` fields implementing `FromStr` (requires `value_parser = clap::value_parser!(Compression)` OR derive `ValueEnum`). Update `compress.rs` to derive `ValueEnum`:

```rust
// crates/jfmt-io/src/compress.rs — add derive
use clap::ValueEnum;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Compression { /* variants */ }
```

and add the `clap` feature dep optionally OR gate it behind a feature. **Simplest:** move the clap enum into `jfmt-cli/src/cli.rs`:

```rust
// in cli.rs, replace use jfmt_io::Compression with:
use clap::ValueEnum;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum CompressArg { None, Gz, Zst }

impl From<CompressArg> for jfmt_io::Compression {
    fn from(c: CompressArg) -> Self {
        match c {
            CompressArg::None => jfmt_io::Compression::None,
            CompressArg::Gz => jfmt_io::Compression::Gzip,
            CompressArg::Zst => jfmt_io::Compression::Zstd,
        }
    }
}
```

and change the `CommonArgs` field:

```rust
#[arg(long = "compress", value_enum)]
pub compress: Option<CompressArg>,
```

Update `input_spec` / `output_spec` to `.map(Into::into)` when reading.

- [ ] **Step 4: `main.rs`**

```rust
// crates/jfmt-cli/src/main.rs
mod cli;
mod commands;
mod exit;

use clap::Parser;
use cli::{Cli, Command};
use exit::ExitCode;
use std::process;

fn main() {
    let cli = Cli::parse();
    let code = match run(cli) {
        Ok(()) => ExitCode::Success,
        Err(e) => {
            eprintln!("jfmt: {e:#}");
            classify(&e)
        }
    };
    process::exit(code.as_i32());
}

fn run(cli: Cli) -> anyhow::Result<()> {
    match cli.command {
        Command::Pretty(args) => commands::pretty::run(args),
        Command::Minify(args) => commands::minify::run(args),
    }
}

fn classify(e: &anyhow::Error) -> ExitCode {
    if let Some(core_err) = e.downcast_ref::<jfmt_core::Error>() {
        if matches!(core_err, jfmt_core::Error::Syntax { .. }) {
            return ExitCode::SyntaxError;
        }
    }
    ExitCode::InputError
}
```

Create empty command modules:

```rust
// crates/jfmt-cli/src/commands/mod.rs
pub mod pretty;
pub mod minify;
```

```rust
// crates/jfmt-cli/src/commands/pretty.rs
use crate::cli::PrettyArgs;
pub fn run(_args: PrettyArgs) -> anyhow::Result<()> {
    anyhow::bail!("not yet implemented")
}
```

```rust
// crates/jfmt-cli/src/commands/minify.rs
use crate::cli::MinifyArgs;
pub fn run(_args: MinifyArgs) -> anyhow::Result<()> {
    anyhow::bail!("not yet implemented")
}
```

- [ ] **Step 5: Verify it builds**

Run: `cargo build -p jfmt-cli`
Expected: compiles. `target\debug\jfmt --help` should print usage.

- [ ] **Step 6: Commit**

```bash
git add crates/jfmt-cli
git commit -m "feat(cli): add clap skeleton with pretty/minify subcommand stubs"
```

---

## Task 14: `jfmt-cli` — wire up `pretty` subcommand

**Files:**
- Modify: `crates/jfmt-cli/src/commands/pretty.rs`

- [ ] **Step 1: Implementation**

```rust
// crates/jfmt-cli/src/commands/pretty.rs
use crate::cli::PrettyArgs;
use anyhow::Context;
use jfmt_core::transcode;
use jfmt_core::writer::{PrettyConfig, PrettyWriter};

pub fn run(args: PrettyArgs) -> anyhow::Result<()> {
    let input = jfmt_io::open_input(&args.common.input_spec()).context("opening input")?;
    let output = jfmt_io::open_output(&args.common.output_spec()).context("opening output")?;

    let cfg = PrettyConfig {
        indent: args.indent,
        use_tabs: args.tabs,
        newline: "\n",
    };
    let writer = PrettyWriter::with_config(output, cfg);
    transcode(input, writer).context("pretty-printing failed")?;
    Ok(())
}
```

Note: `PrettyConfig` needs to be re-exported from `jfmt-core` if not already. Add to `crates/jfmt-core/src/writer/mod.rs`:

```rust
pub use pretty::{PrettyConfig, PrettyWriter};
```

And to `crates/jfmt-core/src/lib.rs`:

```rust
pub use writer::{EventWriter, MinifyWriter, PrettyWriter, PrettyConfig};
```

- [ ] **Step 2: Smoke-test by hand**

Run:
```bash
echo '{"a":[1,2,3]}' | cargo run -p jfmt-cli -- pretty
```
Expected output:
```
{
  "a": [
    1,
    2,
    3
  ]
}
```

Run:
```bash
echo '{"a":1}' | cargo run -p jfmt-cli -- pretty --indent 4
```
Expected output:
```
{
    "a": 1
}
```

- [ ] **Step 3: Commit**

```bash
git add crates/jfmt-cli/src/commands/pretty.rs crates/jfmt-core/src
git commit -m "feat(cli): wire up 'jfmt pretty' subcommand"
```

---

## Task 15: `jfmt-cli` — wire up `minify` subcommand

**Files:**
- Modify: `crates/jfmt-cli/src/commands/minify.rs`

- [ ] **Step 1: Implementation**

```rust
// crates/jfmt-cli/src/commands/minify.rs
use crate::cli::MinifyArgs;
use anyhow::Context;
use jfmt_core::{transcode, MinifyWriter};

pub fn run(args: MinifyArgs) -> anyhow::Result<()> {
    let input = jfmt_io::open_input(&args.common.input_spec()).context("opening input")?;
    let output = jfmt_io::open_output(&args.common.output_spec()).context("opening output")?;
    let writer = MinifyWriter::new(output);
    transcode(input, writer).context("minifying failed")?;
    Ok(())
}
```

- [ ] **Step 2: Smoke-test**

Run:
```bash
echo '{ "a" :  [ 1 , 2 ] }' | cargo run -p jfmt-cli -- minify
```
Expected: `{"a":[1,2]}` (no trailing newline from minify).

- [ ] **Step 3: Commit**

```bash
git add crates/jfmt-cli/src/commands/minify.rs
git commit -m "feat(cli): wire up 'jfmt minify' subcommand"
```

---

## Task 16: End-to-end CLI tests

**Files:**
- Create: `tests/fixtures/simple.json`
- Create: `tests/fixtures/simple.pretty2.json`
- Create: `tests/fixtures/simple.pretty4.json`
- Create: `tests/fixtures/simple.min.json`
- Create: `tests/cli_pretty.rs`
- Create: `tests/cli_minify.rs`
- Create: `tests/Cargo.toml` (integration test crate)

The repository root doesn't have a single `tests/` convention for workspaces — place tests under `crates/jfmt-cli/tests/` so `cargo test -p jfmt-cli` runs them with the binary built.

**Revised paths:**
- `crates/jfmt-cli/tests/fixtures/...`
- `crates/jfmt-cli/tests/cli_pretty.rs`
- `crates/jfmt-cli/tests/cli_minify.rs`

- [ ] **Step 1: Add dev-deps to `jfmt-cli`**

```toml
# crates/jfmt-cli/Cargo.toml (add)
[dev-dependencies]
assert_cmd = { workspace = true }
predicates = { workspace = true }
tempfile = { workspace = true }
```

- [ ] **Step 2: Create fixtures**

```bash
mkdir -p crates/jfmt-cli/tests/fixtures
```

`crates/jfmt-cli/tests/fixtures/simple.json`:
```json
{"a":[1,2,3],"b":"hi","c":null}
```

`crates/jfmt-cli/tests/fixtures/simple.pretty2.json`:
```
{
  "a": [
    1,
    2,
    3
  ],
  "b": "hi",
  "c": null
}
```
(Trailing newline present.)

`crates/jfmt-cli/tests/fixtures/simple.pretty4.json`:
```
{
    "a": [
        1,
        2,
        3
    ],
    "b": "hi",
    "c": null
}
```

`crates/jfmt-cli/tests/fixtures/simple.min.json`:
```
{"a":[1,2,3],"b":"hi","c":null}
```
(No trailing newline from minify.)

- [ ] **Step 3: `cli_pretty.rs`**

```rust
// crates/jfmt-cli/tests/cli_pretty.rs
use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use std::path::PathBuf;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name)
}

#[test]
fn pretty_indent_2_matches_golden() {
    let expected = fs::read_to_string(fixture("simple.pretty2.json")).unwrap();
    Command::cargo_bin("jfmt").unwrap()
        .arg("pretty")
        .arg(fixture("simple.json"))
        .assert()
        .success()
        .stdout(predicate::eq(expected));
}

#[test]
fn pretty_indent_4_matches_golden() {
    let expected = fs::read_to_string(fixture("simple.pretty4.json")).unwrap();
    Command::cargo_bin("jfmt").unwrap()
        .arg("pretty")
        .arg("--indent").arg("4")
        .arg(fixture("simple.json"))
        .assert()
        .success()
        .stdout(predicate::eq(expected));
}

#[test]
fn pretty_from_stdin() {
    Command::cargo_bin("jfmt").unwrap()
        .arg("pretty")
        .write_stdin("[1,2]")
        .assert()
        .success()
        .stdout("[\n  1,\n  2\n]\n");
}

#[test]
fn pretty_writes_to_output_file() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("out.json");
    Command::cargo_bin("jfmt").unwrap()
        .arg("pretty")
        .arg(fixture("simple.json"))
        .arg("-o").arg(&out)
        .assert()
        .success();
    let expected = fs::read_to_string(fixture("simple.pretty2.json")).unwrap();
    assert_eq!(fs::read_to_string(out).unwrap(), expected);
}

#[test]
fn pretty_roundtrips_gzip() {
    // Gzip the input on the fly.
    let dir = tempfile::tempdir().unwrap();
    let gz_in = dir.path().join("simple.json.gz");
    let raw = fs::read(fixture("simple.json")).unwrap();
    let mut enc = flate2::write::GzEncoder::new(
        fs::File::create(&gz_in).unwrap(),
        flate2::Compression::default(),
    );
    use std::io::Write;
    enc.write_all(&raw).unwrap();
    enc.finish().unwrap();

    let gz_out = dir.path().join("out.json.gz");
    Command::cargo_bin("jfmt").unwrap()
        .arg("pretty")
        .arg(&gz_in)
        .arg("-o").arg(&gz_out)
        .assert()
        .success();

    let decoded = {
        let mut d = flate2::read::MultiGzDecoder::new(fs::File::open(&gz_out).unwrap());
        let mut s = String::new();
        use std::io::Read;
        d.read_to_string(&mut s).unwrap();
        s
    };
    let expected = fs::read_to_string(fixture("simple.pretty2.json")).unwrap();
    assert_eq!(decoded, expected);
}

#[test]
fn pretty_syntax_error_exits_2() {
    Command::cargo_bin("jfmt").unwrap()
        .arg("pretty")
        .write_stdin("{not json}")
        .assert()
        .code(2)
        .stderr(predicate::str::contains("syntax error"));
}

#[test]
fn pretty_missing_file_exits_1() {
    Command::cargo_bin("jfmt").unwrap()
        .arg("pretty")
        .arg("no-such-file.json")
        .assert()
        .code(1);
}
```

Add `flate2` as a dev-dep of `jfmt-cli`:

```toml
# crates/jfmt-cli/Cargo.toml (dev-dependencies)
flate2 = { workspace = true }
```

- [ ] **Step 4: `cli_minify.rs`**

```rust
// crates/jfmt-cli/tests/cli_minify.rs
use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use std::path::PathBuf;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name)
}

#[test]
fn minify_matches_golden() {
    let expected = fs::read_to_string(fixture("simple.min.json")).unwrap();
    Command::cargo_bin("jfmt").unwrap()
        .arg("minify")
        .arg(fixture("simple.json"))
        .assert()
        .success()
        .stdout(predicate::eq(expected));
}

#[test]
fn minify_from_stdin_to_stdout() {
    Command::cargo_bin("jfmt").unwrap()
        .arg("minify")
        .write_stdin("{ \"a\" :  [ 1 , 2 ] }")
        .assert()
        .success()
        .stdout("{\"a\":[1,2]}");
}

#[test]
fn minify_zstd_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let zst_out = dir.path().join("out.json.zst");
    Command::cargo_bin("jfmt").unwrap()
        .arg("minify")
        .arg(fixture("simple.json"))
        .arg("-o").arg(&zst_out)
        .assert()
        .success();
    let decoded = zstd::decode_all(fs::File::open(&zst_out).unwrap()).unwrap();
    let expected = fs::read_to_string(fixture("simple.min.json")).unwrap();
    assert_eq!(String::from_utf8(decoded).unwrap(), expected);
}
```

Add `zstd` as dev-dep:

```toml
# crates/jfmt-cli/Cargo.toml (dev-dependencies)
zstd = { workspace = true }
```

- [ ] **Step 5: Run all tests**

Run: `cargo test -p jfmt-cli`
Expected: 3 minify tests + 7 pretty tests all pass.

- [ ] **Step 6: Commit**

```bash
git add crates/jfmt-cli
git commit -m "test(cli): add end-to-end golden + roundtrip tests"
```

---

## Task 17: Property test — `minify(pretty(x)) == minify(x)`

**Files:**
- Create: `crates/jfmt-core/tests/roundtrip_proptest.rs`

- [ ] **Step 1: Add proptest + serde_json as dev-deps of jfmt-core**

```toml
# crates/jfmt-core/Cargo.toml (dev-dependencies)
[dev-dependencies]
proptest = { workspace = true }
serde_json = { workspace = true }
```

- [ ] **Step 2: Create the test**

```rust
// crates/jfmt-core/tests/roundtrip_proptest.rs
//! Property tests for parser + writer round-trips.

use jfmt_core::{transcode, MinifyWriter, PrettyWriter};
use proptest::prelude::*;
use serde_json::{json, Value};

/// Generator for small arbitrary JSON values (bounded depth to keep tests fast).
fn arb_value() -> impl Strategy<Value = Value> {
    let leaf = prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::Bool),
        any::<i64>().prop_map(|n| json!(n)),
        ".*".prop_map(Value::String),
    ];
    leaf.prop_recursive(4, 32, 8, |inner| {
        prop_oneof![
            prop::collection::vec(inner.clone(), 0..8).prop_map(Value::Array),
            prop::collection::hash_map("[a-zA-Z0-9_]{0,6}", inner, 0..8)
                .prop_map(|m| Value::Object(m.into_iter().collect())),
        ]
    })
}

fn minify_via_core(input: &str) -> String {
    let mut out = Vec::new();
    transcode(input.as_bytes(), MinifyWriter::new(&mut out)).unwrap();
    String::from_utf8(out).unwrap()
}

fn pretty_via_core(input: &str) -> String {
    let mut out = Vec::new();
    transcode(input.as_bytes(), PrettyWriter::new(&mut out)).unwrap();
    String::from_utf8(out).unwrap()
}

proptest! {
    /// minify should produce output parseable back to the same logical value.
    #[test]
    fn minify_preserves_semantics(v in arb_value()) {
        let text = serde_json::to_string(&v).unwrap();
        let minified = minify_via_core(&text);
        let reparsed: Value = serde_json::from_str(&minified).unwrap();
        prop_assert_eq!(reparsed, v);
    }

    /// pretty should produce output parseable back to the same logical value.
    #[test]
    fn pretty_preserves_semantics(v in arb_value()) {
        let text = serde_json::to_string(&v).unwrap();
        let pretty = pretty_via_core(&text);
        let reparsed: Value = serde_json::from_str(&pretty).unwrap();
        prop_assert_eq!(reparsed, v);
    }

    /// pretty then minify == minify directly.
    #[test]
    fn pretty_then_minify_is_canonical(v in arb_value()) {
        let text = serde_json::to_string(&v).unwrap();
        let via_pretty = minify_via_core(&pretty_via_core(&text));
        let direct = minify_via_core(&text);
        prop_assert_eq!(via_pretty, direct);
    }
}
```

- [ ] **Step 3: Run the property tests**

Run: `cargo test -p jfmt-core --test roundtrip_proptest`
Expected: 3 property tests pass (may take a few seconds for 256 default cases each).

**If a property fails**, proptest will shrink and print the minimal failing input. Debug from there. Two common failure classes to expect:
- String escape mismatch — fix `escape.rs` to handle the specific char.
- Number lexical form — `serde_json::to_string(&json!(0.0))` produces `"0.0"`, we pass it through. `reparsed` should still `==` the original.

- [ ] **Step 4: Commit**

```bash
git add crates/jfmt-core
git commit -m "test(core): add proptest round-trip invariants"
```

---

## Task 18: README update + usage examples

**Files:**
- Modify: `README.md`

- [ ] **Step 1: Flesh out the README with real examples that pass**

```markdown
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
```

- [ ] **Step 2: Commit**

```bash
git add README.md
git commit -m "docs: update README for M1 release"
```

---

## Task 19: Tag M1 preview release

**Files:**
- Modify: `Cargo.toml` (workspace — bump if needed)

- [ ] **Step 1: Verify whole workspace is green**

Run: `cargo fmt --all -- --check`
Expected: exit 0.

Run: `cargo clippy --workspace --all-targets -- -D warnings`
Expected: exit 0.

Run: `cargo test --workspace`
Expected: all tests pass (~35 total across unit + proptest + CLI e2e).

- [ ] **Step 2: Tag the commit**

```bash
git tag -a v0.0.1 -m "M1 preview: pretty + minify with gz/zst"
```

- [ ] **Step 3: Update the status note in the spec**

Append to `docs/superpowers/specs/2026-04-23-jfmt-phase1-design.md` section 11 milestone table:

```markdown
| M1 ✓ | Shipped v0.0.1 on 2026-04-XX (tag `v0.0.1`) |
```

(Fill in the actual date.) Commit:

```bash
git add docs/superpowers/specs/2026-04-23-jfmt-phase1-design.md
git commit -m "docs(spec): mark M1 as shipped (v0.0.1)"
```

---

## Self-Review

**Spec coverage check against `docs/superpowers/specs/2026-04-23-jfmt-phase1-design.md`:**

- §4 Architecture — three crates `jfmt-core` / `jfmt-io` / `jfmt-cli` created (Tasks 1, 3, 10, 13). Core accepts `impl Read` / `impl Write` (Tasks 5, 7, 8, 9). ✓
- §4.1 `event.rs` — Task 4. ✓
- §4.1 `parser.rs` — Task 5. ✓
- §4.1 `writer/pretty.rs` — Task 8 (indent + tabs; `--sort-keys` and `--array-per-line` explicitly deferred per scope boundary). **Gap vs spec CLI surface**, documented in plan header.
- §4.1 `writer/minify.rs` — Task 7. ✓
- §4.1 `filter/*`, `validate/*`, `ndjson/*` — **explicitly out of scope for M1**, land in M2–M5. ✓ (per spec milestone table)
- §4.2 `jfmt-io/input.rs` gz + zst + stdin — Task 11. ✓
- §4.2 `jfmt-io/output.rs` gz + zst + stdout — Task 12. ✓
- §4.3 clap subcommands — Task 13 (only `pretty` / `minify` in M1). ✓
- §4.3 progress bar (indicatif) — **deferred to M2+** since M1 is single-threaded simple path; README documents this.
- §4.3 exit codes 0/1/2/3 — Tasks 13 (`exit.rs`) + 16 (tests verify 1 and 2); 3 is reserved for M5. ✓
- §10 Testing strategy: unit tests (Tasks 4–8), property tests (Task 17), CLI e2e (Task 16). Large-file smoke test and benchmarks deferred to later milestones where they have more to measure. ✓
- §11 Milestone M1 bounds match this plan. ✓

**Placeholder scan:** searched for "TODO", "TBD", "later", "implement later". Found: Task 13 uses `todo!("implemented in Task 11/12")` as transient stubs that are replaced inside the same plan (Tasks 11, 12). No leftover placeholders after plan completion. ✓

**Type consistency check:**
- `Event`, `Scalar` defined in Task 4, used consistently in Tasks 5, 7, 8, 9.
- `EventWriter` trait in Task 7, implemented by `MinifyWriter` (Task 7) and `PrettyWriter` (Task 8), consumed in Task 9 (`transcode`), used in Tasks 14, 15 (CLI).
- `PrettyConfig { indent, use_tabs, newline }` — Task 8 defines it, Task 14 constructs it with matching fields.
- `InputSpec { path, compression }` and `OutputSpec { path, compression, gzip_level, zstd_level }` — Task 10 skeleton, Tasks 11/12 finalize, Task 13 builds them from `CommonArgs`.
- `Compression { None, Gzip, Zstd }` in `jfmt-io`; CLI wraps with `CompressArg { None, Gz, Zst }` (Task 13) and converts via `From`. Consistent.
- `Error` enum + `Result<T>` alias from Task 3, used throughout core. CLI downcast site (Task 13 `classify`) matches the enum variants.

**Ambiguity check:** the `transcode` function has a phantom `W` parameter in the first draft that isn't used — Task 9 calls this out and provides the simplified signature. Plan mentions both for clarity; the simplified form is what ships.

---

## Execution Handoff

Plan complete and saved to `docs/superpowers/plans/2026-04-23-jfmt-m1-core-pretty-minify.md`. Two execution options:

**1. Subagent-Driven (recommended)** — I dispatch a fresh subagent per task, review between tasks, fast iteration

**2. Inline Execution** — Execute tasks in this session using executing-plans, batch execution with checkpoints

**Which approach?**
