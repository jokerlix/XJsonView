//! I/O adapters: file/stdin/stdout + automatic gzip/zstd (de)compression.

pub mod compress;
pub mod input;
pub mod output;

pub use compress::Compression;
pub use input::{open_input, InputSpec};
pub use output::{open_output, OutputSpec};
