# jfmt M7 — Phase 1b XML Support Design

**Status:** approved 2026-04-26.
**Target version:** v0.2.0 (additive feature, no breaking changes to v0.1.x surface).
**Predecessor:** Phase 1 complete at v0.1.0 / v0.1.1.
**Successor scope deferred:** see §10.

## 1. Goal

Add streaming XML parsing/writing and a `jfmt convert` subcommand that
translates between JSON and XML in either direction, preserving jfmt's
constant-memory promise (O(nesting depth), not O(file size)).

This closes the most-requested Phase 1b item from the parent spec
(`docs/superpowers/specs/2026-04-23-jfmt-phase1-design.md` §1, "XML, YAML,
SQL dump support; format conversion"). YAML and SQL dump are out of scope
for M7 — see §10.

## 2. Non-Goals

- **NDJSON-of-XML** (each line a separate XML document) and
  **XML → record-stream** (slice along a `<record>` boundary, emit one
  JSON per record). These are useful but independent features; defer to
  v0.3.0.
- **DTD / XSD validation.** Outside jfmt's philosophy; specialized tools
  (`xmllint`) do this better.
- **XPath / XSLT.**
- **Alternative mapping conventions** (BadgerFish, JsonML, Parker).
  v0.2.0 ships @attr/#text only. If user demand surfaces, add as a
  `--mapping` flag later.
- **SQL dump support.** Confirmed deferred during brainstorming; will be
  a future `jfmt-sql` crate + `jfmt convert --to ndjson` extension.

## 3. CLI Surface

```
jfmt convert <FILE>                          # input path; format inferred from extension
    [-o, --output <FILE>]                    # output path; default stdout
    [--from xml|json] [--to xml|json]        # explicit format (required for stdin, or to override extension)
    [--array-rule "p1.p2,p3"]                # XML→JSON: comma-separated dotted paths whose elements collapse to scalar/object instead of always-array
    [--root NAME]                            # JSON→XML: wrap output in <NAME>...</NAME>; required when JSON has multiple top-level keys
    [--pretty] [--indent N] [--tabs]         # output formatting; reuses jfmt-core PrettyConfig for JSON, jfmt-xml has equivalent for XML
    [--xml-decl]                             # JSON→XML: emit <?xml version="1.0" encoding="UTF-8"?> prologue
    [--strict]                               # XML→JSON: error (exit 34) when non-contiguous same-name siblings detected; JSON→XML: forbid --root rescue when top-level has multiple keys
```

**Extension inference**: `.xml` → xml; `.json` / `.ndjson` → json; other
extensions error and require `--from`/`--to`. Compression suffixes
(`.gz`, `.zst`) are stripped first by `jfmt-io`, then format inferred from
the next suffix (`out.json.gz` → json + gzip, etc.).

**stdin / stdout**: when `<FILE>` is omitted, input is stdin and `--from`
is required. When `--output` is omitted, output is stdout (no extension to
infer from), so `--to` is required if `--from` doesn't fully determine
direction. The convert command always converts; it never auto-detects
"do nothing" — passing `--from json --to json` is an error.

## 4. JSON ↔ XML Mapping

### 4.1 XML → JSON

| XML construct | JSON output |
|---|---|
| `<a/>` or `<a></a>` | `{"a": [{}]}` (always-array default; see §4.3) |
| `<a x="1" y="2"/>` | `{"a": [{"@x": "1", "@y": "2"}]}` |
| `<a>text</a>` | `{"a": [{"#text": "text"}]}` |
| `<a x="1">text</a>` | `{"a": [{"@x": "1", "#text": "text"}]}` |
| `<a><b/><b/></a>` | `{"a": [{"b": [{}, {}]}]}` |
| `<a>before<b/>after</a>` (mixed content) | `{"a": [{"#text": "beforeafter", "b": [{}]}]}` — all `#text` nodes concatenated; element/text interleaving order is lost |
| `<ns:foo xmlns:ns="..."/>` (namespaces) | `{"ns:foo": [{"@xmlns:ns": "..."}]}` — prefix preserved verbatim |
| `<![CDATA[raw]]>` | Treated identically to `#text` |
| `<!--cmt-->`, `<?xml?>`, processing instructions | **Dropped silently** |
| Document root element | The single top-level key of the output JSON object |

All XML attribute and text values are emitted as JSON strings. The parser
does NOT attempt to coerce numeric or boolean literals (XML has no type
system; preserving everything as a string is the only honest, lossless
choice).

### 4.2 JSON → XML

| JSON construct | XML output |
|---|---|
| `{"a": ...}` (single top-level key) | `<a>...</a>` becomes the document root |
| Top-level object with multiple keys | Error, unless `--root NAME` is given (then wrap in `<NAME>...</NAME>`) |
| Top-level array or scalar | Error, unless `--root NAME` is given |
| `"@x": "1"` field inside object | Element attribute `x="1"` |
| `"#text": "v"` field inside object | Element text content child |
| `"a": [v1, v2]` (array value) | Sibling elements `<a>v1</a><a>v2</a>` |
| `"a": "string"` | `<a>string</a>` |
| `"a": null` | `<a/>` (empty element) |
| `"a": 42` or `"a": true` | `<a>42</a>` / `<a>true</a>` (numeric/boolean literal serialized as text) |
| `"a": {}` (empty object) | `<a/>` |

**Attribute key validation**: keys starting with `@` must contain a valid
XML attribute name after the prefix. Keys starting with `#` other than
exactly `#text` are an error. The `xmlns` and `xmlns:*` attributes pass
through unchanged.

### 4.3 Array Rules

**Default**: every XML element is wrapped in a JSON array. This is the
honest streaming choice — the writer never has to look ahead to decide
whether `<b>` is "single" or "repeated."

**`--array-rule "users.user,items.item"`**: dotted-path syntax. For each
listed path, the element at that path is collapsed: a single occurrence
emits the bare object/scalar; multiple occurrences still emit an array.
The path is resolved against the JSON output structure (i.e. element
names, NOT XML XPath syntax).

Paths are matched relative to the document root. `users.user` matches the
`<user>` children of the root `<users>`. Wildcards are NOT supported in
v0.2.0; users list each path explicitly.

### 4.4 Round-Trip Guarantees

**XML → JSON → XML** is structurally equivalent to the original XML modulo
these documented losses:

1. Comments, processing instructions, and the `<?xml?>` declaration are
   dropped on the JSON side and reconstituted only if `--xml-decl` is
   passed during the reverse trip.
2. In mixed content, the interleaving order of text and child elements
   is lost (text nodes are concatenated). E.g. `<a>x<b/>y</a>` and
   `<a><b/>xy</a>` produce identical JSON.
3. Whitespace inside element content is preserved verbatim. Whitespace
   between elements (indentation in pretty-printed XML) is preserved as
   `#text` and round-trips, though it may look surprising.

**JSON → XML → JSON** is structurally equivalent in default array mode
modulo:

- `"a": null` and `"a": {}` both serialize to `<a/>` and round-trip back
  as `[{}]`. Users who need to distinguish null from empty-object should
  not rely on JSON→XML→JSON for that signal.
- Numeric and boolean scalars round-trip as strings (`42` → `<a>42</a>`
  → `"42"`). XML is typeless; the loss is unavoidable.

Under `--array-rule`, the reverse trip may re-introduce arrays; the user
opts into this asymmetry knowingly.

## 5. Streaming Model

### 5.1 JSON → XML

Trivial. Each JSON event from `jfmt-core::EventReader` maps directly to
zero or more XML events. Memory is O(nesting depth) — same as JSON
pretty/minify in M1.

### 5.2 XML → JSON

Streaming with always-array works **as long as same-name sibling elements
are contiguous** (the overwhelmingly common case for data XML). When
non-contiguous siblings are encountered, e.g. `<root><a/><b/><a/></root>`,
two behaviors:

- **Default (warning)**: emit a warning to stderr (`warning: non-contiguous
  same-name siblings under <root>: 'a' at line N. Output uses
  position-preserving form.`) and switch the parent's children to
  position-preserving array form: `{"root": [{"a": [{}]}, {"b": [{}]},
  {"a": [{}]}]}`. The promise is honored (output is valid JSON; original
  XML can still be reconstructed approximately on round-trip), at the cost
  of an inconsistent shape across documents.
- **`--strict`**: hard error, exit code 34. For users who need a
  predictable shape and would rather rewrite their XML.

This is the ONE place jfmt's "constant memory + everyone-else's standard
shape" claims clash; we honor constant memory and document the shape
divergence rather than buffer the parent.

### 5.3 NDJSON Out of Scope

Single-document XML ↔ single-document JSON only. The convert command
errors when given `.ndjson` input or when JSON input contains multiple
top-level documents. Streaming `<record>`-extraction (XML → NDJSON of
records) is a v0.3.0 candidate.

## 6. Architecture

### 6.1 Crate Layout

```
crates/
  jfmt-core/    unchanged: JSON Event/Reader/Writer
  jfmt-xml/     NEW: XML SAX-style Reader/Writer on top of quick-xml
  jfmt-io/      unchanged: stdin/stdout/gz/zst — XML uses it too
  jfmt-cli/     NEW: src/commands/convert.rs — bridges JSON ⇄ XML event streams
```

`jfmt-xml` is independent of `jfmt-core`; it does NOT extend
`jfmt-core::Event`. The JSON and XML event models are different enough
(attributes, namespaces, mixed content) that forcing a shared event type
either hobbles XML or pollutes JSON. The translation layer
(`commands/convert.rs`) is the only place the two crates meet.

This keeps M1–M6 invariants intact: XML failures cannot reach the JSON
code path, and `jfmt pretty` / `jfmt validate` / `jfmt filter` continue to
have zero XML dependencies.

### 6.2 jfmt-xml Public API (sketch)

```rust
// crates/jfmt-xml/src/event.rs
pub enum XmlEvent<'a> {
    Decl { version: &'a str, encoding: Option<&'a str>, standalone: Option<bool> },
    StartTag { name: Cow<'a, str>, attrs: Vec<(Cow<'a, str>, Cow<'a, str>)> },
    EndTag { name: Cow<'a, str> },
    Text(Cow<'a, str>),
    CData(Cow<'a, str>),
    Comment(Cow<'a, str>),
    Pi { target: Cow<'a, str>, data: Cow<'a, str> },
}

// crates/jfmt-xml/src/reader.rs
pub struct EventReader<R: Read> { /* wraps quick_xml::Reader */ }
impl<R: Read> EventReader<R> {
    pub fn new(reader: R) -> Self;
    pub fn next_event(&mut self) -> Result<Option<XmlEvent<'_>>>;
}

// crates/jfmt-xml/src/writer.rs
pub trait EventWriter { fn write_event(&mut self, ev: &XmlEvent) -> Result<()>; fn finish(self) -> Result<()>; }
pub struct XmlWriter<W: Write> { /* indented or compact */ }
impl<W: Write> XmlWriter<W> {
    pub fn new(w: W) -> Self;
    pub fn with_config(w: W, cfg: XmlPrettyConfig) -> Self;
}

pub struct XmlPrettyConfig { pub indent: usize, pub tabs: bool, pub xml_decl: bool }
```

Names mirror `jfmt-core` (`EventReader`, `EventWriter`) for
consistency. `Cow<'a, str>` lets quick-xml's zero-copy borrowing
propagate when possible without forcing it.

### 6.3 quick-xml Choice

quick-xml is the fastest pure-Rust XML library, well-maintained, MSRV
1.56+ (≤ our 1.75), zero-copy SAX-style. Alternatives considered:

- `xml-rs`: pure-Rust, slower, less actively maintained.
- `roxmltree`: DOM-only, not streaming.
- `libxml` bindings: C dep, MSRV unclear, complicates `cargo install`.

quick-xml version pinned via Task 1 spike (see plan), recorded in
spec Annex when M7 plan is written.

## 7. Error Handling

### 7.1 Library Errors (`jfmt-xml`)

`thiserror`-based enums, line/column attached:

```rust
pub enum XmlError {
    Io(io::Error),
    Parse { line: u64, column: u64, message: String },
    UnexpectedEof,
    Encoding(String),
    InvalidName(String),  // when JSON→XML produces an invalid XML element/attr name
}
```

### 7.2 CLI Errors (`jfmt-cli`)

`anyhow::Result` at the top of `commands/convert.rs`, downcast in
`exit.rs` to map to exit codes.

### 7.3 Exit Codes

Reuse existing scheme (defined in `crates/jfmt-cli/src/exit.rs`); add
**34** for `--strict` non-contiguous siblings violation. Tentative full
table for convert:

| Code | Meaning |
|---|---|
| 0 | Success |
| 2 | CLI usage error (bad flag combination, ambiguous format detection) |
| 10 | I/O error (file not found, permission denied, broken pipe) |
| 20 | JSON syntax error in input |
| 21 | XML syntax error in input |
| 33 | (from M5) JSON Schema validation failure — unrelated to convert |
| 34 | XML→JSON `--strict` mode: non-contiguous same-name siblings |
| 40 | Translation error (e.g. JSON→XML invalid element name, JSON top-level not an object without `--root`) |

`33` reserved for the M5 schema mode and not reused. `40` is new for
translation-layer errors that aren't pure parse/IO failures.

## 8. Testing Strategy

Same shape as Phase 1.

- **Unit tests** in `#[cfg(test)] mod tests` next to the code in
  `jfmt-xml` (parser, writer, error types).
- **Property tests** (`proptest`) in `crates/jfmt-xml/tests/` and
  `crates/jfmt-cli/tests/proptest_convert.rs`:
  - `xml → json → xml` structural equivalence (ignoring documented losses
    from §4.4).
  - `json → xml → json` structural equivalence in default array mode.
  - Generators must produce: nested elements, attributes (incl. `xmlns:*`),
    namespaces, mixed content, CDATA, contiguous-only same-name siblings
    (the streaming-friendly subset).
- **Golden fixtures** in `crates/jfmt-cli/tests/fixtures/convert/`:
  - `atom_feed.xml` (real-world Atom feed snippet)
  - `svg_path.xml` (SVG with namespaces + nested elements)
  - `data_records.xml` (`<root><record/><record/>...</root>` structure)
  - `mixed_content.xml` (text + element interleaving)
  - `noncontiguous_siblings.xml` (triggers warning / `--strict` exit 34)
  - Each with golden `.json` counterparts.
- **CLI e2e** (`assert_cmd`) in `crates/jfmt-cli/tests/cli_convert.rs`:
  - Each flag at least one happy path + one error path.
  - Exit-code assertions for every distinct code added in §7.3.
  - stdin/stdout, file/file, gz, zst all covered for at least one direction.
- **Big-file** (`--features big-tests`): 1 GB single-document XML→JSON to
  confirm constant memory. Generated in-process; no fixture committed.

## 9. Acceptance Criteria

M7 ships when:

1. `cargo test --workspace` is green, including new proptest suites with
   ≥ 256 cases each.
2. `cargo clippy --workspace --all-targets -- -D warnings` is clean.
3. `cargo fmt --all -- --check` is clean.
4. All CLI scenarios in §3 work end-to-end against the golden fixtures.
5. README has a `## Convert` usage section with at least the four core
   examples (file→file, stdin→stdout, `--array-rule`, `--root`).
6. CHANGELOG `[0.2.0]` section documents the convert command and the
   non-contiguous-siblings caveat.
7. Workspace `version = "0.2.0"`; tag `v0.2.0` pushed; cargo-dist
   release.yml succeeds and the GitHub Release lists 5 platform binaries.
8. Phase 1 spec gets a "Phase 1b: M7 ✓ Shipped …" line in its milestone
   table.

## 10. Future Work / Out of Scope (revisit after M7 ships)

| Item | Status |
|---|---|
| NDJSON-of-XML and XML→record-stream | v0.3.0 candidate (Phase 1c) |
| YAML support | v0.3.0 or later (separate `jfmt-yaml` crate, similar pattern) |
| SQL dump → NDJSON | confirmed deferred during M7 brainstorming; future `jfmt-sql` crate |
| Alternative mapping conventions (BadgerFish, JsonML) | reactive — add `--mapping` only if user demand surfaces |
| DTD / XSD validation | out of jfmt scope (use `xmllint`) |
| XPath / XSLT | out of jfmt scope |
| Wildcards in `--array-rule` paths | reactive — add if real users hit the verbosity wall |

## Annex A — quick-xml + transitive pins (frozen by Task 1 spike)

- Version: quick-xml=0.39.2.
- Transitive precise pins required (if any): none required.
- MSRV 1.75 confirmed by `cargo run --example quickxml_spike`.
- API shape confirmed: `Reader::from_str`, `read_event_into`, `Writer::write_event`,
  `Event::{Start, End, Empty, Text, CData, Comment, PI, Decl, DocType, Eof}`.
- Note: `write_event` consumes the event (not a reference); this is correctly
  handled by the writer's `Into<Event>` bound and the spike validates the
  expected round-trip behavior.

## 11. Open Questions

None at spec-approval time. Items resolved during brainstorming:

- Mapping convention → @attr/#text (xml-js / xmltodict default).
- Array rule → always-array default + `--array-rule` opt-out.
- Mixed content → concatenate `#text` nodes; document the order loss.
- Namespaces → preserve prefix verbatim.
- JSON→XML root → require single top-level key, `--root NAME` rescue, `--strict` removes the rescue.
- Streaming clash on non-contiguous siblings → warn + position-preserving form by default; `--strict` errors with exit 34.
- NDJSON scope → out for v0.2.0.
- Crate layout → new `jfmt-xml` crate, parallel to `jfmt-core`.
- XML library → `quick-xml`.
- SQL dump → out for v0.2.0.
- Combined milestone vs split → combined (M7 is a single shippable feature).
