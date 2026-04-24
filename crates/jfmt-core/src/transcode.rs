//! Drive an [`EventReader`] into an [`EventWriter`], closing the pipeline.

use crate::parser::EventReader;
use crate::writer::EventWriter;
use crate::Result;
use std::io::Read;

/// Read every event from `reader` and emit it into `writer`, then finish.
pub fn transcode<R: Read, EW: EventWriter>(reader: R, mut writer: EW) -> Result<()> {
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
        transcode(input.as_slice(), MinifyWriter::new(&mut out)).unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            r#"{"a":[1,2,3],"b":"hi"}"#
        );
    }

    #[test]
    fn transcode_pretty_reformats() {
        let input = br#"{"a":[1,2]}"#;
        let mut out = Vec::new();
        transcode(input.as_slice(), PrettyWriter::new(&mut out)).unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "{\n  \"a\": [\n    1,\n    2\n  ]\n}\n"
        );
    }
}
