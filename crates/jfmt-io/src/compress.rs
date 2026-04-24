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
        assert_eq!(
            Compression::from_path(Path::new("a.json")),
            Compression::None
        );
        assert_eq!(
            Compression::from_path(Path::new("a.JSON.gz")),
            Compression::Gzip
        );
        assert_eq!(
            Compression::from_path(Path::new("a.json.zst")),
            Compression::Zstd
        );
        assert_eq!(
            Compression::from_path(Path::new("a.json.ZSTD")),
            Compression::Zstd
        );
        assert_eq!(
            Compression::from_path(Path::new("no_ext")),
            Compression::None
        );
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
