//! Mapping of internal errors to process exit codes.

/// Exit code convention documented in the Phase 1 spec §4.3.
#[repr(i32)]
pub enum ExitCode {
    Success = 0,
    /// Generic I/O, file-not-found, bad argument.
    InputError = 1,
    /// Malformed JSON input.
    SyntaxError = 2,
    /// JSON-Schema validation failure (reserved for M5).
    _SchemaError = 3,
}

impl ExitCode {
    #[allow(clippy::wrong_self_convention)]
    pub fn as_i32(self) -> i32 {
        self as i32
    }
}
