//! Open an input source as a boxed `BufRead`, applying decompression.

use crate::compress::Compression;
use flate2::read::MultiGzDecoder;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Read};
use std::path::PathBuf;

/// Spec describing where input comes from and whether/how to decompress it.
#[derive(Debug, Clone, Default)]
pub struct InputSpec {
    /// Source path. `None` = stdin.
    pub path: Option<PathBuf>,
    /// Forced compression. `None` = auto-detect from path extension
    /// (stdin with `None` is always treated as uncompressed).
    pub compression: Option<Compression>,
}

impl InputSpec {
    pub fn stdin() -> Self {
        Self::default()
    }
    pub fn file(p: impl Into<PathBuf>) -> Self {
        Self {
            path: Some(p.into()),
            compression: None,
        }
    }
}

/// Open the input described by `spec` and return a boxed `BufRead`.
pub fn open_input(spec: &InputSpec) -> io::Result<Box<dyn BufRead>> {
    let raw: Box<dyn Read> = match &spec.path {
        Some(p) => Box::new(File::open(p)?),
        None => Box::new(io::stdin().lock()),
    };

    let compression = spec
        .compression
        .unwrap_or_else(|| match spec.path.as_deref() {
            Some(p) => Compression::from_path(p),
            None => Compression::None,
        });

    let decoded: Box<dyn Read> = match compression {
        Compression::None => raw,
        Compression::Gzip => Box::new(MultiGzDecoder::new(raw)),
        Compression::Zstd => Box::new(zstd::stream::Decoder::new(raw)?),
    };

    Ok(Box::new(BufReader::with_capacity(64 * 1024, decoded)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    fn tempfile_with(content: &[u8], ext: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join(format!("x.{}", ext));
        std::fs::write(&p, content).unwrap();
        (dir, p)
    }

    fn read_to_string(spec: InputSpec) -> String {
        let mut r = open_input(&spec).unwrap();
        let mut s = String::new();
        r.read_to_string(&mut s).unwrap();
        s
    }

    #[test]
    fn reads_plain_file() {
        let (_d, p) = tempfile_with(b"hello", "json");
        assert_eq!(read_to_string(InputSpec::file(p)), "hello");
    }

    #[test]
    fn decompresses_gzip_by_extension() {
        let mut gz = Vec::new();
        {
            let mut enc = flate2::write::GzEncoder::new(&mut gz, flate2::Compression::default());
            enc.write_all(b"hello gz").unwrap();
            enc.finish().unwrap();
        }
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("x.json.gz");
        std::fs::write(&p, &gz).unwrap();
        assert_eq!(read_to_string(InputSpec::file(p)), "hello gz");
    }

    #[test]
    fn decompresses_zstd_by_extension() {
        let encoded = zstd::encode_all(&b"hello zstd"[..], 0).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("x.json.zst");
        std::fs::write(&p, encoded).unwrap();
        assert_eq!(read_to_string(InputSpec::file(p)), "hello zstd");
    }

    #[test]
    fn forced_compression_overrides_extension() {
        let mut gz = Vec::new();
        {
            let mut enc = flate2::write::GzEncoder::new(&mut gz, flate2::Compression::default());
            enc.write_all(b"forced").unwrap();
            enc.finish().unwrap();
        }
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("x.json");
        std::fs::write(&p, &gz).unwrap();
        let spec = InputSpec {
            path: Some(p),
            compression: Some(Compression::Gzip),
        };
        assert_eq!(read_to_string(spec), "forced");
    }
}
