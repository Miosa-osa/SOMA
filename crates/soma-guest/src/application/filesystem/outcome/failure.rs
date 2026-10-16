//! The closed set of causes one filesystem request can fail with.

use core::fmt;

use crate::Error;

/// Why a filesystem request did not succeed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum FileFailure {
    /// No such path.
    NotFound = 1,
    /// The guest refused the access.
    Denied = 2,
    /// A directory was named where a file was required, or the reverse.
    WrongKind = 3,
    /// The path already exists and the request required that it did not.
    Exists = 4,
    /// A directory was not empty and the request did not ask to remove its contents.
    NotEmpty = 5,
    /// The guest could not complete the operation for any other reason.
    Failed = 6,
}

impl FileFailure {
    /// Decodes one failure code.
    pub(super) fn parse(value: u8) -> Result<Self, Error> {
        match value {
            1 => Ok(Self::NotFound),
            2 => Ok(Self::Denied),
            3 => Ok(Self::WrongKind),
            4 => Ok(Self::Exists),
            5 => Ok(Self::NotEmpty),
            6 => Ok(Self::Failed),
            _ => Err(Error::ApplicationMessageRejected),
        }
    }
}

impl fmt::Display for FileFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Self::NotFound => "not found",
            Self::Denied => "denied",
            Self::WrongKind => "wrong kind",
            Self::Exists => "already exists",
            Self::NotEmpty => "not empty",
            Self::Failed => "failed",
        };
        formatter.write_str(text)
    }
}
