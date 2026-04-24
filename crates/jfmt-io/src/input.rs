use crate::compress::Compression;
use std::io::BufRead;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct InputSpec {
    pub path: Option<PathBuf>,
    pub compression: Option<Compression>,
}

pub fn open_input(_spec: &InputSpec) -> std::io::Result<Box<dyn BufRead>> {
    todo!("implemented in Task 11")
}
