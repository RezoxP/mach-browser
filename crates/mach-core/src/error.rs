//! Error hierarchy shared by every mach crate.
//!
//! Each subsystem maps its own errors into this enum at the crate boundary.
//! Keeps the dependency graph acyclic while still giving the CLI one type to
//! match on for exit-code selection.

use std::io;

/// Top-level error returned across crate boundaries.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Argument-level problem detected before any I/O ran.
    #[error("invalid arguments: {0}")]
    InvalidArguments(String),

    /// Network or HTTP-protocol failure.
    #[error("network error: {0}")]
    Network(String),

    /// HTML/DOM parsing failure.
    #[error("parse error: {0}")]
    Parse(String),

    /// Local filesystem or I/O failure.
    #[error("i/o error: {0}")]
    Io(#[from] io::Error),

    /// Anything else we don't have a structured variant for yet.
    #[error("{0}")]
    Other(String),
}

impl Error {
    /// Exit code for CLI use. See README for the contract.
    pub fn exit_code(&self) -> i32 {
        match self {
            Error::InvalidArguments(_) => 3,
            Error::Parse(_) => 2,
            Error::Network(_) => 1,
            Error::Io(_) | Error::Other(_) => 1,
        }
    }
}

/// Convenience alias.
pub type Result<T, E = Error> = std::result::Result<T, E>;
