//! How a whole-file transfer ended.
//!
//! A transfer ends in one of three ways that are not failures of the control session: it moved
//! the bytes, the caller's own bound stopped it, or the guest declined the operation. None of
//! those is a [`crate::ControlError`], because none of them says anything is wrong with the
//! transport, and a caller has to be able to tell them apart to decide what to do next.

use core::fmt;

use crate::FileFailure;

/// The result of reading a whole file into memory.
#[derive(Clone, Eq, PartialEq)]
pub enum WholeFileRead {
    /// The complete contents of the file.
    Bytes(Vec<u8>),
    /// The file held more bytes than the caller admitted, so nothing is returned.
    TooLarge,
    /// The guest did not perform the read.
    Failed(FileFailure),
}

impl fmt::Debug for WholeFileRead {
    /// Reports the shape and never the bytes, which are tenant data.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bytes(bytes) => write!(formatter, "Bytes({} bytes)", bytes.len()),
            Self::TooLarge => formatter.write_str("TooLarge"),
            Self::Failed(failure) => write!(formatter, "Failed({failure})"),
        }
    }
}

/// The result of writing a whole file from memory.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WholeFileWrite {
    /// Every byte reached the guest and the file ends where they end.
    Written,
    /// The caller offered more bytes than it admitted, so nothing was sent.
    TooLarge,
    /// The guest did not perform the write.
    Failed(FileFailure),
}
