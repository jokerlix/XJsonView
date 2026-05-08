//! Streaming substring search across keys and string-leaf values.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::session::Session;
use crate::types::NodeId;
use jfmt_core::event::{Event, Scalar};
use jfmt_core::parser::EventReader;

#[derive(Debug, Clone, Deserialize)]
pub struct SearchQuery {
    pub needle: String,
    pub case_sensitive: bool,
    pub scope: SearchScope,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SearchScope {
    Both,
    Keys,
    Values,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MatchedIn {
    Key,
    Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct SearchHit {
    pub node: Option<NodeId>,
    pub path: String,
    pub matched_in: MatchedIn,
    pub snippet: String,
}

#[derive(Debug, Serialize)]
pub struct SearchSummary {
    pub total_hits: u32,
    pub cancelled: bool,
}

const SNIPPET_RADIUS: usize = 32;

pub fn run_search<F: FnMut(u64, u64, u32)>(
    session: &Session,
    query: &SearchQuery,
    cancel: &Arc<AtomicBool>,
    mut on_hit: impl FnMut(&SearchHit),
    mut on_progress: F,
) -> Result<SearchSummary> {
    if cancel.load(Ordering::Relaxed) {
        return Ok(SearchSummary {
            total_hits: 0,
            cancelled: true,
        });
    }

    let needle_lc;
    let needle_bytes: &[u8] = if query.case_sensitive {
        query.needle.as_bytes()
    } else {
        needle_lc = query.needle.to_ascii_lowercase();
        needle_lc.as_bytes()
    };
    if needle_bytes.is_empty() {
        return Ok(SearchSummary {
            total_hits: 0,
            cancelled: false,
        });
    }

    let bytes = std::fs::read(session.path())?;
    let mut reader = EventReader::new_unlimited(&bytes[..]);
    let mut total: u32 = 0;
    // path_segments[i] is the key/index of the container at depth i+1
    // (i.e. the segment that points INTO that depth).
    let mut path_segments: Vec<String> = Vec::new();
    let mut next_index_per_depth: Vec<u32> = Vec::new();
    let mut pending_key: Option<String> = None;

    let do_keys = matches!(query.scope, SearchScope::Both | SearchScope::Keys);
    let do_values = matches!(query.scope, SearchScope::Both | SearchScope::Values);

    let total_bytes_len = bytes.len() as u64;
    let mut last_progress_at: u64 = 0;

    loop {
        if cancel.load(Ordering::Relaxed) {
            return Ok(SearchSummary {
                total_hits: total,
                cancelled: true,
            });
        }
        let now_pos = reader.byte_offset();
        if now_pos.saturating_sub(last_progress_at) >= 1_048_576 {
            on_progress(now_pos, total_bytes_len, total);
            last_progress_at = now_pos;
        }
        let pos = reader.byte_offset();
        let ev = match reader.next_event() {
            Ok(Some(e)) => e,
            Ok(None) => break,
            Err(e) => {
                return Err(crate::ViewerError::Parse {
                    pos,
                    msg: e.to_string(),
                });
            }
        };
        match ev {
            Event::StartObject | Event::StartArray => {
                // The segment that points into this new container is determined
                // by the parent context: pending_key for object children, or
                // the next array index. Same logic as a leaf "step".
                let seg = consume_step(&mut pending_key, next_index_per_depth.last_mut());
                path_segments.push(seg);
                next_index_per_depth.push(0);
            }
            Event::EndObject | Event::EndArray => {
                path_segments.pop();
                next_index_per_depth.pop();
            }
            Event::Name(k) => {
                if do_keys && contains_match(&k, needle_bytes, query.case_sensitive) {
                    total += 1;
                    let path = build_path(&path_segments, &k);
                    let hit = SearchHit {
                        node: None,
                        path,
                        matched_in: MatchedIn::Key,
                        snippet: snippet(&k, needle_bytes, query.case_sensitive),
                    };
                    on_hit(&hit);
                }
                pending_key = Some(k);
            }
            Event::Value(scalar) => {
                let seg = consume_step(&mut pending_key, next_index_per_depth.last_mut());
                if do_values {
                    if let Scalar::String(val) = &scalar {
                        if contains_match(val, needle_bytes, query.case_sensitive) {
                            total += 1;
                            let path = build_path(&path_segments, &seg);
                            let hit = SearchHit {
                                node: None,
                                path,
                                matched_in: MatchedIn::Value,
                                snippet: snippet(val, needle_bytes, query.case_sensitive),
                            };
                            on_hit(&hit);
                        }
                    }
                    // Numbers, bools, null not searched in M8.1.
                }
            }
        }
    }

    on_progress(total_bytes_len, total_bytes_len, total);
    Ok(SearchSummary {
        total_hits: total,
        cancelled: false,
    })
}

/// Consume one step into the parent container: pulls the pending object key
/// or post-increments the array index. Returns the segment (always non-empty
/// for object children; numeric for array/ndjson children; "" for top-level).
fn consume_step(pending_key: &mut Option<String>, next_index: Option<&mut u32>) -> String {
    if let Some(k) = pending_key.take() {
        return k;
    }
    if let Some(idx) = next_index {
        let s = idx.to_string();
        *idx += 1;
        return s;
    }
    String::new()
}

fn contains_match(haystack: &str, needle: &[u8], case_sensitive: bool) -> bool {
    if case_sensitive {
        memchr::memmem::find(haystack.as_bytes(), needle).is_some()
    } else if haystack.is_ascii() {
        memchr::memmem::find(&haystack.as_bytes().to_ascii_lowercase(), needle).is_some()
    } else {
        haystack
            .to_lowercase()
            .contains(std::str::from_utf8(needle).unwrap_or(""))
    }
}

fn snippet(haystack: &str, needle: &[u8], case_sensitive: bool) -> String {
    let bytes = haystack.as_bytes();
    let lower;
    let probe = if case_sensitive {
        bytes
    } else {
        lower = bytes.to_ascii_lowercase();
        &lower[..]
    };
    let idx = memchr::memmem::find(probe, needle).unwrap_or(0);
    let raw_start = idx.saturating_sub(SNIPPET_RADIUS);
    let raw_end = (idx + needle.len() + SNIPPET_RADIUS).min(bytes.len());

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
    let before_match = &haystack[start..idx];
    let matched = &haystack[idx..idx + needle.len()];
    let after_match = &haystack[idx + needle.len()..end];
    format!("{prefix}{before_match}**{matched}**{after_match}{suffix}")
}

/// Build a JSON Pointer to a child of the current container, given the
/// already-walked container path and the leaf segment.
fn build_path(segments: &[String], leaf: &str) -> String {
    let mut all: Vec<&str> = segments.iter().map(|s| s.as_str()).collect();
    all.push(leaf);
    crate::pointer::encode_pointer(&all)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Session;
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;

    fn small_session() -> Session {
        let path = format!("{}/tests/fixtures/small.json", env!("CARGO_MANIFEST_DIR"));
        Session::open(path).unwrap()
    }

    #[test]
    fn finds_value_match() {
        let s = small_session();
        let cancel = Arc::new(AtomicBool::new(false));
        let mut hits = Vec::new();
        let summary = run_search(
            &s,
            &SearchQuery {
                needle: "Alice".into(),
                case_sensitive: true,
                scope: SearchScope::Both,
            },
            &cancel,
            |hit| hits.push(hit.clone()),
            |_, _, _| {},
        )
        .unwrap();
        assert_eq!(summary.total_hits, 1);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].matched_in, MatchedIn::Value);
        assert!(hits[0].snippet.contains("**Alice**"));
    }

    #[test]
    fn case_insensitive_finds_mixed_case() {
        let s = small_session();
        let cancel = Arc::new(AtomicBool::new(false));
        let mut hits = Vec::new();
        run_search(
            &s,
            &SearchQuery {
                needle: "alice".into(),
                case_sensitive: false,
                scope: SearchScope::Values,
            },
            &cancel,
            |h| hits.push(h.clone()),
            |_, _, _| {},
        )
        .unwrap();
        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn key_scope_finds_keys_only() {
        let s = small_session();
        let cancel = Arc::new(AtomicBool::new(false));
        let mut hits = Vec::new();
        run_search(
            &s,
            &SearchQuery {
                needle: "name".into(),
                case_sensitive: true,
                scope: SearchScope::Keys,
            },
            &cancel,
            |h| hits.push(h.clone()),
            |_, _, _| {},
        )
        .unwrap();
        assert!(hits.iter().all(|h| h.matched_in == MatchedIn::Key));
        assert_eq!(hits.len(), 2); // two `name` keys in users[]
    }

    #[test]
    fn cancel_stops_scan() {
        let s = small_session();
        let cancel = Arc::new(AtomicBool::new(true)); // pre-cancelled
        let mut hits = Vec::new();
        let summary = run_search(
            &s,
            &SearchQuery {
                needle: "x".into(),
                case_sensitive: false,
                scope: SearchScope::Both,
            },
            &cancel,
            |h| hits.push(h.clone()),
            |_, _, _| {},
        )
        .unwrap();
        assert!(summary.cancelled);
    }
}
