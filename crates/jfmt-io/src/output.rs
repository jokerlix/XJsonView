//! Open an output sink as a boxed `Write`, applying compression.

use crate::compress::Compression;
use flate2::write::GzEncoder;
use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::PathBuf;

/// Spec describing where output goes and whether/how to compress it.
#[derive(Debug, Clone)]
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

impl Default for OutputSpec {
    fn default() -> Self {
        Self {
            path: None,
            compression: None,
            gzip_level: 6,
            zstd_level: 3,
        }
    }
}

impl OutputSpec {
    pub fn stdout() -> Self {
        Self::default()
    }
    pub fn file(p: impl Into<PathBuf>) -> Self {
        Self {
            path: Some(p.into()),
            ..Self::default()
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
