//! Drain-only syntax validator. Consumes events without writing output.

use crate::parser::EventReader;
use crate::Result;
use std::io::Read;

/// Read every event from `reader` to confirm the document is syntactically valid.
/// Returns `Ok(())` iff the document parses cleanly.
pub fn validate_syntax<R: Read>(reader: R) -> Result<()> {
    let mut r = EventReader::new(reader);
    while r.next_event()?.is_some() {}
    r.finish()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Error;

    #[test]
    fn accepts_valid_document() {
        let input = br#"{"a":[1,2,3],"b":null}"#;
        validate_syntax(input.as_slice()).unwrap();
    }

    #[test]
    fn accepts_scalars_and_empties() {
        validate_syntax(br#"null"#.as_slice()).unwrap();
        validate_syntax(br#"[]"#.as_slice()).unwrap();
        validate_syntax(br#"{}"#.as_slice()).unwrap();
        validate_syntax(br#""hi""#.as_slice()).unwrap();
        validate_syntax(br#"42"#.as_slice()).unwrap();
    }

    #[test]
    fn rejects_trailing_garbage() {
        let res = validate_syntax(br#"{"a":1} garbage"#.as_slice());
        assert!(matches!(res, Err(Error::Syntax { .. })), "got {res:?}");
    }

    #[test]
    fn rejects_truncated_input() {
        let res = validate_syntax(br#"{"a":"#.as_slice());
        assert!(matches!(res, Err(Error::Syntax { .. })), "got {res:?}");
    }
}
