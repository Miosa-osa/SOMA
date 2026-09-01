//! What one terminal operation answered with.
//!
//! A refusal is the guest's own closed set of causes, carried across unchanged, for the reason
//! the guest protocol gives: an errno leaks the guest's implementation to the host, and a message
//! leaks tenant data into whatever recorded it.

use core::fmt;

use serde::{Deserialize, Serialize};

use crate::{InstanceId, OperationId};

/// Why a terminal operation did not happen.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PtyRefusal {
    /// No terminal session is open, so there is nothing to write to, read, resize, or close.
    NoSession,
    /// A session is already open, and one Instance carries exactly one at a time.
    AlreadyOpen,
    /// The guest refused to start a terminal at all.
    Denied,
    /// The guest could not complete the operation for any other reason.
    Failed,
}

impl fmt::Display for PtyRefusal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl PtyRefusal {
    /// The stable name a surface reports this refusal by.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::NoSession => "no_session",
            Self::AlreadyOpen => "already_open",
            Self::Denied => "denied",
            Self::Failed => "failed",
        }
    }
}

/// The answer to one terminal operation.
#[derive(Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PtyAnswer {
    /// The session is open at these dimensions.
    Opened {
        /// Width in character cells.
        columns: u16,
        /// Height in character cells.
        rows: u16,
    },
    /// How many leading bytes of the write the terminal accepted, which may be fewer than offered.
    Wrote {
        /// The count.
        bytes: u32,
    },
    /// One bounded chunk of terminal output.
    Output {
        /// The bytes, possibly none.
        bytes: Vec<u8>,
        /// Whether the session has ended and no further byte will ever follow.
        ///
        /// This is the explicit end of the stream. A caller drains a terminal by reading until
        /// this is set rather than by guessing from an empty chunk, which only means nothing
        /// arrived within the wait it asked for.
        end: bool,
    },
    /// The terminal was told its new dimensions.
    Resized {
        /// Width in character cells.
        columns: u16,
        /// Height in character cells.
        rows: u16,
    },
    /// The session and everything running under it are gone.
    Closed,
    /// The operation did not happen.
    Refused(PtyRefusal),
}

impl fmt::Debug for PtyAnswer {
    /// Reports shapes and counts, and never a byte the terminal produced.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Opened { columns, rows } => write!(formatter, "Opened {columns}x{rows}"),
            Self::Wrote { bytes } => write!(formatter, "Wrote {{ {bytes} }}"),
            Self::Output { bytes, end } => {
                write!(formatter, "Output {{ {} bytes, end: {end} }}", bytes.len())
            }
            Self::Resized { columns, rows } => write!(formatter, "Resized {columns}x{rows}"),
            Self::Closed => formatter.write_str("Closed"),
            Self::Refused(refusal) => write!(formatter, "Refused({refusal})"),
        }
    }
}

/// One terminal answer, bound to the operation and Instance it belongs to.
///
/// The identities travel with the answer so the engine can refuse an answer that names a
/// different operation or a different Instance than the one it asked about, rather than reporting
/// one sandbox's terminal as another's.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PtyObservation {
    operation_id: OperationId,
    instance_id: InstanceId,
    answer: PtyAnswer,
}

impl PtyObservation {
    #[must_use]
    pub const fn new(
        operation_id: OperationId,
        instance_id: InstanceId,
        answer: PtyAnswer,
    ) -> Self {
        Self {
            operation_id,
            instance_id,
            answer,
        }
    }

    #[must_use]
    pub const fn operation_id(&self) -> &OperationId {
        &self.operation_id
    }

    #[must_use]
    pub const fn instance_id(&self) -> &InstanceId {
        &self.instance_id
    }

    #[must_use]
    pub const fn answer(&self) -> &PtyAnswer {
        &self.answer
    }

    pub(crate) fn into_answer(self) -> PtyAnswer {
        self.answer
    }
}
