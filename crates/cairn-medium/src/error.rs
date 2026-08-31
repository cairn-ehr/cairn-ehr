//! The one error type for this crate: a medium failed to parse, or the I/O around it failed.
//! Every other module returns this on any failure path — a single type keeps a caller's match
//! arms simple, and there is no reason for e.g. `container::parse_container` and
//! `verify::verify_medium_bytes` to disagree about what "decode failed" looks like.

#[derive(thiserror::Error, Debug)]
pub enum BackupError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    /// The medium bytes are not a valid backup container (bad magic / truncated frame).
    #[error("decode: {0}")]
    Decode(String),
}
