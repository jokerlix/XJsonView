use crate::compress::Compression;
use std::io::Write;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct OutputSpec {
    pub path: Option<PathBuf>,
    pub compression: Option<Compression>,
}

pub fn open_output(_spec: &OutputSpec) -> std::io::Result<Box<dyn Write>> {
    todo!("implemented in Task 12")
}
