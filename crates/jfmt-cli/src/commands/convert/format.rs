//! Format detection: extension and CLI flags → Format enum.

use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Format {
    Json,
    Xml,
}

/// Strip recognized compression suffixes (.gz, .zst) and return the
/// remaining path's "data extension."
pub fn data_extension(path: &Path) -> Option<String> {
    let mut s = path.to_string_lossy().to_string();
    for sfx in [".gz", ".zst"] {
        if let Some(stripped) = s.strip_suffix(sfx) {
            s = stripped.to_string();
        }
    }
    Path::new(&s)
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
}

pub fn infer_from_path(path: &Path) -> Option<Format> {
    match data_extension(path).as_deref() {
        Some("xml") => Some(Format::Xml),
        Some("json") | Some("ndjson") => Some(Format::Json),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn json_ext() {
        assert_eq!(infer_from_path(Path::new("a.json")), Some(Format::Json));
        assert_eq!(infer_from_path(Path::new("a.json.gz")), Some(Format::Json));
        assert_eq!(infer_from_path(Path::new("a.json.zst")), Some(Format::Json));
        assert_eq!(infer_from_path(Path::new("a.ndjson")), Some(Format::Json));
    }

    #[test]
    fn xml_ext() {
        assert_eq!(infer_from_path(Path::new("a.xml")), Some(Format::Xml));
        assert_eq!(infer_from_path(Path::new("a.xml.gz")), Some(Format::Xml));
    }

    #[test]
    fn unknown_ext() {
        assert_eq!(infer_from_path(Path::new("a.txt")), None);
        assert_eq!(infer_from_path(Path::new("noext")), None);
    }
}
