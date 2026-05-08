//! RFC 6901 JSON Pointer encode helpers.

/// Encode a list of unescaped path segments into an RFC 6901 JSON Pointer.
///
/// Empty input yields `""` (root). Each segment is escaped: `~` becomes `~0`,
/// `/` becomes `~1`. Tilde substitution runs first so a literal `~1` in the
/// input segment becomes `~01`, not `/`.
pub fn encode_pointer(segments: &[&str]) -> String {
    if segments.is_empty() {
        return String::new();
    }
    let mut out = String::with_capacity(
        segments.len() + segments.iter().map(|s| s.len()).sum::<usize>(),
    );
    for seg in segments {
        out.push('/');
        for c in seg.chars() {
            match c {
                '~' => out.push_str("~0"),
                '/' => out.push_str("~1"),
                _ => out.push(c),
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_path_is_root() {
        assert_eq!(encode_pointer(&[]), "");
    }

    #[test]
    fn single_segment() {
        assert_eq!(encode_pointer(&["foo"]), "/foo");
    }

    #[test]
    fn nested_segments() {
        assert_eq!(encode_pointer(&["users", "3", "name"]), "/users/3/name");
    }

    #[test]
    fn escapes_tilde_then_slash() {
        // RFC 6901 §3: `~` MUST be encoded as `~0`, `/` as `~1`.
        // Order matters: `~` is encoded first so a literal `~1` survives intact.
        assert_eq!(encode_pointer(&["a~b"]), "/a~0b");
        assert_eq!(encode_pointer(&["a/b"]), "/a~1b");
        assert_eq!(encode_pointer(&["a~/b"]), "/a~0~1b");
        assert_eq!(encode_pointer(&["~1foo"]), "/~01foo");
    }

    #[test]
    fn empty_segment_is_distinct() {
        assert_eq!(encode_pointer(&[""]), "/");
        assert_eq!(encode_pointer(&["", ""]), "//");
    }

    #[test]
    fn unicode_passthrough() {
        assert_eq!(encode_pointer(&["café", "日本"]), "/café/日本");
    }
}
