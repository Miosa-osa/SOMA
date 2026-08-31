use crate::Error;

use super::{FileOutcome, OperationId, OutputChunk, PtyOutcome, TerminalReport, frame};

/// A guest-to-host application message carried by one authenticated record.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GuestMessage {
    /// Reports completion of the fixed authenticated Repair contract.
    RepairComplete {
        /// Identity of the Launch operation being repaired.
        operation: OperationId,
    },
    /// Carries one bounded stdout chunk.
    Stdout {
        /// Identity of the current command operation.
        operation: OperationId,
        /// Exact binary output bytes.
        chunk: OutputChunk,
    },
    /// Carries one bounded stderr chunk.
    Stderr {
        /// Identity of the current command operation.
        operation: OperationId,
        /// Exact binary output bytes.
        chunk: OutputChunk,
    },
    /// Terminates one command output stream.
    Terminal {
        /// Identity of the completed command operation.
        operation: OperationId,
        /// Exact process or agent outcome.
        report: TerminalReport,
    },
    /// Answers one bounded filesystem request.
    FileOutcome {
        /// Identity of the filesystem operation being answered.
        operation: OperationId,
        /// What the guest did, or why it did not.
        outcome: FileOutcome,
    },
    /// Answers one interactive terminal request.
    PtyOutcome {
        /// Identity of the terminal operation being answered.
        operation: OperationId,
        /// What the guest's terminal did, or why it did not.
        outcome: PtyOutcome,
    },
    /// Acknowledges a graceful Shutdown request.
    ShutdownAck {
        /// Identity of the Stop operation.
        operation: OperationId,
    },
}

impl GuestMessage {
    /// Creates a Repair-complete message.
    #[must_use]
    pub const fn repair_complete(operation: OperationId) -> Self {
        Self::RepairComplete { operation }
    }

    /// Creates a stdout message.
    #[must_use]
    pub const fn stdout(operation: OperationId, chunk: OutputChunk) -> Self {
        Self::Stdout { operation, chunk }
    }

    /// Creates a stderr message.
    #[must_use]
    pub const fn stderr(operation: OperationId, chunk: OutputChunk) -> Self {
        Self::Stderr { operation, chunk }
    }

    /// Creates a terminal message.
    #[must_use]
    pub const fn terminal(operation: OperationId, report: TerminalReport) -> Self {
        Self::Terminal { operation, report }
    }

    /// Creates one filesystem answer.
    #[must_use]
    pub const fn file_outcome(operation: OperationId, outcome: FileOutcome) -> Self {
        Self::FileOutcome { operation, outcome }
    }

    /// Creates one interactive terminal answer.
    #[must_use]
    pub const fn pty_outcome(operation: OperationId, outcome: PtyOutcome) -> Self {
        Self::PtyOutcome { operation, outcome }
    }

    /// Creates a graceful Shutdown acknowledgement.
    #[must_use]
    pub const fn shutdown_ack(operation: OperationId) -> Self {
        Self::ShutdownAck { operation }
    }

    /// Encodes this message as exactly one authenticated-record payload.
    ///
    /// # Errors
    ///
    /// Returns an error for a locally invalid terminal status.
    pub fn encode(&self) -> Result<Vec<u8>, Error> {
        match self {
            Self::RepairComplete { operation } => {
                frame::encode(frame::Kind::RepairComplete, *operation, &[])
            }
            Self::Stdout { operation, chunk } => {
                frame::encode(frame::Kind::Stdout, *operation, chunk.as_bytes())
            }
            Self::Stderr { operation, chunk } => {
                frame::encode(frame::Kind::Stderr, *operation, chunk.as_bytes())
            }
            Self::Terminal { operation, report } => {
                frame::encode(frame::Kind::Terminal, *operation, &report.encode()?)
            }
            Self::FileOutcome { operation, outcome } => {
                frame::encode(frame::Kind::FileOutcome, *operation, &outcome.encode_body())
            }
            Self::PtyOutcome { operation, outcome } => {
                frame::encode(frame::Kind::PtyOutcome, *operation, &outcome.encode_body())
            }
            Self::ShutdownAck { operation } => {
                frame::encode(frame::Kind::ShutdownAck, *operation, &[])
            }
        }
    }

    /// Decodes one exact guest message from one authenticated-record payload.
    ///
    /// # Errors
    ///
    /// Returns [`Error::ApplicationMessageRejected`] for every malformed message.
    pub fn decode(encoded: &[u8]) -> Result<Self, Error> {
        let decoded = frame::decode(encoded)?;
        match decoded.kind {
            frame::Kind::RepairComplete if decoded.body.is_empty() => {
                Ok(Self::repair_complete(decoded.operation))
            }
            frame::Kind::Stdout => Ok(Self::stdout(
                decoded.operation,
                OutputChunk::decode(decoded.body)?,
            )),
            frame::Kind::Stderr => Ok(Self::stderr(
                decoded.operation,
                OutputChunk::decode(decoded.body)?,
            )),
            frame::Kind::Terminal => Ok(Self::terminal(
                decoded.operation,
                TerminalReport::decode(decoded.body)?,
            )),
            frame::Kind::FileOutcome => Ok(Self::file_outcome(
                decoded.operation,
                FileOutcome::decode_body(decoded.body)?,
            )),
            frame::Kind::PtyOutcome => Ok(Self::pty_outcome(
                decoded.operation,
                PtyOutcome::decode_body(decoded.body)?,
            )),
            frame::Kind::ShutdownAck if decoded.body.is_empty() => {
                Ok(Self::shutdown_ack(decoded.operation))
            }
            frame::Kind::File
            | frame::Kind::Pty
            | frame::Kind::PrepareAndProbe
            | frame::Kind::Execute
            | frame::Kind::Shutdown
            | frame::Kind::RepairComplete
            | frame::Kind::ShutdownAck => Err(Error::ApplicationMessageRejected),
        }
    }
}
