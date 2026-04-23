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
        assert_eq!(to_string(""), "\"\"");
    }

    #[test]
    fn ascii_pass_through() {
        assert_eq!(to_string("hello"), "\"hello\"");
    }

    #[test]
    fn quote_and_backslash() {
        assert_eq!(to_string("a\"b\\c"), "\"a\\\"b\\\\c\"");
    }

    #[test]
    fn newlines_tabs() {
        assert_eq!(to_string("a\nb\tc"), "\"a\\nb\\tc\"");
    }

    #[test]
    fn control_char_uses_unicode_escape() {
        assert_eq!(to_string("\x01"), "\"\\u0001\"");
        assert_eq!(to_string("\x1f"), "\"\\u001f\"");
    }

    #[test]
    fn short_form_escapes_preferred_over_unicode() {
        // Backspace and form feed have short forms.
        assert_eq!(to_string("\x08"), "\"\\b\"");
        assert_eq!(to_string("\x0c"), "\"\\f\"");
    }

    #[test]
    fn non_ascii_passes_through() {
        assert_eq!(to_string("日本語"), "\"日本語\"");
        assert_eq!(to_string("🦀"), "\"🦀\"");
    }
}
