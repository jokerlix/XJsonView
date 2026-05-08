# jfmt M9 — Regex Search, Subtree Scoping, Subtree Export Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add regex search, subtree-scoped search, and `export_subtree` to the viewer; ship as v0.4.0.

**Architecture:** Backend extends `SearchQuery` with `mode: SearchMode { Substring | Regex }` and `from_node: Option<NodeId>`; `run_search` switches matcher implementation up-front and reads only the scoped byte range when `from_node` is set, with the path stack pre-seeded from `from_node`'s parent chain so emitted hits keep absolute paths. New `Session::export_subtree(node, path, opts)` writes the subtree to disk; new `export_subtree` Tauri command wraps it. Frontend gets a `.*` regex toggle, a "scope: /path" chip, a right-click context menu, and a clickable "Export full subtree" button on the truncation marker.

**Tech Stack additions:** `regex = "1"` (workspace dep). No new frontend libs.

**Spec:** `docs/superpowers/specs/2026-05-09-jfmt-m9-search-enhance-design.md`
**Predecessor:** v0.3.0 at commit `49eeb78` / tag `v0.3.0`.

**Out of scope (M10+):** number / bool / null leaf search, fuzzy matching, path-glob filtering, search-and-replace, saved searches, sidecar index, code signing.

---

## Task 1: Add `regex` workspace dep + `SearchMode` + `InvalidQuery` error

**Files:**
- Modify: `Cargo.toml` (workspace dep)
- Modify: `crates/jfmt-viewer-core/Cargo.toml`
- Modify: `crates/jfmt-viewer-core/src/error.rs`
- Modify: `crates/jfmt-viewer-core/src/search.rs`

- [ ] **Step 1: Append failing test in `error.rs`**

In the existing `mod tests` block of `crates/jfmt-viewer-core/src/error.rs`, append:

```rust
    #[test]
    fn invalid_query_displays_with_message() {
        let err = ViewerError::InvalidQuery("unbalanced ( in pattern".into());
        assert_eq!(err.to_string(), "invalid query: unbalanced ( in pattern");
        let s = serde_json::to_string(&err).unwrap();
        assert!(s.contains("InvalidQuery"), "got {s}");
    }
```

- [ ] **Step 2: Run; expect FAIL**

```bash
cargo test -p jfmt-viewer-core invalid_query 2>&1 | tail -5
```
Expected: `no variant ... named 'InvalidQuery'`.

- [ ] **Step 3: Add `InvalidQuery` to the error enum**

Edit `crates/jfmt-viewer-core/src/error.rs`. Add a variant inside `pub enum ViewerError`, alphabetical (between `Io` and `NotFound`):

```rust
    #[error("invalid query: {0}")]
    InvalidQuery(String),
```

- [ ] **Step 4: Add `regex` to workspace deps**

Edit root `Cargo.toml`. In `[workspace.dependencies]`, append:

```toml
# M9 — regex search.
regex = "1"
```

Edit `crates/jfmt-viewer-core/Cargo.toml`. Append to `[dependencies]`:

```toml
regex.workspace = true
```

- [ ] **Step 5: Add `SearchMode` enum in `search.rs`**

In `crates/jfmt-viewer-core/src/search.rs`, just below the existing `pub enum SearchScope { ... }` block, add:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SearchMode {
    Substring,
    Regex,
}

impl Default for SearchMode {
    fn default() -> Self {
        SearchMode::Substring
    }
}
```

Add `mode` field to `SearchQuery`. Replace the existing definition with:

```rust
#[derive(Debug, Clone, Deserialize)]
pub struct SearchQuery {
    pub needle: String,
    #[serde(default)]
    pub mode: SearchMode,
    pub case_sensitive: bool,
    pub scope: SearchScope,
}
```

(`from_node` is added in Task 3 — keep this scoped to mode for now.)

- [ ] **Step 6: Update existing search tests to set mode**

In each of the four `run_search(...)` calls in `mod tests`, the `SearchQuery { ... }` literal now has a new field. Add `mode: SearchMode::Substring,` to each. Example for `finds_value_match`:

```rust
            &SearchQuery {
                needle: "Alice".into(),
                mode: SearchMode::Substring,
                case_sensitive: true,
                scope: SearchScope::Both,
            },
```

Apply identically to `case_insensitive_finds_mixed_case`, `key_scope_finds_keys_only`, `cancel_stops_scan`.

- [ ] **Step 7: Re-export `SearchMode` from lib.rs**

Edit `crates/jfmt-viewer-core/src/lib.rs`. Update the existing `pub use search::{ ... }` line to include `SearchMode`:

```rust
pub use search::{run_search, MatchedIn, SearchHit, SearchMode, SearchQuery, SearchScope, SearchSummary};
```

- [ ] **Step 8: Run tests; expect PASS**

```bash
cargo test -p jfmt-viewer-core 2>&1 | tail -5
```
Expected: `test result: ok. 36 passed` (35 prior + 1 new in error.rs).

- [ ] **Step 9: Run clippy**

```bash
cargo clippy -p jfmt-viewer-core --all-targets -- -D warnings 2>&1 | tail -3
```
If clippy complains about manual `Default` impl on `SearchMode`, switch to `#[derive(Default)]` and mark the `Substring` variant with `#[default]`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum SearchMode {
    #[default]
    Substring,
    Regex,
}
```

(Drop the manual `impl Default` block.)

- [ ] **Step 10: Commit**

```bash
git add Cargo.toml Cargo.lock crates/jfmt-viewer-core/Cargo.toml crates/jfmt-viewer-core/src/error.rs crates/jfmt-viewer-core/src/search.rs crates/jfmt-viewer-core/src/lib.rs
git commit -m "$(cat <<'EOF'
feat(viewer-core): SearchMode + InvalidQuery + regex dep

M9 starts. Adds the `regex` workspace dep, `SearchMode { Substring,
Regex }` enum (default Substring), and `ViewerError::InvalidQuery`
for ill-formed regex patterns. SearchQuery gains a `mode` field;
existing tests updated to set `mode: Substring`. The regex matcher
itself lands in Task 2.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 2: Implement regex search path

**Files:**
- Modify: `crates/jfmt-viewer-core/src/search.rs`

- [ ] **Step 1: Append failing tests**

Inside `mod tests` in `crates/jfmt-viewer-core/src/search.rs`, append:

```rust
    #[test]
    fn regex_finds_anchor_pattern() {
        let s = small_session();
        let cancel = Arc::new(AtomicBool::new(false));
        let mut hits = Vec::new();
        run_search(
            &s,
            &SearchQuery {
                needle: "^Al".into(),
                mode: SearchMode::Regex,
                case_sensitive: true,
                scope: SearchScope::Values,
            },
            &cancel,
            |hit| hits.push(hit.clone()),
            |_, _, _| {},
        )
        .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].matched_in, MatchedIn::Value);
    }

    #[test]
    fn regex_invalid_pattern_errors() {
        let s = small_session();
        let cancel = Arc::new(AtomicBool::new(false));
        let err = run_search(
            &s,
            &SearchQuery {
                needle: "(".into(),
                mode: SearchMode::Regex,
                case_sensitive: true,
                scope: SearchScope::Both,
            },
            &cancel,
            |_| {},
            |_, _, _| {},
        )
        .unwrap_err();
        assert!(matches!(err, crate::ViewerError::InvalidQuery(_)));
    }

    #[test]
    fn regex_case_insensitive() {
        let s = small_session();
        let cancel = Arc::new(AtomicBool::new(false));
        let mut hits = Vec::new();
        run_search(
            &s,
            &SearchQuery {
                needle: "alice".into(),
                mode: SearchMode::Regex,
                case_sensitive: false,
                scope: SearchScope::Values,
            },
            &cancel,
            |hit| hits.push(hit.clone()),
            |_, _, _| {},
        )
        .unwrap();
        assert_eq!(hits.len(), 1);
    }
```

- [ ] **Step 2: Run; expect FAIL**

```bash
cargo test -p jfmt-viewer-core search::tests::regex 2>&1 | tail -10
```
Expected: 3 fails — current `run_search` ignores `mode` and treats every needle as substring.

- [ ] **Step 3: Implement the matcher abstraction**

In `crates/jfmt-viewer-core/src/search.rs`, near the top (below the imports), add:

```rust
use regex::{Regex, RegexBuilder};

/// One of two pre-compiled matchers, chosen at the start of run_search.
enum Matcher {
    Substring(Vec<u8>, bool), // (lowercased-or-original needle bytes, case_sensitive)
    Regex(Regex),
}

impl Matcher {
    fn build(query: &SearchQuery) -> Result<Self> {
        if query.needle.is_empty() {
            // Empty query — caller must short-circuit before reaching here, but
            // an empty Substring matcher is harmless (it never matches).
            return Ok(Matcher::Substring(Vec::new(), query.case_sensitive));
        }
        match query.mode {
            SearchMode::Substring => {
                let bytes = if query.case_sensitive {
                    query.needle.as_bytes().to_vec()
                } else {
                    query.needle.to_ascii_lowercase().into_bytes()
                };
                Ok(Matcher::Substring(bytes, query.case_sensitive))
            }
            SearchMode::Regex => {
                let mut b = RegexBuilder::new(&query.needle);
                b.case_insensitive(!query.case_sensitive);
                let re = b
                    .build()
                    .map_err(|e| crate::ViewerError::InvalidQuery(e.to_string()))?;
                Ok(Matcher::Regex(re))
            }
        }
    }

    fn is_match(&self, haystack: &str) -> bool {
        match self {
            Matcher::Substring(needle, case_sensitive) => {
                if needle.is_empty() {
                    return false;
                }
                if *case_sensitive {
                    memchr::memmem::find(haystack.as_bytes(), needle).is_some()
                } else if haystack.is_ascii() {
                    memchr::memmem::find(&haystack.as_bytes().to_ascii_lowercase(), needle)
                        .is_some()
                } else {
                    haystack
                        .to_lowercase()
                        .contains(std::str::from_utf8(needle).unwrap_or(""))
                }
            }
            Matcher::Regex(re) => re.is_match(haystack),
        }
    }

    fn first_match_range(&self, haystack: &str) -> Option<(usize, usize)> {
        match self {
            Matcher::Substring(needle, case_sensitive) => {
                if needle.is_empty() {
                    return None;
                }
                if *case_sensitive {
                    let idx = memchr::memmem::find(haystack.as_bytes(), needle)?;
                    Some((idx, idx + needle.len()))
                } else if haystack.is_ascii() {
                    let idx = memchr::memmem::find(
                        &haystack.as_bytes().to_ascii_lowercase(),
                        needle,
                    )?;
                    Some((idx, idx + needle.len()))
                } else {
                    let lower = haystack.to_lowercase();
                    let idx = lower.find(std::str::from_utf8(needle).ok()?)?;
                    // The lowercase haystack may have different byte length than
                    // the original — fall back to a best-effort start at the same
                    // codepoint index. For snippet purposes this is sufficient.
                    Some((idx, idx + needle.len()))
                }
            }
            Matcher::Regex(re) => {
                let m = re.find(haystack)?;
                Some((m.start(), m.end()))
            }
        }
    }
}
```

- [ ] **Step 4: Replace the in-loop matching with `Matcher`**

In `run_search`, before the `loop {` line, replace the existing `needle_lc` / `needle_bytes` / empty-needle short-circuit with:

```rust
    if query.needle.trim().is_empty() {
        return Ok(SearchSummary { total_hits: 0, cancelled: false });
    }
    let matcher = Matcher::build(query)?;
```

Inside the loop, replace the existing `contains_match(...)` calls with `matcher.is_match(...)` and the `snippet(...)` calls with the new helper that takes the matcher (next sub-step).

Replace the existing `snippet` function with:

```rust
fn snippet(haystack: &str, matcher: &Matcher) -> String {
    let bytes = haystack.as_bytes();
    let (m_start, m_end) = matcher.first_match_range(haystack).unwrap_or((0, 0));
    let raw_start = m_start.saturating_sub(SNIPPET_RADIUS);
    let raw_end = (m_end + SNIPPET_RADIUS).min(bytes.len());

    let mut start = raw_start;
    while start > 0 && !haystack.is_char_boundary(start) {
        start -= 1;
    }
    let mut end = raw_end;
    while end < bytes.len() && !haystack.is_char_boundary(end) {
        end += 1;
    }
    let prefix = if start > 0 { "…" } else { "" };
    let suffix = if end < bytes.len() { "…" } else { "" };
    let before_match = &haystack[start..m_start];
    let matched = &haystack[m_start..m_end];
    let after_match = &haystack[m_end..end];
    format!("{prefix}{before_match}**{matched}**{after_match}{suffix}")
}
```

Inside the `Event::Name(k)` arm of `run_search`, replace the snippet/contains call with:

```rust
                if do_keys && matcher.is_match(&k) {
                    total += 1;
                    let path = build_path(&path_segments, &k);
                    let hit = SearchHit {
                        node: None,
                        path,
                        matched_in: MatchedIn::Key,
                        snippet: snippet(&k, &matcher),
                    };
                    on_hit(&hit);
                }
                pending_key = Some(k);
```

Inside the `Event::Value(scalar)` arm, the `if do_values { if let Scalar::String(val) = &scalar { ... } }` block becomes:

```rust
                if do_values {
                    if let Scalar::String(val) = &scalar {
                        if matcher.is_match(val) {
                            total += 1;
                            let path = build_path(&path_segments, &seg);
                            let hit = SearchHit {
                                node: None,
                                path,
                                matched_in: MatchedIn::Value,
                                snippet: snippet(val, &matcher),
                            };
                            on_hit(&hit);
                        }
                    }
                }
```

Delete the now-unused `contains_match` function from the file.

- [ ] **Step 5: Run tests; expect PASS**

```bash
cargo test -p jfmt-viewer-core 2>&1 | tail -5
```
Expected: `test result: ok. 39 passed` (36 prior + 3 new).

- [ ] **Step 6: Run clippy**

```bash
cargo clippy -p jfmt-viewer-core --all-targets -- -D warnings 2>&1 | tail -3
```
Expected: clean. The unused `contains_match` removal should resolve any leftover `dead_code` lint.

- [ ] **Step 7: Commit**

```bash
git add crates/jfmt-viewer-core/src/search.rs
git commit -m "$(cat <<'EOF'
feat(viewer-core): regex matcher in run_search

run_search builds a Matcher (Substring | Regex) once before the
event loop and dispatches per-key / per-value matches through it.
Regex compile errors return ViewerError::InvalidQuery. Snippet
extraction uses Matcher::first_match_range so the highlighted span
is always the actual matched bytes, not just the substring needle.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 3: Subtree scoping (`SearchQuery.from_node`)

**Files:**
- Modify: `crates/jfmt-viewer-core/src/search.rs`
- Modify: `crates/jfmt-viewer-core/src/types.rs` (no — already has the import we need; verify)

- [ ] **Step 1: Append failing tests**

Append to `mod tests` in `search.rs`:

```rust
    #[test]
    fn subtree_scope_excludes_outside_hits() {
        let s = small_session();
        // Find users[0] container.
        let users = s
            .get_children(NodeId::ROOT, 0, 100)
            .unwrap()
            .items
            .into_iter()
            .find(|c| c.key == "users")
            .unwrap();
        let users_id = users.id.unwrap();
        let user0 = s
            .get_children(users_id, 0, 100)
            .unwrap()
            .items
            .into_iter()
            .find(|c| c.key == "0")
            .unwrap();
        let user0_id = user0.id.unwrap();

        let cancel = Arc::new(AtomicBool::new(false));
        let mut hits = Vec::new();
        run_search(
            &s,
            &SearchQuery {
                needle: "name".into(),
                mode: SearchMode::Substring,
                case_sensitive: true,
                scope: SearchScope::Keys,
                from_node: Some(user0_id),
            },
            &cancel,
            |hit| hits.push(hit.clone()),
            |_, _, _| {},
        )
        .unwrap();
        // Without from_node, two `name` keys would match (users[0].name and users[1].name).
        // Scoped to users[0], only one matches.
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].path, "/users/0/name");
    }

    #[test]
    fn subtree_scope_emits_absolute_paths() {
        let s = small_session();
        let users = s
            .get_children(NodeId::ROOT, 0, 100)
            .unwrap()
            .items
            .into_iter()
            .find(|c| c.key == "users")
            .unwrap();
        let users_id = users.id.unwrap();

        let cancel = Arc::new(AtomicBool::new(false));
        let mut hits = Vec::new();
        run_search(
            &s,
            &SearchQuery {
                needle: "Alice".into(),
                mode: SearchMode::Substring,
                case_sensitive: true,
                scope: SearchScope::Values,
                from_node: Some(users_id),
            },
            &cancel,
            |hit| hits.push(hit.clone()),
            |_, _, _| {},
        )
        .unwrap();
        assert_eq!(hits.len(), 1);
        // Path is absolute from the file root, not relative to the scope.
        assert_eq!(hits[0].path, "/users/0/name");
    }
```

- [ ] **Step 2: Add `from_node` field to `SearchQuery`**

In `crates/jfmt-viewer-core/src/search.rs`, replace the existing `SearchQuery` struct with:

```rust
#[derive(Debug, Clone, Deserialize)]
pub struct SearchQuery {
    pub needle: String,
    #[serde(default)]
    pub mode: SearchMode,
    pub case_sensitive: bool,
    pub scope: SearchScope,
    #[serde(default)]
    pub from_node: Option<NodeId>,
}
```

Update existing tests' `SearchQuery { ... }` literals (the four from earlier + the three regex tests + the two new subtree tests) to set `from_node: None,`. Eight places total. Look for `SearchQuery {` and add the field after `scope:`.

- [ ] **Step 3: Run; expect FAIL on the two subtree tests**

```bash
cargo test -p jfmt-viewer-core search::tests::subtree 2>&1 | tail -10
```
Expected: 2 fails (current run_search ignores `from_node`).

- [ ] **Step 4: Implement subtree scoping**

In `run_search`, replace the existing `let bytes = std::fs::read(session.path())?;` and subsequent reader setup with:

```rust
    let bytes = std::fs::read(session.path())?;
    let (slice, mut path_segments, mut next_index_per_depth) =
        if let Some(node) = query.from_node {
            let entry = session
                .index()
                .entries
                .get(node.0 as usize)
                .ok_or(crate::ViewerError::InvalidNode)?;
            let slice =
                bytes[entry.file_offset as usize..entry.byte_end as usize].to_vec();
            // Walk parent chain from the node down to root, then reverse to get
            // root-first segments. These are the segments that prefix every
            // SearchHit.path the inner loop emits.
            let mut segs: Vec<String> = Vec::new();
            let mut cur = node;
            loop {
                let e = &session.index().entries[cur.0 as usize];
                match e.parent {
                    Some(p) => {
                        segs.push(e.key_or_index.as_str().to_string());
                        cur = p;
                    }
                    None => break,
                }
            }
            segs.reverse();
            // Each segment in `segs` corresponds to one open container at the
            // start of the scoped scan. Initialize next_index_per_depth with a
            // 0 entry per depth so consume_step does not underflow when the
            // first event is StartObject inside the scoped slice.
            let depth_count = segs.len();
            (slice, segs, vec![0u32; depth_count])
        } else {
            (bytes.clone(), Vec::new(), Vec::new())
        };
    let mut reader = EventReader::new_unlimited(&slice[..]);
    let total_bytes_len = slice.len() as u64;
    let mut last_progress_at: u64 = 0;
    let mut pending_key: Option<String> = None;
```

(Replace the previous `let mut reader = EventReader::new_unlimited(&bytes[..]);` line and the surrounding initialization.)

The rest of the loop body stays the same — it already uses `path_segments` and `next_index_per_depth`. Note that `build_path(&path_segments, &leaf)` is now correct because `path_segments` includes the parent-chain prefix.

**Edge case**: when scoped to root (`from_node: Some(NodeId::ROOT)`), `parent` of root is `None`, so the loop breaks immediately and `segs` is empty — equivalent to whole-file behavior. Acceptable.

**Edge case**: when scoped to a leaf NodeId (impossible — leaves have no NodeId), `entries.get` returns the container. So `from_node` always points at a container, by construction.

- [ ] **Step 5: Run tests; expect PASS**

```bash
cargo test -p jfmt-viewer-core 2>&1 | tail -5
```
Expected: `test result: ok. 41 passed` (39 prior + 2 new).

If `subtree_scope_emits_absolute_paths` fails with a path like `/0/name` (missing the `/users` prefix), the parent-chain initialization is broken — re-check segs reversal.

- [ ] **Step 6: Run clippy**

```bash
cargo clippy -p jfmt-viewer-core --all-targets -- -D warnings 2>&1 | tail -3
```
Expected: clean.

- [ ] **Step 7: Commit**

```bash
git add crates/jfmt-viewer-core/src/search.rs
git commit -m "$(cat <<'EOF'
feat(viewer-core): subtree-scoped search via from_node

SearchQuery gains an optional from_node: NodeId. When set,
run_search reads only the entry's byte range and pre-seeds the
path-segments stack from the node's parent chain, so emitted
SearchHits keep absolute RFC 6901 paths from the file root.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 4: `Session::export_subtree` core API

**Files:**
- Modify: `crates/jfmt-viewer-core/src/session.rs`
- Modify: `crates/jfmt-viewer-core/src/lib.rs`

- [ ] **Step 1: Append failing tests in `session.rs`**

Inside the existing `mod tests`, append:

```rust
    #[test]
    fn export_subtree_root_pretty_matches_get_value() {
        let s = small_session();
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let bytes = s
            .export_subtree(NodeId::ROOT, tmp.path(), ExportOptions { pretty: true })
            .unwrap();
        assert!(bytes > 0);
        let written = std::fs::read_to_string(tmp.path()).unwrap();
        let via_get_value = s.get_value(NodeId::ROOT, None).unwrap().json;
        // get_value's pretty output should match what export_subtree writes.
        assert_eq!(written, via_get_value);
    }

    #[test]
    fn export_subtree_compact_is_smaller_than_pretty() {
        let s = small_session();
        let pretty_tmp = tempfile::NamedTempFile::new().unwrap();
        let compact_tmp = tempfile::NamedTempFile::new().unwrap();
        let pretty_bytes = s
            .export_subtree(NodeId::ROOT, pretty_tmp.path(), ExportOptions { pretty: true })
            .unwrap();
        let compact_bytes = s
            .export_subtree(NodeId::ROOT, compact_tmp.path(), ExportOptions { pretty: false })
            .unwrap();
        assert!(compact_bytes < pretty_bytes);
    }

    #[test]
    fn export_subtree_invalid_node() {
        let s = small_session();
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let err = s
            .export_subtree(NodeId(9999), tmp.path(), ExportOptions { pretty: true })
            .unwrap_err();
        assert!(matches!(err, crate::ViewerError::InvalidNode));
    }
```

`tempfile` is already a dev-dep on `jfmt-viewer-core`.

- [ ] **Step 2: Run; expect FAIL**

```bash
cargo test -p jfmt-viewer-core session::tests::export 2>&1 | tail -5
```
Expected: cannot find type `ExportOptions` / method `export_subtree`.

- [ ] **Step 3: Implement**

In `crates/jfmt-viewer-core/src/session.rs`, near the other public types (around `GetValueResp`), add:

```rust
#[derive(Debug, Clone, Copy, Deserialize)]
pub struct ExportOptions {
    pub pretty: bool,
}
```

Add `use serde::Deserialize;` to the imports if not already present.

In `impl Session`, add the method. Place it after `get_pointer`:

```rust
    pub fn export_subtree(
        &self,
        node: NodeId,
        target: &std::path::Path,
        options: ExportOptions,
    ) -> Result<u64> {
        let entry = self
            .index
            .entries
            .get(node.0 as usize)
            .ok_or(ViewerError::InvalidNode)?;
        let slice = self.find_container_slice(entry)?;

        let value: serde_json::Value =
            serde_json::from_slice(slice).map_err(|e| ViewerError::Parse {
                pos: entry.file_offset,
                msg: e.to_string(),
            })?;
        let serialized = if options.pretty {
            serde_json::to_vec_pretty(&value)
        } else {
            serde_json::to_vec(&value)
        }
        .map_err(|e| ViewerError::Parse {
            pos: entry.file_offset,
            msg: e.to_string(),
        })?;

        std::fs::write(target, &serialized)
            .map_err(|e| ViewerError::Io(e.to_string()))?;
        Ok(serialized.len() as u64)
    }

    /// Slice the file's bytes from the entry's container start (skipping any
    /// leading whitespace / colon between the recorded `file_offset` and the
    /// actual `{` or `[`) up to `byte_end`.
    fn find_container_slice<'a>(
        &'a self,
        entry: &crate::types::ContainerEntry,
    ) -> Result<&'a [u8]> {
        let raw_start = entry.file_offset as usize;
        let raw_end = entry.byte_end as usize;
        // Same forward-scan workaround used by get_children / get_value.
        let bytes = &self.bytes[raw_start..raw_end];
        let skip = bytes
            .iter()
            .position(|&b| b == b'{' || b == b'[')
            .unwrap_or(0);
        Ok(&bytes[skip..])
    }
```

If `find_container_slice` already exists in the file (added by Task 8 of M8.1 with a different name), reuse it instead of duplicating. The grep:

```bash
grep -n "fn find_container" crates/jfmt-viewer-core/src/session.rs
```

If a similar helper exists, point `export_subtree` at it directly.

- [ ] **Step 4: Re-export `ExportOptions` from lib.rs**

Edit `crates/jfmt-viewer-core/src/lib.rs`. Update the existing `pub use session::{ ... }` line to include `ExportOptions`:

```rust
pub use session::{ExportOptions, Format, GetChildrenResp, GetValueResp, Session};
```

- [ ] **Step 5: Run tests; expect PASS**

```bash
cargo test -p jfmt-viewer-core 2>&1 | tail -5
```
Expected: `test result: ok. 44 passed` (41 prior + 3 new).

- [ ] **Step 6: Run clippy**

```bash
cargo clippy -p jfmt-viewer-core --all-targets -- -D warnings 2>&1 | tail -3
```
Expected: clean.

- [ ] **Step 7: Commit**

```bash
git add crates/jfmt-viewer-core/src/session.rs crates/jfmt-viewer-core/src/lib.rs
git commit -m "$(cat <<'EOF'
feat(viewer-core): Session::export_subtree

Pretty or compact JSON export of a node's subtree to a target file
path. Returns bytes written. Reuses the existing forward-scan
workaround for the file_offset → opening-brace gap. ExportOptions
is the public arg struct so future flags (compress, ndjson) extend
without breaking the API.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 5: Tauri `export_subtree` command + permissions

**Files:**
- Modify: `apps/jfmt-viewer/src-tauri/src/commands.rs`
- Modify: `apps/jfmt-viewer/src-tauri/src/lib.rs`
- Modify: `apps/jfmt-viewer/src-tauri/capabilities/default.json` (add `dialog:allow-save`)
- Modify: `apps/jfmt-viewer/src/api.ts`

- [ ] **Step 1: Add the command in `commands.rs`**

Append at the end of `apps/jfmt-viewer/src-tauri/src/commands.rs`:

```rust
#[derive(Serialize)]
pub struct ExportSubtreeResp {
    pub bytes_written: u64,
    pub elapsed_ms: u64,
}

#[tauri::command]
pub async fn export_subtree(
    session_id: String,
    node: u64,
    target_path: String,
    pretty: bool,
    state: State<'_, ViewerState>,
) -> Result<ExportSubtreeResp, ViewerError> {
    let session = state
        .sessions
        .get(&session_id)
        .ok_or(ViewerError::InvalidSession)?
        .clone();
    let started = Instant::now();
    let target = std::path::PathBuf::from(target_path);
    let bytes_written = tokio::task::spawn_blocking(move || {
        session.export_subtree(
            jfmt_viewer_core::NodeId(node),
            &target,
            jfmt_viewer_core::ExportOptions { pretty },
        )
    })
    .await
    .map_err(|e| ViewerError::Io(e.to_string()))??;
    Ok(ExportSubtreeResp {
        bytes_written,
        elapsed_ms: started.elapsed().as_millis() as u64,
    })
}
```

The required `use` paths are already present (`Instant`, `State`, `ViewerError`, `Serialize`).

- [ ] **Step 2: Register the command in `lib.rs`**

Edit `apps/jfmt-viewer/src-tauri/src/lib.rs`. Append `commands::export_subtree,` to the `tauri::generate_handler![ ... ]` list:

```rust
        .invoke_handler(tauri::generate_handler![
            commands::open_file,
            commands::close_file,
            commands::get_children,
            commands::get_value,
            commands::get_pointer,
            commands::search,
            commands::cancel_search,
            commands::export_subtree,
        ])
```

- [ ] **Step 3: Add `dialog:allow-save` to capabilities**

Edit `apps/jfmt-viewer/src-tauri/capabilities/default.json`. The `permissions` array becomes:

```json
"permissions": [
  "core:default",
  "dialog:allow-open",
  "dialog:allow-save",
  "clipboard-manager:allow-write-text"
]
```

- [ ] **Step 4: Update `api.ts`**

Edit `apps/jfmt-viewer/src/api.ts`. Append:

```ts
export interface ExportSubtreeResp {
  bytes_written: number;
  elapsed_ms: number;
}

export async function exportSubtree(
  sessionId: string,
  node: NodeId,
  targetPath: string,
  pretty: boolean,
): Promise<ExportSubtreeResp> {
  return invoke<ExportSubtreeResp>("export_subtree", {
    sessionId,
    node,
    targetPath,
    pretty,
  });
}
```

Also update the `SearchQuery` interface to add `mode` and `from_node`:

```ts
export type SearchMode = "substring" | "regex";

export interface SearchQuery {
  needle: string;
  mode: SearchMode;
  case_sensitive: boolean;
  scope: "both" | "keys" | "values";
  from_node?: NodeId;
}
```

- [ ] **Step 5: Verify Tauri builds**

```bash
cargo build -p jfmt-viewer-app 2>&1 | tail -5
```
Expected: `Finished dev`. If `tauri::generate_context!` complains about the capability addition, regenerate by deleting `apps/jfmt-viewer/src-tauri/gen/schemas/` and re-running build (Tauri auto-regenerates).

- [ ] **Step 6: Verify frontend compiles**

```bash
cd apps/jfmt-viewer && pnpm build && cd ../..
```
Expected: clean tsc + vite. The existing `useSearch` hook's calls to `start({ needle, case_sensitive, scope })` will fail TypeScript now because `SearchQuery` requires `mode` — Task 6 fixes the toolbar; for now patch:

In `apps/jfmt-viewer/src/lib/searchState.ts`, default `mode` in the query object passed to `search`:

```ts
const fullQuery: SearchQuery = { mode: "substring", from_node: undefined, ...query };
```

(Spread the user-provided fields over defaults.)

Adjust the function signature so callers can still pass partial queries during the transition; Task 6 widens the type.

Actually simpler: change the call site in `start(query: SearchQuery)` — every caller in `SearchBar.tsx` already produces a complete `SearchQuery`. Update `SearchBar.tsx` to set `mode: "substring"` in its `onQuery` payload:

```tsx
        onQuery({ needle, mode: "substring", case_sensitive: caseSensitive, scope });
```

Task 6 wires the `.*` toggle into this; the hard-coded `"substring"` is interim.

- [ ] **Step 7: Commit**

```bash
git add apps/jfmt-viewer/src-tauri apps/jfmt-viewer/src/api.ts apps/jfmt-viewer/src/components/SearchBar.tsx apps/jfmt-viewer/src/lib/searchState.ts
git commit -m "$(cat <<'EOF'
feat(viewer): export_subtree IPC command + dialog:allow-save

Tauri command wraps Session::export_subtree on a blocking pool.
Adds dialog:allow-save to the capabilities manifest. api.ts grows
exportSubtree() and updates SearchQuery to include mode +
from_node. SearchBar passes mode: "substring" until Task 6 wires
the regex toggle.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 6: Regex toggle + `InvalidQuery` UI

**Files:**
- Modify: `apps/jfmt-viewer/src/components/SearchBar.tsx`
- Modify: `apps/jfmt-viewer/src/lib/searchState.ts`

- [ ] **Step 1: SearchState carries query error**

Edit `apps/jfmt-viewer/src/lib/searchState.ts`. The `SearchState` interface already has `error: string | null`. Add a separate field for query-validation errors so they don't conflict with backend search runtime errors:

```ts
export interface SearchState {
  query: SearchQuery;
  hits: Hit[];
  totalSoFar: number;
  scanning: boolean;
  cancelled: boolean;
  error: string | null;
  hitCap: boolean;
  queryError: string | null;  // NEW
}
```

In the `useState` initializer, add `queryError: null,`. In `reset` (if present) and the `start` body's initial `setState`, add `queryError: null,`.

In the `e.kind === "error"` branch of the channel handler, distinguish: if `e.message.startsWith("invalid query:")` set `queryError` rather than `error`:

```ts
        if (e.kind === "error") {
          if (e.message.toLowerCase().startsWith("invalid query")) {
            return { ...prev, scanning: false, queryError: e.message };
          }
          return { ...prev, scanning: false, error: e.message };
        }
```

Actually, the better approach is for the `search` IPC call itself to return an error before any channel events fire. `invoke` rejects the promise on `Err(_)`. Update the `start` function:

```ts
    try {
      const handle = await search(sessionId, fullQuery, (e: SearchEvent) => { /* ... */ });
      handleRef.current = handle.id;
    } catch (err: any) {
      const msg = err?.message ?? String(err);
      if (msg.toLowerCase().startsWith("invalid query")) {
        setState((s) => ({ ...s, scanning: false, queryError: msg }));
      } else {
        setState((s) => ({ ...s, scanning: false, error: msg }));
      }
    }
```

The Tauri command's `Result<_, ViewerError>` rejects with the `Display` form of the error, which begins with `invalid query: ...` for `InvalidQuery(_)`.

- [ ] **Step 2: Add the `.*` toggle to SearchBar**

Edit `apps/jfmt-viewer/src/components/SearchBar.tsx`. Add state:

```tsx
  const [mode, setMode] = useState<SearchMode>("substring");
```

Where `SearchMode` is imported from `../api`.

Replace the `useEffect` debounce body so it includes `mode`:

```tsx
    tRef.current = window.setTimeout(() => {
      if (needle.trim() === "") {
        onCancel();
      } else {
        onQuery({ needle, mode, case_sensitive: caseSensitive, scope });
      }
    }, DEBOUNCE_MS);
```

And add `mode` to the dependency array of that `useEffect`.

Add the toggle button after the `Aa` button:

```tsx
      <button
        onClick={() => setMode((m) => (m === "regex" ? "substring" : "regex"))}
        title="Regex (toggle)"
        style={{ fontWeight: mode === "regex" ? "bold" : "normal" }}
      >
        .*
      </button>
```

Add a red-border state when `state.queryError` is non-null:

```tsx
      <input
        ref={inputRef}
        value={needle}
        onChange={(e) => setNeedle(e.target.value)}
        placeholder="🔍 search"
        title={state.queryError ?? undefined}
        style={{
          width: 200,
          padding: "2px 6px",
          fontFamily: "ui-monospace, monospace",
          fontSize: 12,
          border: state.queryError ? "1px solid #c33" : undefined,
        }}
      />
```

- [ ] **Step 3: Verify build**

```bash
cd apps/jfmt-viewer && pnpm build && cd ../..
```
Expected: clean tsc + vite.

- [ ] **Step 4: Commit**

```bash
git add apps/jfmt-viewer/src
git commit -m "$(cat <<'EOF'
feat(viewer): regex toggle + InvalidQuery UI

Toolbar gains a `.*` toggle next to Aa; toggling re-issues the
query through the existing 250ms debounce. SearchState splits
runtime error vs. query-validation error so the input border can
flash red on invalid regex without polluting the hit-list error
banner.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 7: Right-click context menu + scope chip

**Files:**
- Create: `apps/jfmt-viewer/src/components/ContextMenu.tsx`
- Modify: `apps/jfmt-viewer/src/components/Tree.tsx`
- Modify: `apps/jfmt-viewer/src/components/TreeRow.tsx`
- Modify: `apps/jfmt-viewer/src/App.tsx`
- Modify: `apps/jfmt-viewer/src/components/SearchBar.tsx` (display the scope chip)

- [ ] **Step 1: ContextMenu component**

Create `apps/jfmt-viewer/src/components/ContextMenu.tsx`:

```tsx
import { useEffect } from "react";

export interface ContextMenuItem {
  label: string;
  onClick: () => void;
}

interface Props {
  x: number;
  y: number;
  items: ContextMenuItem[];
  onDismiss: () => void;
}

export function ContextMenu({ x, y, items, onDismiss }: Props) {
  useEffect(() => {
    function onWindow(e: MouseEvent | KeyboardEvent) {
      if (e instanceof KeyboardEvent && e.key !== "Escape") return;
      onDismiss();
    }
    window.addEventListener("click", onWindow as EventListener, { once: true });
    window.addEventListener("keydown", onWindow as EventListener);
    return () => window.removeEventListener("keydown", onWindow as EventListener);
  }, [onDismiss]);

  return (
    <div
      role="menu"
      style={{
        position: "fixed",
        top: y,
        left: x,
        background: "white",
        border: "1px solid #888",
        boxShadow: "0 2px 6px rgba(0,0,0,0.2)",
        padding: 4,
        zIndex: 1000,
        fontFamily: "system-ui",
        fontSize: 13,
      }}
    >
      {items.map((it, i) => (
        <div
          key={i}
          role="menuitem"
          onClick={(e) => {
            e.stopPropagation();
            it.onClick();
            onDismiss();
          }}
          style={{
            padding: "4px 12px",
            cursor: "pointer",
          }}
          onMouseEnter={(e) => (e.currentTarget.style.background = "#eef")}
          onMouseLeave={(e) => (e.currentTarget.style.background = "transparent")}
        >
          {it.label}
        </div>
      ))}
    </div>
  );
}
```

- [ ] **Step 2: TreeRow forwards `onContextMenu`**

Edit `apps/jfmt-viewer/src/components/TreeRow.tsx`. Add prop:

```tsx
interface Props {
  child: ChildSummary;
  depth: number;
  expanded: boolean;
  onToggle: () => void;
  onSelect: () => void;
  onContextMenu?: (e: React.MouseEvent) => void;
}
```

Add to the outer `<div>`:

```tsx
      onContextMenu={(e) => {
        if (onContextMenu) {
          e.preventDefault();
          onContextMenu(e);
        }
      }}
```

- [ ] **Step 3: Tree forwards onContextMenu to its parent with the row's NodeId**

Edit `apps/jfmt-viewer/src/components/Tree.tsx`. Add a new optional prop:

```tsx
interface Props {
  sessionId: string;
  rootId: NodeId;
  onSelect?: (node: NodeId | null) => void;
  selectedId?: NodeId | null;
  onContextMenu?: (node: NodeId | null, x: number, y: number) => void;  // NEW
}
```

Inside the virtualizer's row render block, pass `onContextMenu`:

```tsx
              <TreeRow
                child={row.child}
                depth={row.depth}
                expanded={expanded}
                onToggle={() => row.child.id !== null && toggle(row.child.id)}
                onSelect={() => onSelect?.(row.child.id)}
                onContextMenu={(e) => onContextMenu?.(row.child.id, e.clientX, e.clientY)}
              />
```

- [ ] **Step 4: App.tsx wires context menu state**

Edit `apps/jfmt-viewer/src/App.tsx`. Add imports:

```tsx
import { ContextMenu, ContextMenuItem } from "./components/ContextMenu";
```

Add state:

```tsx
  const [menu, setMenu] = useState<{ node: NodeId | null; x: number; y: number } | null>(null);
  const [searchScope, setSearchScope] = useState<NodeId | undefined>(undefined);
```

Build the menu items lazily:

```tsx
  function menuItems(node: NodeId | null): ContextMenuItem[] {
    if (node === null || !session) return [];
    const items: ContextMenuItem[] = [
      {
        label: "Search from this node",
        onClick: () => setSearchScope(node),
      },
    ];
    items.push({
      label: "Export subtree…",
      onClick: () => exportSubtreeFlow(node),
    });
    return items;
  }
```

`exportSubtreeFlow` lives in Task 8; for now define a stub:

```tsx
  async function exportSubtreeFlow(_node: NodeId) {
    // Filled in Task 8.
  }
```

Wire into the Tree:

```tsx
            <Tree
              ref={treeRef}
              sessionId={session.sessionId}
              rootId={session.rootId}
              onSelect={setSelected}
              selectedId={selected}
              onContextMenu={(node, x, y) => setMenu({ node, x, y })}
            />
```

Render the menu:

```tsx
      {menu && (
        <ContextMenu
          x={menu.x}
          y={menu.y}
          items={menuItems(menu.node)}
          onDismiss={() => setMenu(null)}
        />
      )}
```

- [ ] **Step 5: Pass `searchScope` into `useSearch` + SearchBar chip**

The `useSearch` hook's `start` function takes a full `SearchQuery`. Inject `from_node: searchScope` at the App level:

In App.tsx, replace the existing `startSearch` usage. The `SearchBar` gets a wrapper:

```tsx
  function startSearchScoped(q: SearchQuery) {
    return startSearch({ ...q, from_node: searchScope });
  }
```

Pass `startSearchScoped` instead of `startSearch` into `<SearchBar onQuery=...>`. Re-issue when `searchScope` changes:

```tsx
  useEffect(() => {
    if (searchState.query.needle.trim()) {
      startSearchScoped(searchState.query);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [searchScope]);
```

Now SearchBar shows a chip when `searchScope` is set. Edit `SearchBar.tsx` to accept the new prop:

```tsx
interface Props {
  onQuery: (q: SearchQuery) => void;
  onCancel: () => void;
  state: SearchState;
  cursor: number;
  onCursorChange: (next: number) => void;
  scopePath?: string;          // NEW
  onClearScope?: () => void;   // NEW
}
```

Render the chip after the scope dropdown, before the counter:

```tsx
      {scopePath && (
        <span
          onClick={onClearScope}
          title="Click to clear scope"
          style={{
            background: "#eef",
            border: "1px solid #aac",
            padding: "2px 6px",
            fontSize: 11,
            cursor: "pointer",
            borderRadius: 3,
          }}
        >
          scope: {scopePath} ✕
        </span>
      )}
```

App.tsx must compute and pass `scopePath`. Wrap `getPointer` from the api:

```tsx
  const [scopePath, setScopePath] = useState<string>("");

  useEffect(() => {
    (async () => {
      if (!session || searchScope === undefined) {
        setScopePath("");
        return;
      }
      const p = await getPointer(session.sessionId, searchScope);
      setScopePath(p || "/");
    })();
  }, [session, searchScope]);
```

Pass `scopePath` and `onClearScope={() => setSearchScope(undefined)}` into `<SearchBar />`.

- [ ] **Step 6: Build**

```bash
cd apps/jfmt-viewer && pnpm build && cd ../..
```
Expected: clean.

- [ ] **Step 7: Commit**

```bash
git add apps/jfmt-viewer/src
git commit -m "$(cat <<'EOF'
feat(viewer): right-click menu + subtree scope chip

Tree rows respond to onContextMenu by surfacing a "Search from this
node" + "Export subtree…" menu. Selecting "Search from…" sets a
searchScope state that propagates as from_node to all subsequent
queries; a scope chip near the search input shows the active path
and clears on click.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 8: Export flow + truncation marker button

**Files:**
- Modify: `apps/jfmt-viewer/src/App.tsx`
- Modify: `apps/jfmt-viewer/src/components/Preview.tsx`
- Create: `apps/jfmt-viewer/src/lib/exportFlow.ts`

- [ ] **Step 1: Export helper**

Create `apps/jfmt-viewer/src/lib/exportFlow.ts`:

```ts
import { save } from "@tauri-apps/plugin-dialog";
import { exportSubtree, NodeId } from "../api";

export async function runExportFlow(
  sessionId: string,
  node: NodeId,
  defaultName = "subtree.json",
): Promise<string | null> {
  const path = await save({
    defaultPath: defaultName,
    filters: [{ name: "JSON", extensions: ["json"] }],
  });
  if (!path) return null;
  const r = await exportSubtree(sessionId, node, path, true);
  return `Exported ${r.bytes_written} bytes to ${path}`;
}
```

- [ ] **Step 2: Wire `exportSubtreeFlow` in App.tsx**

Replace the Task-7 stub:

```tsx
  async function exportSubtreeFlow(node: NodeId) {
    if (!session) return;
    const ptr = await getPointer(session.sessionId, node);
    const safe = (ptr || "root").replace(/[/~]/g, "_").replace(/^_/, "");
    const result = await runExportFlow(session.sessionId, node, `${safe || "root"}.json`);
    if (result) {
      setPointerHint(result);
      setTimeout(() => setPointerHint(""), 4000);
    }
  }
```

Add to imports:

```tsx
import { runExportFlow } from "./lib/exportFlow";
```

- [ ] **Step 3: Preview pane truncation button**

Edit `apps/jfmt-viewer/src/components/Preview.tsx`. The current truncated state renders a `<span>` saying "see truncation marker above; full export ships in M9". Replace with a button + the parent component is responsible for wiring.

New `Props`:

```tsx
interface Props {
  sessionId: string;
  node: NodeId | null;
  onExport?: (node: NodeId) => void;
}
```

Replace the truncated state block:

```tsx
      {truncated && node !== null && (
        <button
          onClick={() => onExport?.(node)}
          style={{
            display: "block",
            margin: "8px 0",
            padding: "4px 12px",
            background: "#fee",
            border: "1px solid #c66",
            cursor: "pointer",
          }}
        >
          Export full subtree →
        </button>
      )}
```

App.tsx passes `onExport={exportSubtreeFlow}` into `<Preview />`.

- [ ] **Step 4: Build + smoke**

```bash
cd apps/jfmt-viewer && pnpm build && cd ../..
```
Expected: clean.

Manual smoke (`pnpm tauri dev`): open a fixture, right-click `users` → "Export subtree…", save to a temp file, verify file content with a separate tool (`jq . file.json`).

- [ ] **Step 5: Commit**

```bash
git add apps/jfmt-viewer/src
git commit -m "$(cat <<'EOF'
feat(viewer): export flow + preview truncation button

Right-click "Export subtree…" opens the platform save-as dialog
(tauri-plugin-dialog::save) and writes the pretty-printed subtree
via export_subtree IPC. Preview pane's M8 truncation marker
becomes a "Export full subtree →" button that re-uses the same
flow with a sensible default file name (pointer-derived).

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 9: E2E spec for regex + export

**Files:**
- Create: `apps/jfmt-viewer/e2e/specs/regex-and-export.e2e.ts`

- [ ] **Step 1: Write the spec**

Create `apps/jfmt-viewer/e2e/specs/regex-and-export.e2e.ts`:

```ts
import { browser, $ } from "@wdio/globals";
import { resolve, dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { mkdtempSync, readFileSync } from "node:fs";
import { tmpdir } from "node:os";

const __dirname = dirname(fileURLToPath(import.meta.url));
const FIXTURE = resolve(
  __dirname,
  "../../../../crates/jfmt-viewer-core/tests/fixtures/small.json",
);

describe("regex + export", () => {
  before(async () => {
    await browser.url(`tauri://localhost?file=${encodeURIComponent(FIXTURE)}`);
    await $("strong=users").waitForExist({ timeout: 10_000 });
  });

  it("regex search finds anchored value", async () => {
    // Click the .* toggle.
    await $("button[title='Regex (toggle)']").click();
    const input = await $("input[placeholder='🔍 search']");
    await input.click();
    await input.setValue("^Al");
    // Hit list should render exactly one row.
    const hit = await $("div=K /users/0/name").catch(() => null);
    // Hit's exact text varies; check for substring instead.
    await browser.waitUntil(
      async () => {
        const matches = await $$("div*=Alice");
        return matches.length > 0;
      },
      { timeout: 5_000, timeoutMsg: "expected 'Alice' in hit list" },
    );
  });

  it("export_subtree writes the root subtree", async () => {
    const dir = mkdtempSync(join(tmpdir(), "jfmt-viewer-e2e-"));
    const out = join(dir, "root.json");

    // Right-click root → Export subtree → mock the save dialog return.
    // tauri-driver does not intercept native dialogs; for the e2e we use the
    // browser-side runtime's window.__TAURI_DIALOG_SAVE_PATH__ override that
    // App.tsx honors when set. (See lib/exportFlow.ts for the override hook.)

    await browser.execute((p: string) => {
      (window as unknown as Record<string, unknown>).__TAURI_DIALOG_SAVE_PATH__ = p;
    }, out);

    // Right-click users.
    const users = await $("strong=users");
    await users.click({ button: "right" });
    const exportItem = await $("div*=Export subtree");
    await exportItem.click();

    // Wait for the pointer-hint flash that confirms write.
    await browser.waitUntil(
      async () => {
        const hints = await $$("span*=Exported");
        return hints.length > 0;
      },
      { timeout: 10_000, timeoutMsg: "expected export confirmation" },
    );

    const contents = readFileSync(out, "utf8");
    const parsed = JSON.parse(contents);
    if (!Array.isArray(parsed.users)) {
      throw new Error("expected exported users array");
    }
  });
});
```

- [ ] **Step 2: Add the dialog-save override hook**

Edit `apps/jfmt-viewer/src/lib/exportFlow.ts`. Update `runExportFlow`:

```ts
import { save } from "@tauri-apps/plugin-dialog";
import { exportSubtree, NodeId } from "../api";

export async function runExportFlow(
  sessionId: string,
  node: NodeId,
  defaultName = "subtree.json",
): Promise<string | null> {
  // E2E hook: bypass the native dialog when __TAURI_DIALOG_SAVE_PATH__ is set.
  const w = window as unknown as Record<string, unknown>;
  const overridePath = typeof w.__TAURI_DIALOG_SAVE_PATH__ === "string"
    ? (w.__TAURI_DIALOG_SAVE_PATH__ as string)
    : null;
  const path =
    overridePath ??
    (await save({
      defaultPath: defaultName,
      filters: [{ name: "JSON", extensions: ["json"] }],
    }));
  if (!path) return null;
  const r = await exportSubtree(sessionId, node, path, true);
  return `Exported ${r.bytes_written} bytes to ${path}`;
}
```

This is a minimal e2e seam. The override is only set during E2E and is safe in production (runtime `window` doesn't define it).

- [ ] **Step 3: Build (no e2e run on Windows)**

```bash
cd apps/jfmt-viewer && pnpm build && cd ../..
```
Expected: clean. CI runs the e2e on Linux (existing workflow already picks up `specs/**/*.e2e.ts`).

- [ ] **Step 4: Commit**

```bash
git add apps/jfmt-viewer/e2e apps/jfmt-viewer/src/lib/exportFlow.ts
git commit -m "$(cat <<'EOF'
test(viewer): E2E coverage for regex + export_subtree

Two specs: regex toggle finds an anchored pattern; export_subtree
on the root produces a parseable JSON with the expected shape.
E2E uses a window-level override for the native save dialog so
WebDriver can substitute a tempfile path without driving the OS
dialog (which tauri-driver cannot script).

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 10: README + CHANGELOG + v0.4.0 tag

**Files:**
- Modify: `README.md`
- Modify: `CHANGELOG.md`
- Modify: `Cargo.toml` (root version)
- Modify: `apps/jfmt-viewer/src-tauri/Cargo.toml`
- Modify: `apps/jfmt-viewer/package.json`
- Modify: `apps/jfmt-viewer/src-tauri/tauri.conf.json`
- Modify: `crates/jfmt-viewer-core/Cargo.toml`

- [ ] **Step 1: Update README**

Edit `README.md`'s `## View` section. Replace the bullets list with:

```markdown
- Virtual scrolling for trees with millions of nodes
- Right-pane preview of the selected subtree (pretty-printed)
- Toolbar substring **and regex** search across keys and string-leaf values
  (case-insensitive default; ASCII fast path for substrings; Unicode-aware
  regex via the `regex` crate)
- "Search from this node" right-click menu scopes a query to a subtree;
  matches still report absolute JSON Pointer paths
- Right-click "Export subtree…" writes any selected subtree to disk;
  the preview pane's truncation marker doubles as a one-click export
- One-click copy of the selected node's JSON Pointer (RFC 6901)
```

- [ ] **Step 2: CHANGELOG**

Edit `CHANGELOG.md`. Insert under `## [Unreleased]`:

```markdown
## [0.4.0] - 2026-05-09

### Added

- Regex search in the GUI viewer (`.*` toggle next to `Aa`).
  Invalid patterns surface a red border on the input with the
  full error in a tooltip.
- Subtree-scoped search (`from_node`). Right-click any tree row →
  "Search from this node"; matches outside the subtree are
  excluded but `SearchHit.path` stays absolute.
- `export_subtree` IPC command + UI. Right-click → "Export
  subtree…" or click "Export full subtree →" on the preview
  pane's truncation marker.
- New `regex` workspace dep.

### Changed

- `SearchQuery` gains `mode: "substring" | "regex"` and an
  optional `from_node`. Existing callers that pass JSON without
  `mode` continue to work because both fields default to their
  prior behaviour (`substring` and the whole file).
- `ViewerError` gains `InvalidQuery(String)`.
```

- [ ] **Step 3: Bump versions**

Replace `version = "0.3.0"` with `version = "0.4.0"` in:
- Root `Cargo.toml` (under `[workspace.package]`)
- `crates/jfmt-viewer-core/Cargo.toml`
- `apps/jfmt-viewer/src-tauri/Cargo.toml`

Replace `"version": "0.3.0"` with `"version": "0.4.0"` in:
- `apps/jfmt-viewer/package.json`
- `apps/jfmt-viewer/src-tauri/tauri.conf.json`

- [ ] **Step 4: Verify everything still builds**

```bash
cargo test --workspace 2>&1 | grep -E "^test result:" | tail -25
cargo clippy --workspace --all-targets -- -D warnings 2>&1 | tail -3
cargo fmt --all -- --check
cd apps/jfmt-viewer && pnpm build && cd ../..
```
All four must succeed.

- [ ] **Step 5: Commit**

```bash
git add README.md CHANGELOG.md Cargo.toml Cargo.lock apps/jfmt-viewer/src-tauri/Cargo.toml apps/jfmt-viewer/src-tauri/tauri.conf.json apps/jfmt-viewer/package.json crates/jfmt-viewer-core/Cargo.toml
git commit -m "$(cat <<'EOF'
chore: bump version to 0.4.0 (M9 — regex + scope + export)

M9 ships:
- Regex search in the viewer (.* toggle)
- Subtree-scoped search (right-click → Search from this node)
- export_subtree IPC + UI (right-click → Export subtree…;
  preview truncation marker becomes a button)

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

- [ ] **Step 6: Tag (USER CONFIRMATION REQUIRED before pushing)**

```bash
git tag -a v0.4.0 -m "v0.4.0 — regex search + subtree scoping + export"
```

Stop here and report. The user pushes when ready; pushing the tag triggers cargo-dist + viewer-release.yml.

---

## Plan summary

10 tasks. Final state: rustc 1.85.1, all viewer-core tests + E2E green, cargo-dist + viewer-release continue to work, v0.4.0 tag created locally.
