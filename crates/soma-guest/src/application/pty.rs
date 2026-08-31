//! An interactive pseudo-terminal carried by one authenticated record at a time.
//!
//! Running a command and reading what it printed is not the same capability as sitting at a
//! shell: an agent that has to answer a prompt, interrupt a build, or drive a program that only
//! speaks to a terminal needs a real pseudo-terminal, and every provider in this market exposes
//! one as open, write, read, resize, and close with the dimensions given up front.
//!
//! The name here is `pty` rather than `terminal` because this protocol already spends that word:
//! a [`TerminalReport`](super::TerminalReport) is the exit status of one command, and calling two
//! unrelated things by one name would make every match arm below ambiguous.
//!
//! One record carries a bounded body, so terminal output cannot be a stream on the wire. A read
//! answers with one bounded chunk and a flag saying whether the session has ended, exactly as a
//! bounded filesystem read answers with one chunk and the end of a file, and a caller drains a
//! terminal by reading until that flag is set. Nothing here invents a second framing layer
//! inside the record layer that already bounds one message.
//!
//! A read carries the longest it may wait for the first byte. Without it a caller with nothing
//! to read would have to spin, and a request that blocked without a bound would hold the one
//! serial channel open against the session's own deadlines.

mod codec;
mod outcome;

#[cfg(test)]
mod tests;

pub use outcome::{PtyFailure, PtyOutcome};

use core::fmt;

use crate::Error;

/// Largest number of bytes one terminal request or one terminal answer may carry.
///
/// The value matches the output chunk the command path already uses, so the resident cost of
/// one terminal exchange is the same fixed buffer as one chunk of command output.
pub const MAX_PTY_CHUNK_BYTES: usize = 4096;
/// Widest terminal this protocol will carry, in character cells.
///
/// A terminal is a grid a human or a program draws into. Nothing draws into sixty five thousand
/// columns, so a request for one is a caller defect rather than a request, and admitting it
/// would let a caller make the guest hold a screen out of all proportion to what it is for.
pub const MAX_PTY_COLUMNS: u16 = 1024;
/// Tallest terminal this protocol will carry, in character cells.
pub const MAX_PTY_ROWS: u16 = 1024;
/// Longest one read may wait for the first output byte before it answers with none.
pub const MAX_PTY_WAIT_MILLIS: u32 = 60_000;

/// The dimensions of one terminal, in character cells.
///
/// Both are validated on the way in and on the way out, so a zero or an out-of-range dimension
/// is refused at the decoder rather than reaching an `ioctl` that would report back a window
/// size the guest never actually had.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PtySize {
    columns: u16,
    rows: u16,
}

impl PtySize {
    /// Creates bounded terminal dimensions.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidPtySize`] when either dimension is zero or beyond its bound.
    pub const fn new(columns: u16, rows: u16) -> Result<Self, Error> {
        if columns == 0 || rows == 0 || columns > MAX_PTY_COLUMNS || rows > MAX_PTY_ROWS {
            return Err(Error::InvalidPtySize);
        }
        Ok(Self { columns, rows })
    }

    /// Returns the width in character cells.
    #[must_use]
    pub const fn columns(self) -> u16 {
        self.columns
    }

    /// Returns the height in character cells.
    #[must_use]
    pub const fn rows(self) -> u16 {
        self.rows
    }
}

/// What a caller asks of the guest's terminal.
#[derive(Clone, Eq, PartialEq)]
pub enum PtyRequest {
    /// Opens the session with the dimensions it starts at.
    Open(PtySize),
    /// Writes bytes to the terminal as if they had been typed at it.
    Write {
        /// The bytes, at most [`MAX_PTY_CHUNK_BYTES`] of them.
        bytes: Box<[u8]>,
    },
    /// Reads at most one bounded chunk of whatever the terminal has produced.
    Read {
        /// Longest to wait for the first byte, at most [`MAX_PTY_WAIT_MILLIS`].
        wait_millis: u32,
    },
    /// Tells the terminal it has new dimensions.
    Resize(PtySize),
    /// Ends the session and everything running under it.
    Close,
}

impl fmt::Debug for PtyRequest {
    /// Names the operation and the size of what it carries, and never the bytes themselves.
    ///
    /// What a caller types at a terminal is tenant data, and a password answered at a prompt is
    /// the ordinary case rather than the exceptional one, so no byte of it may reach a log
    /// through a formatter.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Open(size) => write!(formatter, "PtyRequest::Open({size:?})"),
            Self::Write { bytes } => {
                write!(formatter, "PtyRequest::Write {{ {} bytes }}", bytes.len())
            }
            Self::Read { wait_millis } => {
                write!(
                    formatter,
                    "PtyRequest::Read {{ wait_millis: {wait_millis} }}"
                )
            }
            Self::Resize(size) => write!(formatter, "PtyRequest::Resize({size:?})"),
            Self::Close => formatter.write_str("PtyRequest::Close"),
        }
    }
}
