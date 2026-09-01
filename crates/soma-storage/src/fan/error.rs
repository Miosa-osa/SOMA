//! Why one template fan could not be warmed.

use std::fmt;
use std::io;

/// The first failure of a warming pass.
#[derive(Debug)]
pub enum FanError {
    /// A filesystem operation failed.
    Io(io::Error),
    /// A replica's length disagreed with the template.
    SizeMismatch {
        /// Template length in bytes.
        expected: u64,
        /// Replica length in bytes.
        actual: u64,
    },
    /// A replica's bytes did not digest to the template's.
    DigestMismatch,
    /// A replica shares extents with something, so it is not physically independent and would
    /// contend on the same refcount records the template does.
    SharedExtents {
        /// Shared extents observed.
        shared: u64,
        /// Extents observed.
        extents: u64,
    },
}

impl fmt::Display for FanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "template fan io: {error}"),
            Self::SizeMismatch { expected, actual } => {
                write!(f, "replica is {actual} bytes, template is {expected}")
            }
            Self::DigestMismatch => f.write_str("replica bytes are not the template's"),
            Self::SharedExtents { shared, extents } => {
                write!(f, "replica shares {shared} of {extents} extents")
            }
        }
    }
}

impl std::error::Error for FanError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}
