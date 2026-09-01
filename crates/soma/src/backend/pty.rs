//! One bounded interactive terminal exchange against a live Instance.
//!
//! A terminal is a stream and this is not a stream, deliberately. Every transport SOMA already
//! has carries one addressed request and one bounded answer: an HTTP request, a command line
//! process, an MCP tool call, and the single JSON line a machine host relays. A stream would need
//! a second transport on every one of them, and the guest protocol already declines to be one:
//! it carries a terminal as open, write, read, resize and close, where a read answers with one
//! bounded chunk and a flag saying whether the session has ended.
//!
//! So the session lives where the machine lives, for as long as the machine does, and a caller
//! drives it with bounded calls. That is what makes an interactive program work over a
//! request-and-answer surface: the terminal keeps running between calls, the guest holds
//! whatever it produced, and a read collects it. A read carries how long it may wait for the
//! first byte, so a caller with nothing to read blocks in the guest rather than spinning against
//! it.
//!
//! Bytes in both directions are bytes rather than text, for the reason file contents are: a
//! terminal emits escape sequences and whatever the program inside it wrote, none of which is
//! required to be UTF-8.

mod answer;

pub use answer::{PtyAnswer, PtyObservation, PtyRefusal};

use core::fmt;

use serde::{Deserialize, Serialize};

use crate::{InstanceId, OperationId};

/// Largest number of bytes one terminal call carries in either direction.
///
/// The value belongs to the guest protocol, which checks it again on the way in and is the
/// authority on it. It is restated here because this crate is below the protocol crate and a
/// surface has to be able to refuse an oversized chunk before it reaches the wire. The two are
/// held equal by a test in `soma-local`, which depends on both.
///
/// This is why a terminal does not run into the bound a whole-file transfer does. A file is one
/// object of whatever size the sandbox grew it to and the host has to hold all of it; a terminal
/// is incremental by nature, and four kibibytes is one guest record rather than an accumulation.
/// Nothing on the host buffers a session's output across calls.
pub const MAX_PTY_CHUNK_BYTES: usize = 4096;

/// Widest terminal this facade will carry, in character cells.
pub const MAX_PTY_COLUMNS: u16 = 1024;
/// Tallest terminal this facade will carry, in character cells.
pub const MAX_PTY_ROWS: u16 = 1024;
/// Longest one read may wait for the first output byte before it answers with none.
pub const MAX_PTY_WAIT_MILLIS: u32 = 60_000;

/// Why a terminal call cannot be carried to a guest at all.
///
/// This is not a terminal outcome. A call of this shape is one the protocol will not encode, so
/// it never becomes a request the guest declines; it is refused where the request is built.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PtyRejected {
    /// A dimension is zero, or beyond what this protocol carries.
    Size,
    /// The input is longer than one terminal call carries.
    ChunkTooLarge,
    /// The wait is longer than a read may ask for.
    WaitTooLong,
}

impl PtyRejected {
    /// A sentence naming what is wrong, for a surface to report.
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::Size => "each terminal dimension must be between 1 and 1024 character cells",
            Self::ChunkTooLarge => "the input is longer than one terminal call carries",
            Self::WaitTooLong => "the wait is longer than one terminal read may ask for",
        }
    }
}

/// What a caller asks of one Instance's terminal.
#[derive(Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PtyOperation {
    /// Opens the one session this Instance carries, at these dimensions.
    Open {
        /// Width in character cells.
        columns: u16,
        /// Height in character cells.
        rows: u16,
    },
    /// Writes bytes to the terminal as if they had been typed at it.
    Write {
        /// The bytes, at most [`MAX_PTY_CHUNK_BYTES`] of them.
        bytes: Vec<u8>,
    },
    /// Reads at most one bounded chunk of whatever the terminal has produced.
    Read {
        /// Longest to wait for the first byte, at most [`MAX_PTY_WAIT_MILLIS`].
        wait_millis: u32,
    },
    /// Tells the terminal it has new dimensions.
    Resize {
        /// Width in character cells.
        columns: u16,
        /// Height in character cells.
        rows: u16,
    },
    /// Ends the session and everything running under it.
    Close,
}

impl PtyOperation {
    /// Refuses a call the guest protocol will not carry.
    ///
    /// A request built with one of these would not reach the guest as a terminal request at all:
    /// the guest rejects it while decoding, which is a protocol fault that ends the session, so a
    /// caller naming one would destroy its own sandbox instead of being told no. The guest still
    /// performs every one of these checks itself and remains the authority on them.
    ///
    /// # Errors
    ///
    /// Returns the shape rule the call breaks.
    pub const fn check(&self) -> Result<(), PtyRejected> {
        match self {
            Self::Open { columns, rows } | Self::Resize { columns, rows } => {
                if *columns == 0 || *rows == 0 || *columns > MAX_PTY_COLUMNS || *rows > MAX_PTY_ROWS
                {
                    return Err(PtyRejected::Size);
                }
                Ok(())
            }
            Self::Write { bytes } => {
                if bytes.len() > MAX_PTY_CHUNK_BYTES {
                    return Err(PtyRejected::ChunkTooLarge);
                }
                Ok(())
            }
            Self::Read { wait_millis } => {
                if *wait_millis > MAX_PTY_WAIT_MILLIS {
                    return Err(PtyRejected::WaitTooLong);
                }
                Ok(())
            }
            Self::Close => Ok(()),
        }
    }

    /// The operation's own name, as every surface reports it.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Open { .. } => "open",
            Self::Write { .. } => "write",
            Self::Read { .. } => "read",
            Self::Resize { .. } => "resize",
            Self::Close => "close",
        }
    }
}

impl fmt::Debug for PtyOperation {
    /// Names the operation and the sizes, and never a byte a caller typed.
    ///
    /// What is typed at a terminal is tenant data down to the password in it, so none of it may
    /// reach a log through a derived formatter. This mirrors the guest protocol's own redaction,
    /// which would otherwise be undone the moment a request was rebuilt on this side of the seam.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Open { columns, rows } => {
                write!(formatter, "PtyOperation::open {columns}x{rows}")
            }
            Self::Resize { columns, rows } => {
                write!(formatter, "PtyOperation::resize {columns}x{rows}")
            }
            Self::Write { bytes } => write!(formatter, "PtyOperation::write {} bytes", bytes.len()),
            Self::Read { wait_millis } => write!(formatter, "PtyOperation::read {wait_millis} ms"),
            Self::Close => formatter.write_str("PtyOperation::close"),
        }
    }
}

/// One terminal operation addressed to one exact Instance.
#[derive(Clone, Copy, Debug)]
pub struct PtyRequest<'a> {
    operation_id: &'a OperationId,
    instance_id: &'a InstanceId,
    operation: &'a PtyOperation,
}

impl<'a> PtyRequest<'a> {
    pub(crate) const fn new(
        operation_id: &'a OperationId,
        instance_id: &'a InstanceId,
        operation: &'a PtyOperation,
    ) -> Self {
        Self {
            operation_id,
            instance_id,
            operation,
        }
    }

    #[must_use]
    pub const fn operation_id(&self) -> &OperationId {
        self.operation_id
    }

    #[must_use]
    pub const fn instance_id(&self) -> &InstanceId {
        self.instance_id
    }

    #[must_use]
    pub const fn operation(&self) -> &PtyOperation {
        self.operation
    }
}
